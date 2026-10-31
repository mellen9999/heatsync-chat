// each chatter's own heatsync inventory. privacy invariant: a heatsync emote renders on a line
// iff THAT SENDER owned it when they sent it. nothing here ever comes from a catalog or search
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{cw_blocked, valid_name};
use crate::images::url::allowed;
use crate::util::time::parse_iso;

pub struct HsEmote {
  pub url: Box<str>,
  pub w: u16,
  pub h: u16,
  pub zero_width: bool,
  // ownership window in unix ms; a line renders the emote only if sent inside it
  from: i64,
  until: i64,
}

pub type Set = HashMap<Box<str>, HsEmote>;

// clock skew between the sender, twitch and heatsync
const SLACK_MS: i64 = 120_000;
const TTL_HIT: Duration = Duration::from_secs(15 * 60);
const TTL_MISS: Duration = Duration::from_secs(90);
// a request that never answered (network drop) may be retried after this
const INFLIGHT: Duration = Duration::from_secs(30);
const MAX_SENDERS: usize = 500;
const MAX_ROWS: usize = 5000;
const MAX_URL: usize = 2048;

struct Entry {
  set: Set,
  fetched: Option<Instant>,
  asked: Option<Instant>,
}

#[derive(Default)]
pub struct Senders {
  map: HashMap<u64, Entry>,
  wanted: Vec<u64>,
}

impl Senders {
  // the emote `word` as `id` owned it at `ts`. queues a fetch when the set is unknown or stale
  pub fn lookup(&mut self, id: u64, word: &str, ts: i64) -> Option<&HsEmote> {
    let now = Instant::now();
    if !self.map.contains_key(&id) && self.map.len() >= MAX_SENDERS {
      self.evict();
    }
    let e = self.map.entry(id).or_insert(Entry { set: Set::new(), fetched: None, asked: None });
    let ttl = if e.set.is_empty() { TTL_MISS } else { TTL_HIT };
    let stale = e.fetched.is_none_or(|f| now - f > ttl);
    if stale && e.asked.is_none_or(|a| now - a > INFLIGHT) {
      e.asked = Some(now);
      self.wanted.push(id);
    }
    let e = self.map.get(&id)?;
    e.set.get(word).filter(|x| ts >= x.from.saturating_sub(SLACK_MS) && ts < x.until.saturating_add(SLACK_MS))
  }

  // what `id` can type right now (tab-complete for our own lines)
  pub fn names(&self, id: u64, now: i64) -> Vec<&str> {
    self.map.get(&id).map_or_else(Vec::new, |e| {
      e.set.iter().filter(|(_, x)| now >= x.from.saturating_sub(SLACK_MS) && now < x.until).map(|(k, _)| &**k).collect()
    })
  }

  // what `id` owns right now, with images (the picker's "my emotes")
  pub fn entries(&self, id: u64, now: i64) -> Vec<(&str, &HsEmote)> {
    self.map.get(&id).map_or_else(Vec::new, |e| {
      e.set
        .iter()
        .filter(|(_, x)| now >= x.from.saturating_sub(SLACK_MS) && now < x.until)
        .map(|(k, v)| (&**k, v))
        .collect()
    })
  }

  pub fn take_wanted(&mut self) -> Vec<u64> {
    std::mem::take(&mut self.wanted)
  }

  // true when lines from this sender may now render differently
  pub fn put(&mut self, id: u64, set: Set) -> bool {
    let Some(e) = self.map.get_mut(&id) else { return false };
    let changed = !(e.set.is_empty() && set.is_empty());
    e.set = set;
    e.fetched = Some(Instant::now());
    e.asked = None;
    changed
  }

  // the server said this sender's inventory changed. None = never seen them, nothing to do.
  // else the caller refetches now; Some(true) = a removal already took an emote off their lines
  pub fn stale(&mut self, id: u64, removed: Option<&str>) -> Option<bool> {
    let e = self.map.get_mut(&id)?;
    e.fetched = None;
    e.asked = Some(Instant::now());
    Some(removed.is_some_and(|w| e.set.remove(w).is_some()))
  }

  fn evict(&mut self) {
    let oldest = self.map.iter().min_by_key(|(_, e)| e.fetched.or(e.asked)).map(|(&k, _)| k);
    if let Some(k) = oldest {
      self.map.remove(&k);
    }
  }
}

// one sender's rows from `/api/users/emotes/batch`
pub fn parse_set(v: &Value) -> Set {
  let Some(rows) = v.as_array() else { return Set::new() };
  let mut set = Set::new();
  for r in rows.iter().take(MAX_ROWS) {
    let s = |k: &str| r.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let Some(name) = s("custom_name").or_else(|| s("name")).or_else(|| s("code")).filter(|n| valid_name(n)) else {
      continue;
    };
    let Some(url) = s("url").or_else(|| s("src")).filter(|u| u.len() <= MAX_URL && allowed(u)) else { continue };
    let dim = |k: &str| r.get(k).and_then(Value::as_u64).map_or(0, |n| n.min(u16::MAX as u64) as u16);
    let (w, h) = (dim("width"), dim("height"));
    if h == 0 || cw_blocked(r) {
      continue;
    }
    set.insert(
      name.into(),
      HsEmote {
        url: url.into(),
        w,
        h,
        zero_width: r.get("zero_width").and_then(Value::as_bool) == Some(true),
        from: s("added_at").and_then(parse_iso).unwrap_or(i64::MIN),
        until: s("until").and_then(parse_iso).unwrap_or(i64::MAX),
      },
    );
  }
  set
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  const T: i64 = 1_793_399_211_099;

  fn row(name: &str) -> Value {
    json!({"custom_name": name, "url": "https://cdn.7tv.app/emote/x/1x.webp", "width": 32, "height": 32,
      "added_at": "2026-10-30T22:26:51.099Z"})
  }

  #[test]
  fn parse_gates() {
    let rows = json!([
      row("ok"),
      {"name": "nsfw", "url": "https://cdn.7tv.app/a", "height": 1, "nsfw": true},
      {"name": "evil", "url": "https://evil.example/a", "height": 1},
      {"name": "flat", "url": "https://cdn.7tv.app/a", "height": 0},
      {"name": "two words", "url": "https://cdn.7tv.app/a", "height": 1},
    ]);
    let set = parse_set(&rows);
    assert_eq!(set.len(), 1);
    assert!(set.contains_key("ok"));
  }

  #[test]
  fn ownership_window() {
    let mut s = Senders::default();
    assert!(s.lookup(1, "ok", T).is_none());
    assert_eq!(s.take_wanted(), vec![1]);
    // in flight: no duplicate request
    s.lookup(1, "ok", T);
    assert!(s.take_wanted().is_empty());
    assert!(s.put(1, parse_set(&json!([row("ok")]))));
    assert!(s.lookup(1, "ok", T).is_some());
    assert!(s.lookup(1, "ok", T - SLACK_MS + 1).is_some());
    // sent before they owned it: plain text
    assert!(s.lookup(1, "ok", T - SLACK_MS - 1).is_none());
    // someone else's line never borrows this set
    assert!(s.lookup(2, "ok", T).is_none());
    assert_eq!(s.stale(1, Some("ok")), Some(true));
    assert_eq!(s.stale(99, None), None);
    assert!(s.lookup(1, "ok", T).is_none());
    // the stale refetch is already in flight
    assert_eq!(s.take_wanted(), vec![2]);
  }

  #[test]
  fn empty_answer_is_not_a_change() {
    let mut s = Senders::default();
    s.lookup(7, "x", T);
    assert!(!s.put(7, Set::new()));
  }
}
