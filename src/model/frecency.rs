// how much you use each emote (ext hs-mc-emote-frecency): a count that halves every 7 days, so tab
// puts the ones you reach for first. 200 names, the faintest shed; seeded once from the picker's recents
use std::collections::HashMap;

use crate::store;

const FILE: &str = "emote-frecency";
const CAP: usize = 200;
const HALF_LIFE_MS: f64 = 7.0 * 86_400_000.0;

#[derive(Default)]
pub struct Frecency(HashMap<Box<str>, (f64, i64)>);

impl Frecency {
  pub fn load(recents: &[Box<str>], now: i64) -> Self {
    let Some(text) = store::read(FILE) else {
      // first run: what you picked lately counts once each
      return Self(recents.iter().take(CAP).map(|n| (n.clone(), (1.0, now))).collect());
    };
    let mut f = Self::default();
    for l in text.lines() {
      let mut p = l.split('\t');
      let (Some(name), Some(n), Some(t)) = (p.next(), p.next(), p.next()) else { continue };
      let (Ok(n), Ok(t)) = (n.parse::<f64>(), t.parse::<i64>()) else { continue };
      if crate::heatsync::valid_name(name) && n.is_finite() && n > 0.0 && f.0.len() < CAP {
        f.0.insert(name.into(), (n, t));
      }
    }
    f
  }

  pub fn score(&self, name: &str, now: i64) -> f64 {
    self.0.get(name).map_or(0.0, |&(n, t)| n * (-((now - t).max(0) as f64) / HALF_LIFE_MS).exp2())
  }

  fn bump_at(&mut self, name: &str, now: i64) {
    let n = self.score(name, now) + 1.0;
    self.0.insert(name.into(), (n, now));
    if self.0.len() > CAP {
      let faint = (self.0.iter())
        .filter(|(k, _)| ***k != *name)
        .map(|(k, _)| (k.clone(), self.score(k, now)))
        .min_by(|a, b| a.1.total_cmp(&b.1));
      if let Some((k, _)) = faint {
        self.0.remove(&k);
      }
    }
  }

  pub fn record(&mut self, name: &str, now: i64) {
    if !crate::heatsync::valid_name(name) {
      return;
    }
    self.bump_at(name, now);
    let s: String = self.0.iter().map(|(k, (n, t))| format!("{k}\t{n}\t{t}\n")).collect();
    if let Err(e) = store::write(FILE, &s) {
      eprintln!("save frecency: {e}");
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn halves_weekly_and_sheds_the_faintest() {
    let mut f = Frecency::default();
    f.bump_at("old", 0);
    f.bump_at("old", 0);
    assert_eq!(f.score("old", 0), 2.0);
    let week = HALF_LIFE_MS as i64;
    assert!((f.score("old", week) - 1.0).abs() < 1e-9);
    assert_eq!(f.score("never", week), 0.0);
    for i in 0..CAP {
      f.bump_at(&format!("e{i}"), week);
    }
    assert_eq!(f.0.len(), CAP);
    // "old" (1.0 at a week) ties the fresh ones; a full shed keeps the cap either way
    f.bump_at("new", week);
    assert_eq!(f.0.len(), CAP);
    assert!(f.score("new", week) > 0.0);
  }
}
