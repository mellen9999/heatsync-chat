// inline pictures in chat (ext feed-embed.js extractChatEmbed): the FIRST link of a line, when it is a
// direct image, shows under the text. other hosts go through heatsync's `/api/img` proxy so the image
// host never sees our ip (the ext's hsProxyImg); the few hosts the ext fetches direct stay direct.
// the link is hostile text: bounded, scheme + host checked, nothing else is ever fetched
use std::ops::Range;

use crate::images::url::host;

// ext HS_IMG_DIRECT_HOSTS, https only
const DIRECT: [&str; 8] = [
  "static-cdn.jtvnw.net",
  "cdn.7tv.app",
  "cdn.betterttv.net",
  "cdn.frankerfacez.com",
  "files.kick.com",
  "heatsync.org",
  "cdn.heatsync.org",
  "d3aqoihi2n8ty8.cloudfront.net",
];
const EXTS: [&str; 6] = [".jpg", ".jpeg", ".png", ".gif", ".webp", ".avif"];
// short enough that the proxied form stays under the 2048 url cap even fully escaped
const MAX_LINK: usize = 600;
const PROXY: &str = "https://heatsync.org/api/img?url=";

#[derive(Debug, PartialEq, Eq)]
pub struct Embed {
  // what to fetch
  pub display: Box<str>,
  // the word that is exactly this link, so the text can fold it away once the picture is up
  pub word: Option<Range<usize>>,
}

// encodeURIComponent
fn escape(s: &str) -> String {
  let mut out = String::with_capacity(s.len() * 2);
  for b in s.bytes() {
    if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

// trailing sentence punctuation goes; a closing ) or ] only when unbalanced
fn trim(mut u: &str) -> &str {
  loop {
    u = u.trim_end_matches(['.', ',', ';', '!', '?']);
    let (open, close) = match u.as_bytes().last() {
      Some(b')') => (b'(', b')'),
      Some(b']') => (b'[', b']'),
      _ => return u,
    };
    let n = |c: u8| u.bytes().filter(|&b| b == c).count();
    if n(close) <= n(open) {
      return u;
    }
    u = &u[..u.len() - 1];
  }
}

fn is_image(u: &str) -> bool {
  let path = u.split_once('?').map_or(u, |(p, _)| p).to_ascii_lowercase();
  EXTS.iter().any(|e| path.ends_with(e))
}

fn authority_ok(u: &str) -> bool {
  let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://")).unwrap_or("");
  let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let a = &rest[..end];
  let tld = a.rsplit('.').next().unwrap_or("");
  // a name with a dot; never a bare ip (no numeric last label), never a port or userinfo
  !a.is_empty()
    && a.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    && a.contains('.')
    && !tld.is_empty()
    && !tld.bytes().all(|b| b.is_ascii_digit())
}

pub fn find(body: &str) -> Option<Embed> {
  let lower = body.to_ascii_lowercase();
  let at = lower.find("http://").into_iter().chain(lower.find("https://")).min()?;
  let tail = &body[at..];
  let end = tail.find(|c: char| c.is_whitespace() || c.is_control() || "<>\"'".contains(c)).unwrap_or(tail.len());
  let url = trim(&tail[..end]);
  if url.len() > MAX_LINK || !authority_ok(url) || !is_image(url) {
    return None;
  }
  let display: Box<str> = match host(url).map(str::to_ascii_lowercase) {
    Some(h) if DIRECT.contains(&h.as_str()) => {
      // 7tv serves avif by name; we decode webp
      match url.strip_suffix(".avif").filter(|_| h == "cdn.7tv.app") {
        Some(stem) => format!("{stem}.webp").into(),
        None => url.into(),
      }
    }
    _ => format!("{PROXY}{}", escape(url)).into(),
  };
  // the line is split on single spaces, same as the renderer; fold only a word that is the whole link
  let start = body[..at].rfind(' ').map_or(0, |i| i + 1);
  let whole = start == at && url.len() == end && (body[at + end..].is_empty() || body[at + end..].starts_with(' '));
  Some(Embed { display, word: whole.then_some(at..at + end) })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn direct_images_only() {
    let e = find("look https://i.imgur.com/ab.PNG?x=1&y=2 lol").unwrap();
    assert_eq!(&*e.display, "https://heatsync.org/api/img?url=https%3A%2F%2Fi.imgur.com%2Fab.PNG%3Fx%3D1%26y%3D2");
    assert_eq!(e.word, Some(5..39));
    assert!(find("https://example.com/page").is_none());
    assert!(find("https://example.com/a.png.exe").is_none());
    assert!(find("no link here").is_none());
    // the first link decides, like the ext
    assert!(find("https://example.com/page https://i.imgur.com/a.png").is_none());
  }

  #[test]
  fn direct_hosts_stay_direct_and_avif_becomes_webp() {
    let e = find("https://cdn.7tv.app/emote/abc/4x.avif").unwrap();
    assert_eq!(&*e.display, "https://cdn.7tv.app/emote/abc/4x.webp");
    assert_eq!(&*find("https://files.kick.com/images/a.webp").unwrap().display, "https://files.kick.com/images/a.webp");
    // a direct host over plain http goes through the proxy
    assert!(find("http://files.kick.com/a.webp").unwrap().display.starts_with(PROXY));
  }

  #[test]
  fn hostile_links_are_refused() {
    assert!(find("https://user@evil.com/a.png").is_none());
    assert!(find("https://evil.com:8080/a.png").is_none());
    assert!(find("https://localhost/a.png").is_none());
    assert!(find(&format!("https://a.com/{}.png", "x".repeat(MAX_LINK))).is_none());
    assert!(find("https:///a.png").is_none());
    assert!(find("https://127.0.0.1/a.png").is_none());
    // a worst case escape still fits the url cap
    let long = format!("https://a.com/{}.png", "é".repeat(250));
    assert!(find(&long).is_none_or(|e| e.display.len() <= 2048));
  }

  #[test]
  fn trailing_punctuation_and_folding() {
    let e = find("https://a.com/b.gif!").unwrap();
    assert!(e.display.ends_with("b.gif"));
    // trailing junk stuck to the word: keep the text, fold nothing
    assert_eq!(e.word, None);
    let e = find("(https://a.com/b.gif)").unwrap();
    assert!(e.display.ends_with("b.gif"));
    assert_eq!(e.word, None);
    assert_eq!(find("https://a.com/b.gif").unwrap().word, Some(0..19));
  }
}
