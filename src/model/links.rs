// links in chat text. full links are the ext's LINK_RE (emotes.js): a word that starts `http(s)://` or is
// `host.tld/path`. partial links are a port of linkifyPartialLinks (utils.js): host-less youtube / reddit /
// tweet fragments, defanged `site (dot) com`, and bare hosts on a curated tld list. the shown text stays as
// typed; the target is the canonical url. hand-written matchers, same semantics as the ext's regexes
// (lookbehinds, greedy tails, and what they refuse). every match is plain ascii at its edges
use std::ops::Range;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Link {
  // bytes of the body as typed
  pub range: Range<usize>,
  pub url: String,
}

const MAX_URL: usize = 2048;
const MAX_LINKS: usize = 32;
// the ext's COMMON_TLDS. the two-letter english-word ones (im me is it at in to us be no so do by) are out on purpose
const TLDS: [&str; 32] = [
  "com", "net", "org", "io", "gg", "tv", "dev", "app", "xyz", "info", "online", "shop", "club", "wiki", "news", "blog",
  "art", "fm", "ai", "co", "uk", "de", "fr", "jp", "nl", "se", "pl", "ru", "br", "ca", "au", "eu",
];

type Hit = Option<(usize, Option<String>)>;

pub fn find(body: &str, partial: bool) -> Vec<Link> {
  let mut out: Vec<Link> = Vec::new();
  // almost every line has none of these: skip without scanning
  let maybe = body.contains(['/', '.', '?']) || body.to_ascii_lowercase().contains("dot");
  if !maybe {
    return out;
  }
  let b = body.as_bytes();
  let mut at = 0;
  while at < b.len() {
    if b[at].is_ascii_whitespace() {
      at += 1;
      continue;
    }
    let end = b[at..].iter().position(|c| c.is_ascii_whitespace()).map_or(b.len(), |n| at + n);
    if let Some(url) = full(&body[at..end]) {
      add(&mut out, at..end, url);
    }
    at = end;
  }
  if partial {
    pass(body, &mut out, yt, false);
    pass(body, &mut out, yt_list, false);
    pass(body, &mut out, reddit, false);
    pass(body, &mut out, tweet, false);
    pass(body, &mut out, defang, true);
    pass(body, &mut out, bare, false);
  }
  out.sort_by_key(|l| l.range.start);
  out
}

fn add(out: &mut Vec<Link>, range: Range<usize>, url: String) {
  if out.len() < MAX_LINKS
    && url.len() <= MAX_URL
    && !out.iter().any(|l| l.range.start < range.end && range.start < l.range.end)
  {
    out.push(Link { range, url });
  }
}

// one regex of the ext, run over the whole body: matches never overlap a link already taken
// a defanged run may swallow a full-link word it starts with: `https://heatsync (dot) org`
fn pass(s: &str, out: &mut Vec<Link>, m: fn(&str, usize) -> Hit, swallow: bool) {
  let mut i = 0;
  while i < s.len() {
    match m(s, i) {
      Some((end, url)) if end > i => {
        if let Some(url) = url {
          if swallow {
            out.retain(|l| !(l.range.start < end && i < l.range.end));
          }
          add(out, i..end, url);
        }
        i = end;
      }
      _ => i += 1,
    }
  }
}

// the ext's LINK_RE on one word: /^(https?:\/\/\S+|[a-z0-9-]+(\.[a-z0-9-]+)+\/\S*)/i
fn full(w: &str) -> Option<String> {
  let lower = w.get(..8).unwrap_or(w).to_ascii_lowercase();
  let scheme = if lower.starts_with("https://") {
    8
  } else if lower.starts_with("http://") {
    7
  } else {
    0
  };
  if scheme > 0 {
    return (w.len() > scheme).then(|| w.to_string());
  }
  let b = w.as_bytes();
  let (mut p, mut dots) = (0, 0);
  loop {
    let n = b[p..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'-').count();
    if n == 0 {
      return None;
    }
    p += n;
    if b.get(p) == Some(&b'.') {
      dots += 1;
      p += 1;
    } else {
      break;
    }
  }
  (dots > 0 && b.get(p) == Some(&b'/')).then(|| format!("https://{w}"))
}

fn word(c: u8) -> bool {
  c.is_ascii_alphanumeric() || c == b'_'
}

fn idc(c: u8) -> bool {
  word(c) || c == b'-'
}

// js (?<![\w<blocked>]) at i
fn before(b: &[u8], i: usize, blocked: &[u8]) -> bool {
  i == 0 || (!word(b[i - 1]) && !blocked.contains(&b[i - 1]))
}

fn run(b: &[u8], p: usize, ok: impl Fn(u8) -> bool) -> usize {
  b.get(p..).map_or(0, |t| t.iter().take_while(|c| ok(**c)).count())
}

fn s(b: &[u8], a: usize, e: usize) -> &str {
  std::str::from_utf8(&b[a..e]).unwrap_or("")
}

// PARTIAL_YT_RE: /?(watch?v=|shorts/|live/|embed/|v/) + 11-char id + params, not inside a word or url
fn yt(t: &str, i: usize) -> Hit {
  let b = t.as_bytes();
  if !before(b, i, b"/.=-") {
    return None;
  }
  let p = i + usize::from(b[i] == b'/');
  let kind = ["watch?v=", "shorts/", "live/", "embed/", "v/"].into_iter().find(|k| b[p..].starts_with(k.as_bytes()))?;
  let id = p + kind.len();
  if run(b, id, idc) < 11 {
    return None;
  }
  let mut e = id + 11;
  let mut qs = String::new();
  let qc = |c: u8| c.is_ascii_alphanumeric() || b"_=%.+-".contains(&c);
  loop {
    let sep = if b[e..].starts_with(b"&amp;") && run(b, e + 5, qc) > 0 {
      5
    } else if matches!(b.get(e), Some(b'?' | b'&')) && run(b, e + 1, qc) > 0 {
      1
    } else {
      break;
    };
    let n = run(b, e + sep, qc);
    qs.push(if b[e] == b'?' { '?' } else { '&' });
    qs.push_str(s(b, e + sep, e + sep + n));
    e += sep + n;
  }
  if b.get(e).is_some_and(|c| idc(*c)) {
    return None;
  }
  let id = s(b, id, id + 11);
  let path = match kind {
    "watch?v=" | "v/" => format!("watch?v={id}{qs}"),
    k => format!("{k}{id}{qs}"),
  };
  Some((e, Some(format!("https://www.youtube.com/{path}"))))
}

// PARTIAL_YT_LIST_RE: playlist?list=<12+> / channel/UC<12+>. no @handles (those are mentions)
fn yt_list(t: &str, i: usize) -> Hit {
  let b = t.as_bytes();
  if !before(b, i, b"/.=-") {
    return None;
  }
  let p = i + usize::from(b[i] == b'/');
  let kind = ["playlist?list=", "channel/UC"].into_iter().find(|k| b[p..].starts_with(k.as_bytes()))?;
  let id = p + kind.len();
  let n = run(b, id, idc);
  if n < 12 {
    return None;
  }
  let id = s(b, id, id + n);
  let path = if kind.starts_with('p') { format!("playlist?list={id}") } else { format!("channel/UC{id}") };
  Some((p + kind.len() + n, Some(format!("https://www.youtube.com/{path}"))))
}

// PARTIAL_REDDIT_RE: r/ u/ user/ + a 3-21 char name, optionally a post permalink
fn reddit(t: &str, i: usize) -> Hit {
  let b = t.as_bytes();
  if !before(b, i, b"/.-") {
    return None;
  }
  let p = i + usize::from(b[i] == b'/');
  let (kind, off) = [("r", 2), ("user", 5), ("u", 2)]
    .into_iter()
    .find(|(k, n)| b[p..].starts_with(k.as_bytes()) && b.get(p + n - 1) == Some(&b'/'))?;
  let n0 = p + off;
  if !b.get(n0).is_some_and(|c| c.is_ascii_alphanumeric()) {
    return None;
  }
  let len = run(b, n0, word).min(21);
  for l in (3..=len).rev() {
    let ne = n0 + l;
    for e in post_ends(b, ne) {
      for slash in [1, 0] {
        if slash == 1 && b.get(e) != Some(&b'/') {
          continue;
        }
        let after = b.get(e + slash).copied();
        if !after.is_some_and(|c| word(c) || c == b'/' || c == b'-') {
          let name = s(b, n0, ne);
          let who = if kind == "r" { "r" } else { "user" };
          return Some((e + slash, Some(format!("https://www.reddit.com/{who}/{name}{}", s(b, ne, e)))));
        }
      }
    }
  }
  None
}

// where a `/comments/<id>(/<slug>){0,2}` group can end after the name, best first; the bare name is last
fn post_ends(b: &[u8], p: usize) -> Vec<usize> {
  let mut v = Vec::new();
  if b[p..].starts_with(b"/comments/") {
    let id = p + 10;
    let lc = |c: u8| c.is_ascii_digit() || c.is_ascii_lowercase();
    for l in (2..=run(b, id, lc).min(12)).rev() {
      let mut ends = vec![id + l];
      let mut q = id + l;
      for _ in 0..2 {
        let n = if b.get(q) == Some(&b'/') { run(b, q + 1, idc) } else { 0 };
        if n == 0 {
          break;
        }
        q += 1 + n;
        ends.push(q);
      }
      v.extend(ends.into_iter().rev());
    }
  }
  v.push(p);
  v
}

// PARTIAL_TWEET_RE: [@]name/status/<8-25 digits>[?query], not inside a word or url
fn tweet(t: &str, i: usize) -> Hit {
  let b = t.as_bytes();
  if !before(b, i, b"/.@-") {
    return None;
  }
  let p = i + usize::from(b[i] == b'@');
  let n = run(b, p, word);
  if !(1..=15).contains(&n) || !b[p + n..].starts_with(b"/status/") {
    return None;
  }
  let d = p + n + 8;
  let digits = run(b, d, |c| c.is_ascii_digit()).min(25);
  if digits < 8 {
    return None;
  }
  let e = d + digits;
  let qc = |c: u8| word(c) || b"=&;%-".contains(&c);
  let mut ends = Vec::new();
  if b.get(e) == Some(&b'?') {
    let q = run(b, e + 1, qc);
    ends.extend((0..=q).rev().map(|k| e + 1 + k));
  }
  ends.push(e);
  let end = ends.into_iter().find(|&x| !b.get(x).is_some_and(|c| word(*c) || *c == b'/'))?;
  Some((end, Some(format!("https://x.com/{}/status/{}", s(b, p, p + n), s(b, d, e)))))
}

// BARE_HOST_RE: `heatsync.org` with no path, tld on the curated list. emails, run-on prose and files stay text
fn bare(t: &str, i: usize) -> Hit {
  let b = t.as_bytes();
  if !before(b, i, b".@/-") || !b[i].is_ascii_alphanumeric() {
    return None;
  }
  let mut dots = Vec::new();
  let mut p = i;
  loop {
    let n = run(b, p, |c| c.is_ascii_alphanumeric() || c == b'-');
    if n == 0 || !b[p].is_ascii_alphanumeric() || b.get(p + n) != Some(&b'.') {
      break;
    }
    p += n + 1;
    dots.push(p);
  }
  for &d in dots.iter().rev() {
    let n = run(b, d, word);
    let tld = s(b, d, d + n).to_ascii_lowercase();
    if TLDS.contains(&tld.as_str()) && !matches!(b.get(d + n), Some(b'.' | b'-' | b'/')) {
      let host = s(b, i, d + n).to_ascii_lowercase();
      return Some((d + n, Some(format!("https://{host}"))));
    }
  }
  None
}

// the gap between defanged labels: ( dot ) [dot] {.} with optional spaces, or the bare word " dot "
fn sep(b: &[u8], p: usize) -> Option<usize> {
  let sp = |q: usize| q + run(b, q, |c| c.is_ascii_whitespace());
  let dot = |q: usize| {
    if b.get(q..q + 3).is_some_and(|w| w.eq_ignore_ascii_case(b"dot")) {
      Some(q + 3)
    } else {
      (b.get(q) == Some(&b'.')).then_some(q + 1)
    }
  };
  let q = sp(p);
  if matches!(b.get(q), Some(b'(' | b'[' | b'{'))
    && let Some(r) = dot(sp(q + 1))
    && let r = sp(r)
    && matches!(b.get(r), Some(b')' | b']' | b'}'))
  {
    return Some(sp(r + 1));
  }
  let q = sp(p);
  (q > p
    && b.get(q..q + 3).is_some_and(|w| w.eq_ignore_ascii_case(b"dot"))
    && b.get(q + 3).is_some_and(|c| c.is_ascii_whitespace()))
  .then(|| sp(q + 3))
}

// DEFANG_RE + defangedToHost: `https://heatsync (dot) org/x`, `hxxps://evil[.]example`. the tail label must
// look like a tld, else the run is eaten but not linked ("3 dot 5")
fn defang(t: &str, i: usize) -> Hit {
  let b = t.as_bytes();
  if !before(b, i, b".-") {
    return None;
  }
  let mut p = i;
  let mut proto = "https://".to_string();
  if b[p..].len() >= 8 && b[p].eq_ignore_ascii_case(&b'h') {
    let head = s(b, p, p + 4).to_ascii_lowercase();
    if head == "http" || head == "hxxp" {
      let mut q = p + 4;
      let secure = b.get(q).is_some_and(|c| c.eq_ignore_ascii_case(&b's'));
      q += usize::from(secure);
      if b[q..].starts_with(b"://") {
        proto = format!("http{}://", if secure { "s" } else { "" });
        p = q + 3;
      }
    }
  }
  let label = |q: usize| {
    let ok = b.get(q).is_some_and(|c| c.is_ascii_alphanumeric());
    ok.then(|| q + run(b, q, |c| c.is_ascii_alphanumeric() || c == b'-'))
  };
  let mut e = label(p)?;
  let mut labels = vec![s(b, p, e)];
  while let Some(next) = sep(b, e).and_then(|n| label(n).map(|le| (n, le))) {
    labels.push(s(b, next.0, next.1));
    e = next.1;
  }
  if labels.len() < 2 {
    return None;
  }
  let path = if b.get(e) == Some(&b'/') {
    t[e..].find(|c: char| c.is_whitespace() || "<>\"'()".contains(c)).unwrap_or(t.len() - e)
  } else {
    0
  };
  let end = e + path;
  let tail = labels[labels.len() - 1];
  if !(2..=24).contains(&tail.len()) || !tail.bytes().all(|c| c.is_ascii_alphabetic()) {
    return Some((end, None));
  }
  let host = labels.join(".").to_ascii_lowercase();
  Some((end, Some(format!("{proto}{host}{}", &t[e..end]))))
}

#[cfg(test)]
mod tests {
  use super::*;

  // (shown text, target) of every link in the line
  fn got(body: &str) -> Vec<(&str, String)> {
    find(body, true).into_iter().map(|l| (&body[l.range], l.url)).collect()
  }

  fn one(body: &str) -> (String, String) {
    let g = got(body);
    assert_eq!(g.len(), 1, "{body}: {g:?}");
    (g[0].0.to_string(), g[0].1.clone())
  }

  fn none(body: &str) {
    assert!(got(body).is_empty(), "{body}: {:?}", got(body));
  }

  fn is(body: &str, shown: &str, url: &str) {
    assert_eq!(one(body), (shown.to_string(), url.to_string()), "{body}");
  }

  #[test]
  fn full_links() {
    is("see https://heatsync.org/x now", "https://heatsync.org/x", "https://heatsync.org/x");
    is("HTTP://a.b", "HTTP://a.b", "HTTP://a.b");
    is("heatsync.org/emotes", "heatsync.org/emotes", "https://heatsync.org/emotes");
    none("https://");
    none("nothing.here");
    // a full link keeps its parts: nothing inside it links again
    assert_eq!(got("youtube.com/watch?v=l9i0hDNBdZM").len(), 1);
    assert_eq!(got("youtube.com/shorts/e2B1_DNpsV4").len(), 1);
    assert_eq!(got("reddit.com/r/place").len(), 1);
    none("javascript:alert(1)");
  }

  #[test]
  fn partial_off_leaves_only_full_links() {
    assert!(find("watch?v=Gz0fAz9n_Os heatsync.org", false).is_empty());
    assert_eq!(find("heatsync.org/x", false).len(), 1);
  }

  #[test]
  fn youtube_watch() {
    is("watch?v=Gz0fAz9n_Os", "watch?v=Gz0fAz9n_Os", "https://www.youtube.com/watch?v=Gz0fAz9n_Os");
    is("check watch?v=l9i0hDNBdZM out", "watch?v=l9i0hDNBdZM", "https://www.youtube.com/watch?v=l9i0hDNBdZM");
    none("watch?v=short");
    is(
      "watch?v=l9i0hDNBdZM&amp;t=30s",
      "watch?v=l9i0hDNBdZM&amp;t=30s",
      "https://www.youtube.com/watch?v=l9i0hDNBdZM&t=30s",
    );
    is("watch?v=l9i0hDNBdZM&t=30s", "watch?v=l9i0hDNBdZM&t=30s", "https://www.youtube.com/watch?v=l9i0hDNBdZM&t=30s");
    none("watch?v=l9i0hDNBdZMX");
  }

  #[test]
  fn youtube_fragments() {
    is("lol shorts/e2B1_DNpsV4", "shorts/e2B1_DNpsV4", "https://www.youtube.com/shorts/e2B1_DNpsV4");
    assert_eq!(one("live/e2B1_DNpsV4").1, "https://www.youtube.com/live/e2B1_DNpsV4");
    assert_eq!(one("embed/e2B1_DNpsV4").1, "https://www.youtube.com/embed/e2B1_DNpsV4");
    assert_eq!(one("v/e2B1_DNpsV4").1, "https://www.youtube.com/watch?v=e2B1_DNpsV4");
    is("/shorts/e2B1_DNpsV4", "/shorts/e2B1_DNpsV4", "https://www.youtube.com/shorts/e2B1_DNpsV4");
    none("wearing shorts/pants today");
    none("live/nope");
    none("olive/e2B1_DNpsV4");
    is(
      "playlist?list=PLl9i0hDNBdZMxx",
      "playlist?list=PLl9i0hDNBdZMxx",
      "https://www.youtube.com/playlist?list=PLl9i0hDNBdZMxx",
    );
    assert_eq!(one("channel/UCl9i0hDNBdZMxxxxxxxxxx").1, "https://www.youtube.com/channel/UCl9i0hDNBdZMxxxxxxxxxx");
    none("@someverylonghandle");
  }

  #[test]
  fn tweets() {
    is(
      "GripAshford/status/2107279328723484841",
      "GripAshford/status/2107279328723484841",
      "https://x.com/GripAshford/status/2107279328723484841",
    );
    none("my status/12345678 order");
    none("build/status/42");
    assert_eq!(
      one("lol GripAshford/status/2107279328723484841?s=20").1,
      "https://x.com/GripAshford/status/2107279328723484841"
    );
    is(
      "https://x.com/a/status/2107279328723484841",
      "https://x.com/a/status/2107279328723484841",
      "https://x.com/a/status/2107279328723484841",
    );
  }

  #[test]
  fn reddit_posts_and_names() {
    let post = "r/PlayTheBazaar/comments/1xj6sir/new_item_preview/";
    is(&format!("look {post} lol"), post, "https://www.reddit.com/r/PlayTheBazaar/comments/1xj6sir/new_item_preview");
    is(
      "r/PlayTheBazaar/comments/1xk7c35/new_items_preview_from_whisperzz_live_stream/",
      "r/PlayTheBazaar/comments/1xk7c35/new_items_preview_from_whisperzz_live_stream/",
      "https://www.reddit.com/r/PlayTheBazaar/comments/1xk7c35/new_items_preview_from_whisperzz_live_stream",
    );
    is("see r/place", "r/place", "https://www.reddit.com/r/place");
    is("u/spez", "u/spez", "https://www.reddit.com/user/spez");
    is("/user/spez", "/user/spez", "https://www.reddit.com/user/spez");
    none("r/place/whatever");
    none("w/e 24/7 and/or");
    none("r/");
    none("r/ab");
    none("r/_abc");
    // inside a real url the fragment is left alone: the whole word is the one link
    is("reddit.com/r/place", "reddit.com/r/place", "https://reddit.com/r/place");
  }

  #[test]
  fn bare_hosts() {
    is("go to heatsync.org", "heatsync.org", "https://heatsync.org");
    is("7tv.app", "7tv.app", "https://7tv.app");
    is("HeatSync.ORG", "HeatSync.ORG", "https://heatsync.org");
    is("a.com.au", "a.com.au", "https://a.com.au");
    none("lol.im dead");
    none("wait.what");
    none("nice.jpg");
    none("mail me at x@heatsync.org");
    none("heatsync.org.");
    none("x.com.zzz");
    is("heatsync.org,", "heatsync.org", "https://heatsync.org");
  }

  #[test]
  fn defanged() {
    is("go to https://heatsync (DOT) org now", "https://heatsync (DOT) org", "https://heatsync.org");
    is("hxxps://evil[.]example", "hxxps://evil[.]example", "https://evil.example");
    is("hxxp://evil[.]example", "hxxp://evil[.]example", "http://evil.example");
    is("heatsync(dot)org/emotes", "heatsync(dot)org/emotes", "https://heatsync.org/emotes");
    is("join heatsync dot org today", "heatsync dot org", "https://heatsync.org");
    is("site (.) com", "site (.) com", "https://site.com");
    is("site {dot} com", "site {dot} com", "https://site.com");
    is("a dot b dot com", "a dot b dot com", "https://a.b.com");
    is("HEATSYNC (DOT) ORG", "HEATSYNC (DOT) ORG", "https://heatsync.org");
    none("rated 3 dot 5 stars");
    none("v1 (dot) 2");
    none("the dot");
  }

  #[test]
  fn hostile_text_is_bounded_and_boundary_safe() {
    let long = format!("{}.com/{}", "a".repeat(5000), "b".repeat(5000));
    assert!(find(&long, true).is_empty());
    let many = "a.com ".repeat(500);
    assert_eq!(find(&many, true).len(), MAX_LINKS);
    for t in ["é.com", "héllo wörld r/éé", "(dot)é", "a (dot) é.com/é", "watch?v=ééééééé", "x\u{a0}dot\u{a0}org"]
    {
      for l in find(t, true) {
        assert!(t.is_char_boundary(l.range.start) && t.is_char_boundary(l.range.end), "{t}");
      }
    }
  }
}
