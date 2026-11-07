// per-channel notification rules (local only: the ext has no per-streamer prefs). two things per channel:
// the go-live bell (default follows the desktop notifications switch, on always tells you, off never does)
// and muted pings (a mention there still lands in the mentions tab but makes no sound, flash or popup).
// kept in one small file, `platform:channel \t bell \t mute` per line; a bad line is skipped, never an error
use std::collections::BTreeMap;

use crate::model::message::Platform;

// rules kept; a channel back at its defaults is not kept at all
const MAX: usize = 500;
const NAME_MAX: usize = 64;
const FILE: &str = "pings";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bell {
  #[default]
  Default,
  On,
  Off,
}

impl Bell {
  pub fn label(self) -> &'static str {
    match self {
      Bell::Default => "default",
      Bell::On => "on",
      Bell::Off => "off",
    }
  }

  // default → on → off → default
  pub fn next(self) -> Bell {
    match self {
      Bell::Default => Bell::On,
      Bell::On => Bell::Off,
      Bell::Off => Bell::Default,
    }
  }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Rule {
  bell: Bell,
  mute: bool,
}

#[derive(Default, Debug)]
pub struct Pings {
  by: BTreeMap<String, Rule>,
}

// the one spelling of a channel in the file: key() writes it and parse() only accepts it
fn norm(chan: &str) -> String {
  chan.trim().trim_start_matches('@').to_lowercase()
}

fn key(p: Platform, chan: &str) -> String {
  format!("{}:{}", p.name(), norm(chan))
}

impl Pings {
  pub fn bell(&self, p: Platform, chan: &str) -> Bell {
    self.by.get(&key(p, chan)).map_or(Bell::Default, |r| r.bell)
  }

  pub fn muted(&self, p: Platform, chan: &str) -> bool {
    self.by.get(&key(p, chan)).is_some_and(|r| r.mute)
  }

  fn edit(&mut self, p: Platform, chan: &str, f: impl FnOnce(&mut Rule)) -> bool {
    let k = key(p, chan);
    if k.len() > NAME_MAX + 8 || k.ends_with(':') {
      return false;
    }
    let mut r = self.by.get(&k).copied().unwrap_or_default();
    f(&mut r);
    if r == Rule::default() {
      self.by.remove(&k);
    } else if self.by.len() < MAX || self.by.contains_key(&k) {
      self.by.insert(k, r);
    } else {
      return false;
    }
    true
  }

  // false = refused (too many rules, or no usable name)
  pub fn set_bell(&mut self, p: Platform, chan: &str, bell: Bell) -> bool {
    self.edit(p, chan, |r| r.bell = bell)
  }

  pub fn set_mute(&mut self, p: Platform, chan: &str, mute: bool) -> bool {
    self.edit(p, chan, |r| r.mute = mute)
  }

  pub fn render(&self) -> String {
    let mut s = String::new();
    for (k, r) in &self.by {
      s.push_str(&format!("{k}\t{}\t{}\n", r.bell.label(), u8::from(r.mute)));
    }
    s
  }

  pub fn parse(s: &str) -> Self {
    let mut by = BTreeMap::new();
    for line in s.lines() {
      let mut f = line.split('\t');
      let (Some(k), Some(bell), Some(mute)) = (f.next(), f.next(), f.next()) else { continue };
      let Some((plat, chan)) = k.split_once(':') else { continue };
      let ok = matches!(plat, "twitch" | "kick" | "youtube")
        && !chan.is_empty()
        && chan.len() <= NAME_MAX
        && norm(chan) == chan
        && chan.chars().all(|c| !c.is_control() && !c.is_whitespace());
      let bell = match bell {
        "on" => Bell::On,
        "off" => Bell::Off,
        _ => Bell::Default,
      };
      let r = Rule { bell, mute: mute == "1" };
      if ok && r != Rule::default() && by.len() < MAX {
        by.insert(k.to_owned(), r);
      }
    }
    Pings { by }
  }

  pub fn load() -> Self {
    crate::store::read_capped(FILE, 1 << 20).map(|s| Self::parse(&s)).unwrap_or_default()
  }

  pub fn save(&self) {
    if self.by.is_empty() {
      return crate::store::remove(FILE);
    }
    // a failed save is said once by the store
    let _ = crate::store::write(FILE, &self.render());
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_match_any_case_and_a_youtube_at() {
    let mut p = Pings::default();
    assert!(p.set_mute(Platform::Youtube, "@Tim", true));
    assert!(p.muted(Platform::Youtube, "tim") && !p.muted(Platform::Twitch, "tim"));
  }

  #[test]
  fn a_unicode_name_survives_a_save_and_reload() {
    let mut p = Pings::default();
    p.set_mute(Platform::Youtube, "@Émile", true);
    p.set_bell(Platform::Kick, "ÅSA", Bell::On);
    let back = Pings::parse(&p.render());
    assert!(back.muted(Platform::Youtube, "@émile") && back.muted(Platform::Youtube, "ÉMILE"));
    assert_eq!(back.bell(Platform::Kick, "åsa"), Bell::On);
  }

  #[test]
  fn back_at_defaults_is_forgotten() {
    let mut p = Pings::default();
    p.set_bell(Platform::Twitch, "a", Bell::On);
    p.set_bell(Platform::Twitch, "a", Bell::Default);
    assert_eq!(p.render(), "");
  }

  #[test]
  fn round_trips_and_a_hostile_file_is_skipped() {
    let mut p = Pings::default();
    p.set_bell(Platform::Twitch, "a", Bell::Off);
    p.set_mute(Platform::Kick, "b", true);
    let back = Pings::parse(&p.render());
    assert_eq!((back.bell(Platform::Twitch, "a"), back.muted(Platform::Kick, "b")), (Bell::Off, true));
    let junk = format!(
      "nope\nirc:x\ton\t1\ntwitch:\ton\t1\ntwitch:{}\ton\t1\ntwitch:Up\ton\t1\ntwitch:a\x07b\ton\t1\n",
      "x".repeat(99)
    );
    assert_eq!(Pings::parse(&junk).render(), "");
  }

  #[test]
  fn the_file_is_bounded() {
    let mut p = Pings::default();
    for i in 0..MAX {
      assert!(p.set_mute(Platform::Twitch, &format!("c{i}"), true));
    }
    assert!(!p.set_mute(Platform::Twitch, "one-more", true));
    assert!(p.set_bell(Platform::Twitch, "c1", Bell::On), "an existing one still edits");
    let many: String = (0..MAX + 50).map(|i| format!("twitch:d{i}\ton\t0\n")).collect();
    assert_eq!(Pings::parse(&many).by.len(), MAX);
  }

  #[test]
  fn bell_cycles() {
    assert_eq!(Bell::Default.next().next().next(), Bell::Default);
  }
}
