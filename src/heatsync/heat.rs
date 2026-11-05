// each chatter's heat on heatsync (what their posts earned): a thin bar before the name on twitch lines.
// asked by login in batches, kept a couple of minutes, and updated live by the server's heat push.
// everything the server says is hostile: counts, names and numbers are bounded here
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::ui::theme;

// the ext's HEAT_CACHE_MAX; the contract's ttl for a score
const MAX_LOGINS: usize = 1000;
const TTL: Duration = Duration::from_secs(120);
// a request that never answered may be asked again after this
const INFLIGHT: Duration = Duration::from_secs(30);
// the most updates one push frame is read for
pub const MAX_PUSH: usize = 500;
// a score is a whole number far below this; anything past it is a lie
const MAX_HEAT: f64 = 1.0e9;

// the bar a score draws: a glyph and its ink. none under 5 (the ext's applyHeatBorderToElement). the
// site's ladder: white while cold, yellow warming, red from 1000; thicker as it climbs
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bar {
  pub glyph: &'static str,
  pub ink: u32,
}

pub fn bar(heat: u32) -> Option<Bar> {
  let (glyph, ink) = match heat {
    0..5 => return None,
    5..20 => ("\u{258e}", theme::WHITE),
    20..50 => ("\u{258d}", theme::WHITE),
    50..100 => ("\u{258d}", theme::YELLOW),
    100..1000 => ("\u{258c}", theme::YELLOW),
    _ => ("\u{258a}", theme::RED),
  };
  Some(Bar { glyph, ink })
}

// a twitch login as heatsync takes it: lowercase letters, digits, underscore, 1..=25
pub fn valid_login(s: &str) -> bool {
  (1..=25).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

// a score off the wire: a finite, non-negative number, whole
fn score(v: Option<&Value>) -> u32 {
  v.and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0).map_or(0, |n| n.min(MAX_HEAT) as u32)
}

struct Entry {
  heat: u32,
  fetched: Option<Instant>,
  asked: Option<Instant>,
}

#[derive(Default)]
pub struct Heats {
  map: HashMap<Box<str>, Entry>,
  wanted: Vec<Box<str>>,
  // the synthetic flood's made-up names are never asked about
  off: bool,
}

impl Heats {
  // `login`'s score, 0 while unknown. queues a fetch when it is unknown or stale (the old score keeps
  // serving meanwhile). no allocation for a login already held
  pub fn get(&mut self, login: &str) -> u32 {
    if self.off || !valid_login(login) {
      return 0;
    }
    let now = Instant::now();
    if !self.map.contains_key(login) {
      if self.map.len() >= MAX_LOGINS {
        self.evict();
      }
      self.map.insert(login.into(), Entry { heat: 0, fetched: None, asked: None });
    }
    let Some(e) = self.map.get_mut(login) else { return 0 };
    if e.fetched.is_none_or(|f| now - f > TTL) && e.asked.is_none_or(|a| now - a > INFLIGHT) {
      e.asked = Some(now);
      self.wanted.push(login.into());
    }
    e.heat
  }

  // never ask (the synthetic flood)
  pub fn mute(&mut self) {
    self.off = true;
  }

  pub fn take_wanted(&mut self) -> Vec<Box<str>> {
    std::mem::take(&mut self.wanted)
  }

  // an answer for `login`. true when the bar they wear changed
  pub fn put(&mut self, login: &str, heat: u32) -> bool {
    let Some(e) = self.map.get_mut(login) else { return false };
    let changed = bar(e.heat) != bar(heat);
    e.heat = heat;
    e.fetched = Some(Instant::now());
    e.asked = None;
    changed
  }

  fn evict(&mut self) {
    let oldest = self.map.iter().min_by_key(|(_, e)| e.fetched.or(e.asked)).map(|(k, _)| k.clone());
    if let Some(k) = oldest {
      self.map.remove(&k);
    }
  }
}

// the server's push (`user:heat_batch_update {updates[{username,userHeat,op,re}]}`): new scores by login.
// at most `MAX_PUSH` are read. `Heats::put` ignores a login we do not hold, so a push never grows the cache
pub fn parse_push(v: &Value) -> Vec<(Box<str>, u32)> {
  let ups = v.get("updates").and_then(Value::as_array).into_iter().flatten().take(MAX_PUSH);
  ups
    .filter_map(|u| {
      let login = u.get("username").and_then(Value::as_str).map(str::to_ascii_lowercase)?;
      valid_login(&login).then(|| (login.into(), score(u.get("userHeat"))))
    })
    .collect()
}

// `POST /api/users/heat` answer: `users{login:{heat,op,re}}`, read back per login asked (a login the
// server left out has none: 0)
pub fn parse(v: &Value, asked: &[Box<str>]) -> Option<Vec<(Box<str>, u32)>> {
  let users = v.get("users")?.as_object()?;
  Some(asked.iter().map(|l| (l.clone(), score(users.get(&**l).and_then(|u| u.get("heat"))))).collect())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn tiers_follow_the_contract() {
    let ladder = [
      (0, None),
      (4, None),
      (5, Some(("\u{258e}", theme::WHITE))),
      (19, Some(("\u{258e}", theme::WHITE))),
      (20, Some(("\u{258d}", theme::WHITE))),
      (49, Some(("\u{258d}", theme::WHITE))),
      (50, Some(("\u{258d}", theme::YELLOW))),
      (99, Some(("\u{258d}", theme::YELLOW))),
      (100, Some(("\u{258c}", theme::YELLOW))),
      (200, Some(("\u{258c}", theme::YELLOW))),
      (500, Some(("\u{258c}", theme::YELLOW))),
      (999, Some(("\u{258c}", theme::YELLOW))),
      (1000, Some(("\u{258a}", theme::RED))),
      (5000, Some(("\u{258a}", theme::RED))),
      (u32::MAX, Some(("\u{258a}", theme::RED))),
    ];
    for (heat, want) in ladder {
      assert_eq!(bar(heat), want.map(|(glyph, ink)| Bar { glyph, ink }), "{heat}");
    }
  }

  #[test]
  fn logins_are_heatsyncs_shape() {
    for ok in ["a", "ennortix", "user_01", &"x".repeat(25)] {
      assert!(valid_login(ok), "{ok}");
    }
    for bad in ["", "Ennortix", "a b", "a-b", "é", &"x".repeat(26), "a\n"] {
      assert!(!valid_login(bad), "{bad:?}");
    }
  }

  #[test]
  fn an_unknown_login_is_asked_once_then_served_stale() {
    let mut h = Heats::default();
    assert_eq!(h.get("bob"), 0);
    assert_eq!(h.take_wanted(), ["bob".into()]);
    assert_eq!(h.get("bob"), 0);
    assert!(h.take_wanted().is_empty(), "in flight: not asked twice");
    assert!(h.put("bob", 70), "a new bar");
    assert!(!h.put("bob", 90), "same bar, nothing to redraw");
    assert_eq!(h.get("bob"), 90);
    assert!(h.take_wanted().is_empty(), "fresh");
    h.map.get_mut("bob").unwrap().fetched = Some(Instant::now() - TTL - Duration::from_secs(1));
    assert_eq!(h.get("bob"), 90, "stale data keeps serving");
    assert_eq!(h.take_wanted(), ["bob".into()]);
    assert_eq!(h.get("Not A Login"), 0);
    assert!(h.take_wanted().is_empty() && !h.map.contains_key("Not A Login"));
  }

  #[test]
  fn the_cache_is_bounded() {
    let mut h = Heats::default();
    for i in 0..MAX_LOGINS + 200 {
      h.get(&format!("u{i}"));
    }
    assert_eq!(h.map.len(), MAX_LOGINS);
    assert!(h.take_wanted().len() <= MAX_LOGINS + 200);
  }

  #[test]
  fn the_push_is_read_by_login_and_updates_only_logins_we_hold() {
    let mut h = Heats::default();
    h.get("bob");
    h.put("bob", 1);
    h.get("amy");
    h.put("amy", 300);
    let v = json!({"updates": [
      {"username": "BOB", "userHeat": 1200}, {"username": "amy", "userHeat": 310},
      {"username": "stranger", "userHeat": 9999}, {"username": "bad name", "userHeat": 5},
      {"username": 5, "userHeat": 1}, {"userHeat": 3}, "x", null
    ]});
    let got = parse_push(&v);
    assert_eq!(got.len(), 3, "bad names and shapes are dropped");
    let changed: Vec<_> = got.iter().filter(|(l, n)| h.put(l, *n)).map(|(l, _)| l.to_string()).collect();
    assert_eq!(changed, ["bob"], "amy stays on the same bar");
    assert_eq!((h.get("bob"), h.get("amy")), (1200, 310));
    assert!(!h.map.contains_key("stranger"), "a push never grows the cache");
    assert!(parse_push(&json!({"updates": 4})).is_empty() && parse_push(&json!(null)).is_empty());
  }

  #[test]
  fn a_push_reads_at_most_five_hundred_updates() {
    let ups: Vec<Value> = (0..MAX_PUSH + 10).map(|i| json!({"username": format!("u{i}"), "userHeat": 6})).collect();
    assert_eq!(parse_push(&json!({"updates": ups})).len(), MAX_PUSH);
  }

  #[test]
  fn hostile_scores_are_clamped() {
    assert_eq!(score(Some(&json!(-3))), 0);
    assert_eq!(score(Some(&json!("9"))), 0);
    assert_eq!(score(Some(&json!(1.0e300))), 1_000_000_000);
    assert_eq!(score(Some(&json!(12.9))), 12);
    assert_eq!(score(None), 0);
  }

  #[test]
  fn the_answer_is_read_per_login_asked() {
    let asked: Vec<Box<str>> = vec!["bob".into(), "amy".into()];
    let v = json!({"users": {"bob": {"heat": 120, "op": 3, "re": 40}, "stranger": {"heat": 5}}});
    assert_eq!(parse(&v, &asked), Some(vec![("bob".into(), 120), ("amy".into(), 0)]));
    for bad in [json!(null), json!({"users": []}), json!({"users": 3}), json!({})] {
      assert_eq!(parse(&bad, &asked), None);
    }
  }
}
