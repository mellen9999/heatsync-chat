// inline pictures in chat (ext feed-embed.js extractChatEmbed): the FIRST link of a line, when it is a
// direct image, shows under the text; a youtube video shows its still with a ▶ (a click plays it). other hosts go through heatsync's `/api/img` proxy so the image
// host never sees our ip (the ext's hsProxyImg); the few hosts the ext fetches direct stay direct.
// the link is hostile text: bounded, scheme + host checked, nothing else is ever fetched
use std::ops::Range;

use crate::images::url::host;
use crate::model::preview;

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
  // a video's link: the picture is its still, and a click plays this
  pub play: Option<Box<str>>,
}

// encodeURIComponent
pub fn escape(s: &str) -> String {
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

// what a line embeds under it
#[derive(Debug, PartialEq, Eq)]
pub enum Found {
  Pic(Embed),
  // a link heatsync's resolver previews (model::preview)
  Preview(Box<str>),
}

// the first http(s) link when it embeds; else (partial links on, ext extractChatEmbed) the first youtube
// video, tweet or reddit post named without its scheme or host: `watch?v=…`, `name/status/…`, `r/x/comments/…`
pub fn find(body: &str, partial: bool) -> Option<Found> {
  full(body).or_else(|| {
    let links = if partial { crate::model::links::find(body, true) } else { Vec::new() };
    links.into_iter().find_map(|l| match crate::model::video::yt_id(&l.url) {
      Some(id) => Some(Found::Pic(yt(id, &l.url))),
      // a bare `r/sub` is only a link: a post (`…/comments/…`) previews
      None => (preview::site(&l.url).is_some() && (!l.url.contains("reddit.com/") || l.url.contains("/comments/")))
        .then(|| Found::Preview(l.url.into())),
    })
  })
}

// a youtube video's still (through the proxy, ext feed-embed.js); the link stays readable
fn yt(id: &str, url: &str) -> Embed {
  Embed { display: proxied(&format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")), word: None, play: Some(url.into()) }
}

// a picture from anywhere, as we fetch it: the ext's few direct hosts as they are, the rest through
// heatsync's proxy
pub fn proxied(url: &str) -> Box<str> {
  match host(url).map(str::to_ascii_lowercase) {
    Some(h) if DIRECT.contains(&h.as_str()) => {
      // 7tv serves avif by name; we decode webp
      match url.strip_suffix(".avif").filter(|_| h == "cdn.7tv.app") {
        Some(stem) => format!("{stem}.webp").into(),
        None => url.into(),
      }
    }
    _ => format!("{PROXY}{}", escape(url)).into(),
  }
}

fn full(body: &str) -> Option<Found> {
  let lower = body.to_ascii_lowercase();
  let at = lower.find("http://").into_iter().chain(lower.find("https://")).min()?;
  let tail = &body[at..];
  let end = tail.find(|c: char| c.is_whitespace() || c.is_control() || "<>\"'".contains(c)).unwrap_or(tail.len());
  let url = trim(&tail[..end]);
  if url.len() > MAX_LINK || !authority_ok(url) {
    return None;
  }
  if let Some(id) = crate::model::video::yt_id(url) {
    return Some(Found::Pic(yt(id, url)));
  }
  if preview::site(url).is_some() {
    return Some(Found::Preview(url.into()));
  }
  if !is_image(url) {
    return None;
  }
  // the line is split on single spaces, same as the renderer; fold only a word that is the whole link
  let start = body[..at].rfind(' ').map_or(0, |i| i + 1);
  let whole = start == at && url.len() == end && (body[at + end..].is_empty() || body[at + end..].starts_with(' '));
  Some(Found::Pic(Embed { display: proxied(url), word: whole.then_some(at..at + end), play: None }))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn pic(body: &str) -> Option<Embed> {
    match full(body) {
      Some(Found::Pic(e)) => Some(e),
      _ => None,
    }
  }

  #[test]
  fn direct_images_only() {
    let e = pic("look https://i.imgur.com/ab.PNG?x=1&y=2 lol").unwrap();
    assert_eq!(&*e.display, "https://heatsync.org/api/img?url=https%3A%2F%2Fi.imgur.com%2Fab.PNG%3Fx%3D1%26y%3D2");
    assert_eq!(e.word, Some(5..39));
    assert!(pic("https://example.com/page").is_none());
    assert!(pic("https://example.com/a.png.exe").is_none());
    assert!(pic("no link here").is_none());
    // the first link decides, like the ext
    assert!(pic("https://example.com/page https://i.imgur.com/a.png").is_none());
  }

  #[test]
  fn a_youtube_link_shows_its_still_and_plays() {
    let e = pic("watch https://youtu.be/dQw4w9WgXcQ?t=5 lol").unwrap();
    assert_eq!(
      &*e.display,
      "https://heatsync.org/api/img?url=https%3A%2F%2Fi.ytimg.com%2Fvi%2FdQw4w9WgXcQ%2Fmqdefault.jpg"
    );
    assert_eq!((e.word, e.play.as_deref()), (None, Some("https://youtu.be/dQw4w9WgXcQ?t=5")));
    assert!(pic("https://www.youtube.com/@someone").is_none());
  }

  #[test]
  fn a_youtube_link_without_its_host_embeds_when_partial_links_are_on() {
    for t in ["watch?v=dQw4w9WgXcQ lol", "lol youtube.com/watch?v=dQw4w9WgXcQ", "see shorts/dQw4w9WgXcQ"] {
      let Some(Found::Pic(e)) = find(t, true) else { panic!("{t}") };
      assert!(e.display.contains("dQw4w9WgXcQ"), "{t}");
      assert!(e.play.as_deref().is_some_and(|p| p.starts_with("https://") && p.contains("youtube.com/")), "{t}");
      assert!(find(t, false).is_none(), "{t}: partial links off");
    }
    // a real link that embeds wins over a fragment
    assert!(matches!(
      find("https://a.com/b.gif watch?v=dQw4w9WgXcQ", true),
      Some(Found::Pic(Embed { play: None, .. }))
    ));
    assert!(find("https://example.com/page", true).is_none());
  }

  #[test]
  fn direct_hosts_stay_direct_and_avif_becomes_webp() {
    let e = pic("https://cdn.7tv.app/emote/abc/4x.avif").unwrap();
    assert_eq!(&*e.display, "https://cdn.7tv.app/emote/abc/4x.webp");
    assert_eq!(&*pic("https://files.kick.com/images/a.webp").unwrap().display, "https://files.kick.com/images/a.webp");
    // a direct host over plain http goes through the proxy
    assert!(pic("http://files.kick.com/a.webp").unwrap().display.starts_with(PROXY));
  }

  #[test]
  fn hostile_links_are_refused() {
    assert!(pic("https://user@evil.com/a.png").is_none());
    assert!(pic("https://evil.com:8080/a.png").is_none());
    assert!(pic("https://localhost/a.png").is_none());
    assert!(pic(&format!("https://a.com/{}.png", "x".repeat(MAX_LINK))).is_none());
    assert!(pic("https:///a.png").is_none());
    assert!(pic("https://127.0.0.1/a.png").is_none());
    // a worst case escape still fits the url cap
    let long = format!("https://a.com/{}.png", "é".repeat(250));
    assert!(pic(&long).is_none_or(|e| e.display.len() <= 2048));
  }

  #[test]
  fn trailing_punctuation_and_folding() {
    let e = pic("https://a.com/b.gif!").unwrap();
    assert!(e.display.ends_with("b.gif"));
    // trailing junk stuck to the word: keep the text, fold nothing
    assert_eq!(e.word, None);
    let e = pic("(https://a.com/b.gif)").unwrap();
    assert!(e.display.ends_with("b.gif"));
    assert_eq!(e.word, None);
    assert_eq!(pic("https://a.com/b.gif").unwrap().word, Some(0..19));
  }

  #[test]
  fn provider_links_ask_for_a_preview() {
    let r = "https://www.reddit.com/r/rust/comments/1c0rgn3/x/";
    assert_eq!(find(&format!("look {r}"), false), Some(Found::Preview(r.into())));
    assert_eq!(
      find("lol someone/status/1234567890", true),
      Some(Found::Preview("https://x.com/someone/status/1234567890".into()))
    );
    assert_eq!(find("lol someone/status/1234567890", false), None);
    assert_eq!(&*proxied("https://i.scdn.co/a"), "https://heatsync.org/api/img?url=https%3A%2F%2Fi.scdn.co%2Fa");
  }
}
