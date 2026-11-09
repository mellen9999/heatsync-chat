// the picker's gifs tab (ext gifs.js + gif-search-remote.js). the search is a request to heatsync and
// nothing else: its own corpus answers `/api/gifs/search?q=` and `/api/gifs/top`, and only the
// pictures come from the provider's cdn (we go through heatsync's image proxy, so the cdn never sees
// us). every field of an answer is hostile: urls are checked, text is bounded, rows are capped.
// picking inserts the gif's direct url, which chat then embeds
use crate::i18n::{f, t};
use serde_json::Value;

use crate::images::url::host;
use crate::store;

// rows per page, and a hard ceiling whatever the server sends (ext GIF_PAGE_SIZE, HS_GIF_CAP)
pub const PAGE: usize = 24;
pub const RECENT_CAP: usize = 12;
const RECENT_FILE: &str = "recent-gifs";
const MAX_QUERY: usize = 64;
// short enough that the proxied form stays under the 2048 url cap even fully escaped
const MAX_URL: usize = 600;
const MAX_LABEL: usize = 80;
// the ext keeps 20 searches for 5 minutes
const CACHE_MAX: usize = 20;
const CACHE_MS: i64 = 5 * 60_000;
// providers heatsync indexes (giphy today, tenor in the ext's notes), and heatsync's own files
const HOSTS: [&str; 3] = ["giphy.com", "tenor.com", "heatsync.org"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gif {
  pub id: Box<str>,
  // the direct media url that goes in the message
  pub url: Box<str>,
  // the still the grid paints, and the moving one the picked tile plays
  pub preview: Box<str>,
  pub animated: Box<str>,
  pub label: Box<str>,
}

// what the server says about its own corpus: an empty one is our cold start, not a bad query
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Library {
  #[default]
  Ready,
  Empty,
  Building(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Page {
  pub gifs: Vec<Gif>,
  pub library: Library,
}

fn good(u: &str) -> bool {
  if u.len() > MAX_URL || u.bytes().any(|b| b.is_ascii_control() || b == b' ') {
    return false;
  }
  let Some(h) = host(u).map(str::to_ascii_lowercase) else { return false };
  HOSTS.iter().any(|a| h == *a || h.strip_suffix(a).is_some_and(|p| p.ends_with('.')))
}

fn text(v: Option<&Value>, max: usize) -> Box<str> {
  let s: String = v.and_then(Value::as_str).unwrap_or("").chars().filter(|c| !c.is_control()).take(max).collect();
  s.trim().into()
}

// one record, or None when its one needed field (a good url) is missing. a missing or bad still or
// animation falls back to the url itself, as the ext does
pub fn gif(v: &Value) -> Option<Gif> {
  let url = v.get("url")?.as_str().filter(|u| good(u))?;
  let alt = |k: &str| v.get(k).and_then(Value::as_str).filter(|u| good(u)).unwrap_or(url);
  let id = match v.get("id") {
    Some(Value::String(s)) => crate::util::strip(s).take(40).collect(),
    Some(Value::Number(n)) => n.to_string(),
    _ => url.to_owned(),
  };
  Some(Gif {
    id: id.into(),
    url: url.into(),
    preview: alt("preview").into(),
    animated: alt("animated").into(),
    label: text(v.get("label"), MAX_LABEL),
  })
}

pub fn parse(v: &Value) -> Page {
  let rows = v.get("gifs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  let mut seen = std::collections::HashSet::new();
  let gifs = rows.iter().take(PAGE * 2).filter_map(gif).filter(|g| seen.insert(g.url.clone())).take(PAGE).collect();
  let lib = v.get("library");
  let indexed = lib.and_then(|l| l.get("indexed")).and_then(Value::as_u64).unwrap_or(0);
  let library = match lib.and_then(|l| l.get("state")).and_then(Value::as_str) {
    Some("empty") => Library::Empty,
    Some("building") => Library::Building(indexed),
    _ => Library::Ready,
  };
  Page { gifs, library }
}

// the query as typed, trimmed and bounded; empty = the top list
pub fn clean(q: &str) -> String {
  crate::util::strip(q).take(MAX_QUERY).collect::<String>().trim().to_owned()
}

pub fn path(q: &str) -> String {
  match clean(q).as_str() {
    "" => format!("/api/gifs/top?limit={PAGE}"),
    q => format!("/api/gifs/search?q={}&limit={PAGE}", crate::ui::embed::escape(q)),
  }
}

// what an empty grid says (an empty library is ours, not the reader's query)
pub fn empty_note(q: &str, lib: Library) -> String {
  match (lib, q.is_empty()) {
    (Library::Empty, _) => t("chat1.gifs.empty_lib").into(),
    (Library::Building(n), false) => f("chat1.gifs.building", &[("n", &n.to_string())]),
    (_, false) => t("chat1.gifs.none").into(),
    (_, true) => t("chat1.gifs.prompt").into(),
  }
}

// why an ask failed: the status (0 = no answer), and for a 429 the seconds the server says to wait
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fail {
  pub status: u16,
  pub wait: u32,
}

impl Fail {
  // `resetAt` is a ms epoch in the body (ext: ceil((resetAt - now) / 1000), at least 1, bounded here)
  pub fn of(status: u16, body: &Value, now: i64) -> Fail {
    let at = body.get("resetAt").and_then(Value::as_i64).unwrap_or(0);
    let wait = if status == 429 { ((at - now + 999) / 1000).clamp(1, 3600) as u32 } else { 0 };
    Fail { status, wait }
  }

  // what a failed ask says, in plain words
  pub fn note(self) -> String {
    match self.status {
      0 => t("chat1.gifs.offline").into(),
      429 => f("chat1.gifs.rate", &[("n", &self.wait.max(1).to_string())]),
      _ => t("chat1.gifs.fail").into(),
    }
  }
}

// searches already answered, newest last, for a few minutes
#[derive(Default)]
pub struct Cache(Vec<(Box<str>, i64, Page)>);

impl Cache {
  pub fn get(&self, q: &str, now: i64) -> Option<&Page> {
    self.0.iter().find(|(k, at, _)| **k == *q && now - at < CACHE_MS).map(|e| &e.2)
  }

  pub fn put(&mut self, q: &str, now: i64, page: Page) {
    self.0.retain(|e| *e.0 != *q);
    self.0.push((q.into(), now, page));
    if self.0.len() > CACHE_MAX {
      self.0.remove(0);
    }
  }
}

// the last gifs you sent, newest first, kept in a file like the emote recents
pub fn load_recent() -> Vec<Gif> {
  let Some(v) = store::read(RECENT_FILE).and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
    return Vec::new();
  };
  v.as_array().map(|a| a.iter().filter_map(gif).take(RECENT_CAP).collect()).unwrap_or_default()
}

// move `g` to the front (deduped by url, capped)
pub fn bump(list: &mut Vec<Gif>, g: &Gif) {
  list.retain(|x| x.url != g.url);
  list.insert(0, g.clone());
  list.truncate(RECENT_CAP);
}

pub fn save_recent(list: &[Gif]) {
  let rows: Vec<Value> = list
    .iter()
    .map(|g| serde_json::json!({ "id": &*g.id, "url": &*g.url, "preview": &*g.preview, "animated": &*g.animated, "label": &*g.label }))
    .collect();
  if let Err(e) = store::write(RECENT_FILE, &Value::Array(rows).to_string()) {
    eprintln!("save recent gifs: {e}");
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn row(n: u32) -> Value {
    json!({
      "id": n, "provider": "giphy", "label": format!("cat {n}"), "uses": 5,
      "url": format!("https://media.giphy.com/media/a{n}/giphy.gif"),
      "preview": format!("https://media.giphy.com/media/a{n}/200_s.gif"),
      "animated": format!("https://media.giphy.com/media/a{n}/200.gif"),
    })
  }

  #[test]
  fn a_real_answer_parses() {
    let p = parse(&json!({ "gifs": [row(1), row(2)], "library": { "state": "ready", "indexed": 204677 } }));
    assert_eq!(p.gifs.len(), 2);
    assert_eq!(&*p.gifs[0].id, "1");
    assert_eq!(&*p.gifs[0].label, "cat 1");
    assert_eq!(&*p.gifs[0].preview, "https://media.giphy.com/media/a1/200_s.gif");
    assert_eq!(p.library, Library::Ready);
  }

  #[test]
  fn hostile_answers_are_bounded_and_checked() {
    // not json shaped at all
    for v in [Value::Null, json!([]), json!({"gifs": 7}), json!({"gifs": [1, "x", null, {}]})] {
      assert_eq!(parse(&v), Page::default());
    }
    // a bad host, scheme, userinfo, port, control byte or an over-long url drops the row
    let bad = [
      "http://media.giphy.com/a.gif",
      "https://evil.com/a.gif",
      "https://giphy.com.evil.com/a.gif",
      "https://evilgiphy.com/a.gif",
      "https://user@media.giphy.com/a.gif",
      "https://media.giphy.com:8443/a.gif",
      "javascript:alert(1)",
      "https://media.giphy.com/a b.gif",
      "https://media.giphy.com/a\n.gif",
    ];
    for u in bad {
      assert_eq!(gif(&json!({ "url": u })), None, "{u}");
    }
    let long = format!("https://media.giphy.com/{}", "a".repeat(MAX_URL));
    assert_eq!(gif(&json!({ "url": long })), None);
    // a bad still falls back to the gif itself; a bad label is cut and cleaned
    let g = gif(&json!({ "url": "https://media.tenor.com/x.gif", "preview": "https://evil.com/p.gif", "label": format!("a\u{7}b{}", "x".repeat(500)) })).unwrap();
    assert_eq!((&*g.preview, &*g.animated), ("https://media.tenor.com/x.gif", "https://media.tenor.com/x.gif"));
    assert!(g.label.chars().count() <= MAX_LABEL && !g.label.contains('\u{7}'));
    assert_eq!(&*g.id, "https://media.tenor.com/x.gif");
    // a flood is capped at a page, repeats dropped
    let many: Vec<Value> = (0..500).map(row).collect();
    assert_eq!(parse(&json!({ "gifs": many })).gifs.len(), PAGE);
    let dup: Vec<Value> = (0..30).map(|_| row(1)).collect();
    assert_eq!(parse(&json!({ "gifs": dup })).gifs.len(), 1);
  }

  #[test]
  fn library_states() {
    let st = |s: &str| parse(&json!({ "gifs": [], "library": { "state": s, "indexed": 12 } })).library;
    assert_eq!(st("empty"), Library::Empty);
    assert_eq!(st("building"), Library::Building(12));
    assert_eq!(st("surprise"), Library::Ready);
    assert!(empty_note("cat", Library::Empty).contains("library"));
    assert!(empty_note("cat", Library::Building(3)).contains('3'));
    assert_eq!(empty_note("cat", Library::Ready), "no gifs match");
    assert_eq!(empty_note("", Library::Ready), "type to search gifs");
  }

  #[test]
  fn failures_say_why() {
    let b = json!({ "resetAt": 10_500 });
    assert_eq!(Fail::of(429, &b, 3_000).note(), "too many searches, wait 8s");
    // no or past reset time: at least a second; absurd ones are bounded; other statuses ignore it
    assert_eq!(Fail::of(429, &Value::Null, 3_000).wait, 1);
    assert_eq!(Fail::of(429, &json!({ "resetAt": 9e15 as i64 }), 0).wait, 3600);
    assert_eq!(Fail::of(500, &b, 0), Fail { status: 500, wait: 0 });
    assert_eq!(Fail::of(500, &b, 0).note(), "couldn't load gifs");
    assert_eq!(Fail::of(0, &Value::Null, 0).note(), "couldn't reach heatsync");
  }

  #[test]
  fn paths_escape_and_bound_the_query() {
    assert_eq!(path(""), "/api/gifs/top?limit=24");
    assert_eq!(path("  "), "/api/gifs/top?limit=24");
    assert_eq!(path("a b&c=d"), "/api/gifs/search?q=a%20b%26c%3Dd&limit=24");
    assert_eq!(clean(&"x".repeat(500)).len(), MAX_QUERY);
    assert_eq!(clean("a\nb"), "ab");
  }

  #[test]
  fn cache_expires_and_is_bounded() {
    let mut c = Cache::default();
    let page = parse(&json!({ "gifs": [row(1)] }));
    c.put("cat", 0, page.clone());
    assert_eq!(c.get("cat", CACHE_MS - 1), Some(&page));
    assert_eq!(c.get("cat", CACHE_MS), None);
    for n in 0..40 {
      c.put(&format!("q{n}"), 0, Page::default());
    }
    assert_eq!(c.0.len(), CACHE_MAX);
    assert!(c.get("q39", 0).is_some() && c.get("q0", 0).is_none());
  }

  #[test]
  fn recents_are_mru_deduped_and_capped() {
    let mut l = Vec::new();
    for n in 0..30 {
      bump(&mut l, &gif(&row(n)).unwrap());
    }
    bump(&mut l, &gif(&row(20)).unwrap());
    assert_eq!(l.len(), RECENT_CAP);
    assert_eq!(&*l[0].id, "20");
    assert_eq!(l.iter().filter(|g| &*g.id == "20").count(), 1);
  }
}
