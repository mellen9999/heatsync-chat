// 7tv cosmetics of chatters: the paint on their name and the badge before it. heatsync's
// `/api/cosmetics/batch` hands them over per twitch id (the ext's route); everything is hostile
pub mod fx;
mod fx_tints;
pub mod paint;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::badges::Badge;
use crate::images::url::allowed;
use paint::Paint;

// ext: 30min for a hit, 5min for "has none" so a fresh grant shows soon; 500 chatters
const TTL_HIT: Duration = Duration::from_secs(30 * 60);
const TTL_MISS: Duration = Duration::from_secs(5 * 60);
const INFLIGHT: Duration = Duration::from_secs(30);
const MAX_USERS: usize = 500;

#[derive(Clone, Debug)]
pub struct Cosmetic {
  pub paint: Option<Arc<Paint>>,
  pub badge: Option<Badge>,
}

// badge image: 2x when 7tv has it (downscaled crisp), else 1x, else any webp
fn badge(v: &Value) -> Option<Badge> {
  let host = v.get("host")?;
  let base = host.get("url")?.as_str()?;
  let base = base.strip_prefix("//").map_or_else(|| base.to_owned(), |b| format!("https://{b}"));
  let files: Vec<&str> = host
    .get("files")?
    .as_array()?
    .iter()
    .filter_map(|f| f.get("name")?.as_str())
    .filter(|n| n.ends_with(".webp") && n.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
    .collect();
  let file = ["2x.webp", "1x.webp"].into_iter().find(|w| files.contains(w)).or_else(|| files.first().copied())?;
  let url = format!("{}/{file}", base.trim_end_matches('/'));
  if !allowed(&url) {
    return None;
  }
  let title = v.get("tooltip").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("7tv");
  Some(Badge {
    url: url.into(),
    title: title.chars().filter(|c| !c.is_control()).take(100).collect::<String>().into(),
    bg: None,
  })
}

// one entry of the batch response. `hs` is the chatter's heatsync-native paint (`/api/paints`), which
// outranks their 7tv one like in the ext. None = nothing to show
pub fn parse(v: &Value, hs: Option<&Value>, paints: &mut HashMap<Box<str>, Arc<Paint>>) -> Option<Cosmetic> {
  let paint = hs.and_then(Paint::from_hs).or_else(|| v.get("paint").and_then(Paint::parse)).map(|p| {
    // one allocation per distinct paint: thousands of chatters share a few hundred
    if paints.len() > 400 {
      paints.clear();
    }
    paints.entry(p.id.clone()).or_insert_with(|| Arc::new(p)).clone()
  });
  let badge = v.get("badge").and_then(badge);
  (paint.is_some() || badge.is_some()).then_some(Cosmetic { paint, badge })
}

struct Entry {
  c: Option<Cosmetic>,
  fetched: Option<Instant>,
  asked: Option<Instant>,
}

pub struct Cosmetics {
  map: HashMap<u64, Entry>,
  wanted: Vec<u64>,
  // the ext setting sevenTvPaints (default on) gates paints and badges alike
  pub on: bool,
}

impl Default for Cosmetics {
  fn default() -> Self {
    Cosmetics { map: HashMap::new(), wanted: Vec::new(), on: true }
  }
}

impl Cosmetics {
  // what `uid` wears. queues a fetch when unknown or stale; stale data keeps serving meanwhile
  pub fn get(&mut self, uid: u64) -> Option<&Cosmetic> {
    if !self.on {
      return None;
    }
    let now = Instant::now();
    if !self.map.contains_key(&uid) && self.map.len() >= MAX_USERS {
      self.evict();
    }
    let e = self.map.entry(uid).or_insert(Entry { c: None, fetched: None, asked: None });
    let ttl = if e.c.is_some() { TTL_HIT } else { TTL_MISS };
    if e.fetched.is_none_or(|f| now - f > ttl) && e.asked.is_none_or(|a| now - a > INFLIGHT) {
      e.asked = Some(now);
      self.wanted.push(uid);
    }
    self.map.get(&uid)?.c.as_ref()
  }

  // a chatter whose cosmetics are already known (the synthetic flood): never fetched
  pub fn seed(&mut self, uid: u64, c: Cosmetic) {
    self.map.insert(uid, Entry { c: Some(c), fetched: Some(Instant::now()), asked: None });
  }

  pub fn take_wanted(&mut self) -> Vec<u64> {
    std::mem::take(&mut self.wanted)
  }

  // an answer for `uid`. true when what their lines show may have changed
  pub fn put(&mut self, uid: u64, c: Option<Cosmetic>) -> bool {
    let Some(e) = self.map.get_mut(&uid) else { return false };
    let same = |a: &Option<Cosmetic>, b: &Option<Cosmetic>| match (a, b) {
      (None, None) => true,
      (Some(a), Some(b)) => {
        a.paint.as_ref().map(|p| &p.id) == b.paint.as_ref().map(|p| &p.id)
          && a.badge.as_ref().map(|x| &x.url) == b.badge.as_ref().map(|x| &x.url)
      }
      _ => false,
    };
    let changed = !same(&e.c, &c);
    e.c = c;
    e.fetched = Some(Instant::now());
    e.asked = None;
    changed
  }

  // 7tv says their cosmetics changed: ask again now (the old look serves until the answer)
  pub fn stale(&mut self, uid: u64) {
    if let Some(e) = self.map.get_mut(&uid) {
      e.fetched = None;
      e.asked = Some(Instant::now());
      self.wanted.push(uid);
    }
  }

  fn evict(&mut self) {
    let oldest = self.map.iter().min_by_key(|(_, e)| e.fetched.or(e.asked)).map(|(&k, _)| k);
    if let Some(k) = oldest {
      self.map.remove(&k);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn paints() -> HashMap<Box<str>, Arc<Paint>> {
    HashMap::new()
  }

  #[test]
  fn parses_paint_and_badge_and_shares_paints() {
    let v = json!({
      "paint": {"id": "P1", "function": "linear-gradient", "angle": 0, "stops": [{"at": 0, "color": -1}]},
      "badge": {"id": "B1", "tooltip": "Subscriber", "host": {"url": "//cdn.7tv.app/badge/B1", "files": [{"name": "1x.webp"}, {"name": "2x.webp"}, {"name": "3x.avif"}]}}
    });
    let mut cache = paints();
    let c = parse(&v, None, &mut cache).unwrap();
    assert_eq!(&*c.badge.as_ref().unwrap().url, "https://cdn.7tv.app/badge/B1/2x.webp");
    assert_eq!(&*c.badge.as_ref().unwrap().title, "Subscriber");
    let again = parse(&v, None, &mut cache).unwrap();
    assert!(Arc::ptr_eq(c.paint.as_ref().unwrap(), again.paint.as_ref().unwrap()));
  }

  #[test]
  fn own_paint_outranks_the_7tv_one_and_stands_alone() {
    let hs = json!({"base": {"type": "solid", "stops": [{"pos": 0, "color": "#ffb000"}]}});
    let seven =
      json!({"paint": {"id": "P1", "function": "linear-gradient", "angle": 0, "stops": [{"at": 0, "color": -1}]}});
    let mut cache = paints();
    let c = parse(&seven, Some(&hs), &mut cache).unwrap();
    assert!(c.paint.unwrap().id.starts_with("hs"));
    // ennortix: heatsync says `paint: null`, /api/paints has the row
    let c = parse(&json!({"paint": null, "badge": null}), Some(&hs), &mut cache).unwrap();
    assert!(c.paint.is_some() && c.badge.is_none());
    assert!(parse(&seven, None, &mut cache).unwrap().paint.unwrap().id.as_ref() == "P1");
  }

  #[test]
  fn hostile_cosmetics_are_dropped() {
    let mut cache = paints();
    assert!(parse(&Value::Null, None, &mut cache).is_none());
    assert!(parse(&json!({"paint": 5, "badge": []}), None, &mut cache).is_none());
    let evil = json!({"badge": {"host": {"url": "//evil.example/b", "files": [{"name": "1x.webp"}]}}});
    assert!(parse(&evil, None, &mut cache).is_none());
    let traversal =
      json!({"badge": {"host": {"url": "//cdn.7tv.app/badge/x", "files": [{"name": "../../1x.webp"}, {"name": 5}]}}});
    assert!(parse(&traversal, None, &mut cache).is_none());
  }

  #[test]
  fn store_asks_once_then_serves_and_refreshes_on_stale() {
    let mut s = Cosmetics::default();
    assert!(s.get(7).is_none());
    assert!(s.get(7).is_none());
    assert_eq!(s.take_wanted(), vec![7]);
    let c = Cosmetic {
      paint: None,
      badge: Some(Badge { url: "https://cdn.7tv.app/b/1x.webp".into(), title: "t".into(), bg: None }),
    };
    assert!(s.put(7, Some(c.clone())));
    assert!(!s.put(7, Some(c)));
    assert!(s.get(7).is_some());
    assert!(s.take_wanted().is_empty());
    s.stale(7);
    assert_eq!(s.take_wanted(), vec![7]);
    // the old badge serves while the refresh is out
    assert!(s.get(7).is_some());
    // an answer for someone never asked about is ignored
    assert!(!s.put(99, None));
    s.on = false;
    assert!(s.get(7).is_none());
  }

  #[test]
  fn store_is_bounded() {
    let mut s = Cosmetics::default();
    for id in 0..(MAX_USERS as u64 * 2) {
      s.get(id);
      s.put(id, None);
    }
    assert!(s.map.len() <= MAX_USERS);
  }
}
