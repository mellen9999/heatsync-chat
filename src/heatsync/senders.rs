// each chatter's own heatsync inventory. privacy invariant: a heatsync emote renders on a line
// iff THAT SENDER owned it when they sent it. nothing here ever comes from a catalog or search
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::content::emote_hidden;
use super::valid_name;
use crate::images::url::allowed;
use crate::util::time::{now_ms, parse_iso};

#[derive(PartialEq)]
pub struct HsEmote {
  pub url: Box<str>,
  pub w: u16,
  pub h: u16,
  pub zero_width: bool,
  // ownership window in unix ms (the ext's addedAt / untilAt / removedAt): a line draws the emote only if
  // sent inside it. `until` is the server's (a past pairing's last use), `removed` is our own tombstone:
  // the moment we learned they dropped it. i64::MIN / i64::MAX = unbounded
  from: i64,
  until: i64,
  removed: i64,
}

pub type Set = HashMap<Box<str>, HsEmote>;

// clock skew between the sender, twitch and heatsync (the ext's EMOTE_ADDED_SKEW_MS): before added_at,
// and after a server `until`. a removal tombstone is our own clock: no slack
const SKEW_MS: i64 = 120_000;
const TTL_HIT: Duration = Duration::from_secs(15 * 60);
const TTL_MISS: Duration = Duration::from_secs(90);
// a request that never answered (network drop) may be retried after this
const INFLIGHT: Duration = Duration::from_secs(30);
const MAX_SENDERS: usize = 500;
// rows per sender, tombstones included (the oldest tombstones go first past it)
const MAX_ROWS: usize = 5000;
const MAX_URL: usize = 2048;

impl HsEmote {
  // the ext's _sGate: was it theirs at `ts`? `skew` pads the start only
  fn at(&self, ts: i64, skew: i64) -> bool {
    ts >= self.from.saturating_sub(skew) && ts < self.removed && ts < self.until.saturating_add(SKEW_MS)
  }

  // theirs right now (no slack past a bound: a past pairing or a removal is not theirs any more)
  fn now(&self, now: i64) -> bool {
    self.removed == i64::MAX && now >= self.from.saturating_sub(SKEW_MS) && now < self.until
  }
}

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
    self.map.get(&id)?.set.get(word).filter(|x| x.at(ts, SKEW_MS))
  }

  // `word` as `id` owns it right now, without asking for the set (our own is fetched at login)
  pub fn peek(&self, id: u64, word: &str, now: i64) -> Option<&HsEmote> {
    self.map.get(&id)?.set.get(word).filter(|x| x.now(now))
  }

  // what `id` owns right now, with images (the picker's "my emotes")
  pub fn entries(&self, id: u64, now: i64) -> Vec<(&str, &HsEmote)> {
    self
      .map
      .get(&id)
      .map_or_else(Vec::new, |e| e.set.iter().filter(|(_, x)| x.now(now)).map(|(k, v)| (&**k, v)).collect())
  }

  pub fn take_wanted(&mut self) -> Vec<u64> {
    std::mem::take(&mut self.wanted)
  }

  // a fresh fetch of their set, merged like the ext's replaceSenderEmotes: a name it lacks is tombstoned
  // (lines sent while they had it keep it), not dropped; a name it has clears any tombstone and keeps the
  // earliest start we knew, so a re-add never takes the emote off lines it already drew on. true when
  // lines from this sender may now render differently
  pub fn put(&mut self, id: u64, fresh: Set) -> bool {
    let Some(e) = self.map.get_mut(&id) else { return false };
    e.fetched = Some(Instant::now());
    e.asked = None;
    merge(&mut e.set, fresh, now_ms())
  }

  // the server said this sender's inventory changed. None = never seen them, nothing to do.
  // else the caller refetches now; Some(true) = a removal closed an emote's window on their lines
  pub fn stale(&mut self, id: u64, removed: Option<&str>) -> Option<bool> {
    let e = self.map.get_mut(&id)?;
    e.fetched = None;
    e.asked = Some(Instant::now());
    let x = removed.and_then(|w| e.set.get_mut(w)).filter(|x| x.removed == i64::MAX);
    Some(x.map(|x| x.removed = now_ms()).is_some())
  }

  fn evict(&mut self) {
    let oldest = self.map.iter().min_by_key(|(_, e)| e.fetched.or(e.asked)).map(|(&k, _)| k);
    if let Some(k) = oldest {
      self.map.remove(&k);
    }
  }
}

fn merge(set: &mut Set, mut fresh: Set, now: i64) -> bool {
  let mut changed = false;
  set.retain(|name, x| {
    if fresh.contains_key(name) || x.removed != i64::MAX {
      return true;
    }
    changed = true;
    // a row with no start was never confirmed owned: nothing to keep it for
    x.removed = now;
    x.from != i64::MIN
  });
  for (name, mut x) in fresh.drain() {
    if let Some(old) = set.get(&name) {
      if old.from != i64::MIN && x.from != i64::MIN {
        x.from = x.from.min(old.from);
      }
      if *old == x {
        continue;
      }
    }
    set.insert(name, x);
    changed = true;
  }
  if set.len() > MAX_ROWS {
    let mut dead: Vec<(i64, Box<str>)> =
      set.iter().filter(|(_, x)| x.removed != i64::MAX).map(|(k, x)| (x.removed, k.clone())).collect();
    dead.sort_unstable();
    for (_, k) in dead.into_iter().take(set.len() - MAX_ROWS) {
      set.remove(&k);
    }
  }
  changed
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
    if h == 0 || emote_hidden(r) {
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
        removed: i64::MAX,
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
    assert!(s.lookup(1, "ok", T - SKEW_MS + 1).is_some());
    // sent before they owned it: plain text
    assert!(s.lookup(1, "ok", T - SKEW_MS - 1).is_none());
    // someone else's line never borrows this set
    assert!(s.lookup(2, "ok", T).is_none());
    assert_eq!(s.stale(1, Some("ok")), Some(true));
    assert_eq!(s.stale(99, None), None);
    // already tombstoned: not a second change
    assert_eq!(s.stale(1, Some("ok")), Some(false));
    assert!(s.lookup(1, "ok", T).is_some(), "a line from before the removal keeps it");
    // the stale refetch is already in flight
    assert_eq!(s.take_wanted(), vec![2]);
  }

  #[test]
  fn empty_answer_is_not_a_change() {
    let mut s = Senders::default();
    s.lookup(7, "x", T);
    assert!(!s.put(7, Set::new()));
  }

  // a live removal closes the window instead of deleting: what they said before still draws it, what they
  // say after (no slack) is the plain word
  #[test]
  fn a_removal_keeps_the_emote_on_lines_sent_before_it() {
    let now = crate::util::time::now_ms();
    let mut s = Senders::default();
    s.lookup(1, "ok", now);
    s.put(1, parse_set(&json!([row("ok")])));
    assert_eq!(s.stale(1, Some("ok")), Some(true));
    assert!(s.lookup(1, "ok", now - 1000).is_some(), "said before the removal");
    assert!(s.lookup(1, "ok", now + 1000).is_none(), "said after it");
    assert!(s.peek(1, "ok", now + 1000).is_none(), "not theirs any more");
  }

  // a refetch without a name they had tombstones it; a fresh live row of that name clears the tombstone
  #[test]
  fn a_refetch_missing_a_name_tombstones_it_and_a_fresh_row_clears_it() {
    let now = crate::util::time::now_ms();
    let mut s = Senders::default();
    s.lookup(1, "ok", now);
    s.put(1, parse_set(&json!([row("ok"), row("kept")])));
    assert!(s.put(1, parse_set(&json!([row("kept")]))), "a dropped name is a change");
    assert!(s.lookup(1, "ok", now - 1000).is_some(), "said while they had it");
    assert!(s.lookup(1, "ok", now + 1000).is_none(), "said after the refetch");
    s.put(1, parse_set(&json!([row("ok"), row("kept")])));
    assert!(s.lookup(1, "ok", now + 1000).is_some(), "theirs again");
  }

  // a row never stamped with added_at was never confirmed owned: a refetch without it drops it
  #[test]
  fn an_unstamped_row_is_dropped_not_tombstoned() {
    let now = crate::util::time::now_ms();
    let mut s = Senders::default();
    s.lookup(1, "x", now);
    let bare = json!({"custom_name": "x", "url": "https://cdn.7tv.app/emote/x/1x.webp", "height": 32});
    s.put(1, parse_set(&json!([bare])));
    assert!(s.lookup(1, "x", now - 1000).is_some());
    s.put(1, Set::new());
    assert!(s.lookup(1, "x", now - 1000).is_none());
  }

  // a re-add keeps the earliest start we knew, so the lines it drew on keep it
  #[test]
  fn a_re_add_keeps_the_earliest_start() {
    let mut s = Senders::default();
    s.lookup(1, "ok", T);
    s.put(1, parse_set(&json!([row("ok")])));
    let mut later = row("ok");
    later["added_at"] = json!("2026-11-01T00:00:00.000Z");
    s.put(1, parse_set(&json!([later])));
    assert!(s.lookup(1, "ok", T).is_some());
  }

  // tombstones never grow a set past its cap: the oldest go first
  #[test]
  fn tombstones_stay_bounded() {
    let mut set = Set::new();
    let one = |n: usize| parse_set(&json!([row(&format!("e{n}"))]));
    for n in 0..MAX_ROWS + 10 {
      merge(&mut set, one(n), n as i64);
    }
    assert_eq!(set.len(), MAX_ROWS);
    assert!(set.contains_key(format!("e{}", MAX_ROWS + 9).as_str()), "the live row stays");
    assert!(!set.contains_key("e0"), "the oldest tombstone went");
  }
}
