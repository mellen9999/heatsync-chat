// a chatter's sub flair: the twitch sub badge they picked on heatsync to wear in its chat, drawn as a badge
// before the name on twitch lines. asked by twitch id in batches (`/api/users/flair?ids=`), kept ten
// minutes. hostile input: the picture must pass the image allowlist, the channel name is cut to a login
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::badges::Badge;
use crate::images::url::allowed;

// the contract's ttl, for a flair and for "has none" alike
const TTL: Duration = Duration::from_secs(600);
const INFLIGHT: Duration = Duration::from_secs(30);
const MAX_CHATTERS: usize = 500;
const MAX_URL: usize = 512;

struct Entry {
  badge: Option<Badge>,
  fetched: Option<Instant>,
  asked: Option<Instant>,
}

#[derive(Default)]
pub struct Flairs {
  map: HashMap<u64, Entry>,
  wanted: Vec<u64>,
  // the synthetic flood's made-up ids are never asked about
  off: bool,
}

impl Flairs {
  // what `id` wears. queues a fetch when unknown or stale; stale data keeps serving meanwhile
  pub fn get(&mut self, id: u64) -> Option<&Badge> {
    if self.off {
      return None;
    }
    let now = Instant::now();
    if !self.map.contains_key(&id) {
      if self.map.len() >= MAX_CHATTERS {
        self.evict();
      }
      self.map.insert(id, Entry { badge: None, fetched: None, asked: None });
    }
    let e = self.map.get_mut(&id)?;
    if e.fetched.is_none_or(|f| now - f > TTL) && e.asked.is_none_or(|a| now - a > INFLIGHT) {
      e.asked = Some(now);
      self.wanted.push(id);
    }
    e.badge.as_ref()
  }

  // account:changed (a flair set or cleared on heatsync, a link or unlink): ask again about everyone held.
  // what they wear keeps showing until the answer lands, and `put` says which rows to repaint
  pub fn stale(&mut self) {
    if self.off {
      return;
    }
    let now = Instant::now();
    for (&id, e) in &mut self.map {
      e.fetched = None;
      e.asked = Some(now);
      self.wanted.push(id);
    }
  }

  // never ask (the synthetic flood)
  pub fn mute(&mut self) {
    self.off = true;
  }

  pub fn take_wanted(&mut self) -> Vec<u64> {
    std::mem::take(&mut self.wanted)
  }

  // an answer for `id`. true when what their lines show may have changed
  pub fn put(&mut self, id: u64, badge: Option<Badge>) -> bool {
    let Some(e) = self.map.get_mut(&id) else { return false };
    let changed = e.badge.as_ref().map(|b| &b.url) != badge.as_ref().map(|b| &b.url);
    e.badge = badge;
    e.fetched = Some(Instant::now());
    e.asked = None;
    changed
  }

  fn evict(&mut self) {
    let oldest = self.map.iter().min_by_key(|(_, e)| e.fetched.or(e.asked)).map(|(&k, _)| k);
    if let Some(k) = oldest {
      self.map.remove(&k);
    }
  }
}

// one flair row: only the twitch sub kind, with a picture on the allowlist
fn badge(v: &Value) -> Option<Badge> {
  if v.get("flair_type").and_then(Value::as_str) != Some("twitch_sub") {
    return None;
  }
  let url = v.get("badge_url").and_then(Value::as_str).filter(|u| u.len() <= MAX_URL && allowed(u))?;
  let who: String = v
    .get("broadcaster_login")
    .and_then(Value::as_str)
    .unwrap_or("")
    .chars()
    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
    .take(25)
    .collect();
  let title = if who.is_empty() { "sub".to_owned() } else { format!("{who} sub") };
  Some(Badge { url: url.into(), title: title.into(), bg: None })
}

// `GET /api/users/flair` answer: `flairs{id:{…}}`, read back per id asked (absent = wears none)
pub fn parse(v: &Value, asked: &[u64]) -> Option<Vec<(u64, Option<Badge>)>> {
  let flairs = v.get("flairs")?.as_object()?;
  Some(asked.iter().map(|id| (*id, flairs.get(&id.to_string()).and_then(badge))).collect())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn sub(url: &str, who: &str) -> Value {
    json!({"flair_type": "twitch_sub", "broadcaster_login": who, "badge_url": url, "badge_url_4x": url, "tier": "1000"})
  }

  #[test]
  fn reads_only_twitch_sub_flairs_on_the_allowlist() {
    let v = json!({"flairs": {
      "1": sub("https://static-cdn.jtvnw.net/badges/v1/a/1", "examplestreamer"),
      "2": sub("https://evil.example/a", "x"),
      "3": {"flair_type": "something_else", "badge_url": "https://static-cdn.jtvnw.net/a"},
      "4": sub("http://static-cdn.jtvnw.net/a", "x"),
      "5": sub("https://static-cdn.jtvnw.net/a", "we ird\n<b>"),
      "7": 5
    }});
    let got = parse(&v, &[1, 2, 3, 4, 5, 6, 7]).unwrap();
    let urls: Vec<_> =
      got.iter().map(|(i, b)| (*i, b.as_ref().map(|b| (b.url.to_string(), b.title.to_string())))).collect();
    assert_eq!(urls[0], (1, Some(("https://static-cdn.jtvnw.net/badges/v1/a/1".into(), "examplestreamer sub".into()))));
    assert!(urls[1].1.is_none() && urls[2].1.is_none() && urls[3].1.is_none(), "{urls:?}");
    assert_eq!(urls[4].1.as_ref().unwrap().1, "weirdb sub", "the name is cut to login characters");
    assert!(urls[5].1.is_none() && urls[6].1.is_none());
    for bad in [json!(null), json!({"flairs": []}), json!({})] {
      assert!(parse(&bad, &[1]).is_none());
    }
  }

  #[test]
  fn a_long_url_is_refused() {
    let long = format!("https://static-cdn.jtvnw.net/{}", "a".repeat(MAX_URL));
    assert!(badge(&sub(&long, "x")).is_none());
  }

  #[test]
  fn an_account_change_asks_again_about_everyone_and_keeps_showing_meanwhile() {
    let mut f = Flairs::default();
    f.get(7);
    f.take_wanted();
    let b = Badge { url: "https://static-cdn.jtvnw.net/x".into(), title: "sub".into(), bg: None };
    f.put(7, Some(b));
    f.stale();
    assert_eq!(f.take_wanted(), vec![7]);
    assert!(f.get(7).is_some(), "the old flair shows until the answer");
    assert!(f.take_wanted().is_empty(), "asked once, not again while in flight");
    assert!(f.put(7, None), "a cleared flair repaints");
    assert!(f.get(7).is_none());
  }

  #[test]
  fn asked_once_served_stale_and_bounded() {
    let mut f = Flairs::default();
    assert!(f.get(7).is_none());
    assert_eq!(f.take_wanted(), [7]);
    f.get(7);
    assert!(f.take_wanted().is_empty(), "in flight");
    let b = Badge { url: "https://static-cdn.jtvnw.net/a".into(), title: "x sub".into(), bg: None };
    assert!(f.put(7, Some(b.clone())));
    assert!(!f.put(7, Some(b)), "same picture, nothing to redraw");
    assert!(f.get(7).is_some());
    assert!(f.take_wanted().is_empty());
    f.map.get_mut(&7).unwrap().fetched = Some(Instant::now() - TTL - Duration::from_secs(1));
    assert!(f.get(7).is_some(), "stale data keeps serving");
    assert_eq!(f.take_wanted(), [7]);
    for i in 0..MAX_CHATTERS as u64 + 100 {
      f.get(1000 + i);
    }
    assert_eq!(f.map.len(), MAX_CHATTERS);
  }
}
