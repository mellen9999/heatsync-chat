// which platforms a merged tab hides (ext main.js platformFilters, device-local: never synced). the tab is
// named by its saved feeds line (`forsen kick:xqc`), its choice is a few bits, and the file is one small
// `line \t bits` per tab. a bad line, a stranger's tab or a choice that would hide every feed is dropped on
// read: the worst a hostile file can do is show you everything
use std::collections::BTreeMap;

use crate::model::chan::Spec;
use crate::model::message::Platform;

pub const FILE: &str = "platforms";
// tabs kept, and the longest feeds line one may be named by
const MAX: usize = 200;
const LINE_MAX: usize = 160;
// a file past this is not one of ours
pub const FILE_MAX: u64 = 64 * 1024;

pub const fn bit(p: Platform) -> u8 {
  match p {
    Platform::Twitch => 1,
    Platform::Kick => 2,
    Platform::Youtube => 4,
  }
}

// the feeds a tab carries, as bits
pub fn linked(sources: &[(Platform, Box<str>)]) -> u8 {
  sources.iter().fold(0, |m, s| m | bit(s.0))
}

#[derive(Default, Debug)]
pub struct PlatFilter {
  by: BTreeMap<String, u8>,
}

impl PlatFilter {
  // the bits hidden on a tab: only its own feeds count, only a merged tab has any, and never all of them
  pub fn hidden(&self, line: &str, linked: u8) -> u8 {
    let m = self.by.get(line).copied().unwrap_or(0) & linked;
    if linked.count_ones() < 2 || m == linked { 0 } else { m }
  }

  // flip one feed on a tab. false = refused: it is not one of the tab's feeds, would hide the last one
  // showing, or the list is full
  pub fn flip(&mut self, line: &str, p: Platform, linked: u8) -> bool {
    let cur = self.hidden(line, linked);
    let now = cur ^ bit(p);
    if linked & bit(p) == 0 || linked.count_ones() < 2 || now == linked || line.len() > LINE_MAX {
      return false;
    }
    if now == 0 {
      self.by.remove(line);
    } else if self.by.len() < MAX || self.by.contains_key(line) {
      self.by.insert(line.to_owned(), now);
    } else {
      return false;
    }
    true
  }

  // a tab's feeds changed (edit): what it hid carries over to the feeds it still has
  pub fn rename(&mut self, from: &str, to: &str, linked: u8) {
    if from == to {
      return;
    }
    let Some(m) = self.by.remove(from) else { return };
    let m = m & linked;
    if m != 0 && m != linked && to.len() <= LINE_MAX && (self.by.len() < MAX || self.by.contains_key(to)) {
      self.by.insert(to.to_owned(), m);
    }
  }

  // forget every tab not in `lines` (boot, once the tabs are known): the file stays bounded by the tabs
  pub fn keep_only<'a>(&mut self, lines: impl IntoIterator<Item = &'a str>) -> bool {
    let keep: std::collections::HashSet<&str> = lines.into_iter().collect();
    let before = self.by.len();
    self.by.retain(|k, _| keep.contains(k.as_str()));
    self.by.len() != before
  }

  pub fn render(&self) -> String {
    self.by.iter().map(|(k, m)| format!("{k}\t{m}\n")).collect()
  }

  pub fn parse(s: &str) -> Self {
    let mut by = BTreeMap::new();
    for row in s.lines() {
      let Some((line, bits)) = row.split_once('\t') else { continue };
      let Ok(m) = bits.trim().parse::<u8>() else { continue };
      let merged = Spec::from_line(line).is_ok_and(|sp| sp.line() == line && line.split(' ').count() > 1);
      if merged && line.len() <= LINE_MAX && (1..=7).contains(&m) && by.len() < MAX {
        by.insert(line.to_owned(), m);
      }
    }
    PlatFilter { by }
  }

  pub fn load() -> Self {
    crate::store::read_capped(FILE, FILE_MAX).map(|s| Self::parse(&s)).unwrap_or_default()
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

  const TK: u8 = 3;
  const TKY: u8 = 7;

  #[test]
  fn a_flip_hides_one_feed_and_flips_back() {
    let mut f = PlatFilter::default();
    assert!(f.flip("forsen kick:xqc", Platform::Kick, TK));
    assert_eq!(f.hidden("forsen kick:xqc", TK), bit(Platform::Kick));
    assert!(f.flip("forsen kick:xqc", Platform::Kick, TK));
    assert_eq!(f.hidden("forsen kick:xqc", TK), 0);
    assert!(f.render().is_empty(), "back at the default keeps nothing");
  }

  #[test]
  fn the_last_feed_showing_cannot_be_hidden() {
    let mut f = PlatFilter::default();
    assert!(f.flip("a kick:b", Platform::Kick, TK));
    assert!(!f.flip("a kick:b", Platform::Twitch, TK));
    assert_eq!(f.hidden("a kick:b", TK), 2);
    // two of three hidden is fine, the third is not
    assert!(f.flip("a kick:b yt:@c", Platform::Kick, TKY));
    assert!(f.flip("a kick:b yt:@c", Platform::Youtube, TKY));
    assert!(!f.flip("a kick:b yt:@c", Platform::Twitch, TKY));
  }

  #[test]
  fn a_single_feed_tab_and_a_feed_it_lacks_refuse() {
    let mut f = PlatFilter::default();
    assert!(!f.flip("forsen", Platform::Twitch, 1));
    assert!(!f.flip("a kick:b", Platform::Youtube, TK));
  }

  #[test]
  fn a_hostile_file_shows_everything_it_cannot_trust() {
    let hostile = "a kick:b\t3\nforsen\t1\nnot a tab\t2\na kick:b\tx\na kick:b yt:@c\t300\n\t1\nbad:thing kick:b\t1\na kick:b\t99999999999\n";
    let f = PlatFilter::parse(hostile);
    assert_eq!(f.hidden("a kick:b", TK), 0, "every feed hidden reads as none");
    assert_eq!(f.hidden("forsen", 1), 0, "a lone feed has no filter");
    assert_eq!(f.hidden("a kick:b yt:@c", TKY), 0);
  }

  #[test]
  fn it_survives_a_round_trip_and_stays_bounded() {
    let mut f = PlatFilter::default();
    assert!(f.flip("a kick:b", Platform::Kick, TK));
    assert_eq!(PlatFilter::parse(&f.render()).hidden("a kick:b", TK), 2);
    let mut big = String::new();
    for i in 0..MAX + 50 {
      big.push_str(&format!("a{i} kick:b\t2\n"));
    }
    assert_eq!(PlatFilter::parse(&big).by.len(), MAX);
  }

  #[test]
  fn an_edit_carries_the_choice_to_the_feeds_that_stay() {
    let mut f = PlatFilter::default();
    assert!(f.flip("a kick:b yt:@c", Platform::Kick, TKY));
    f.rename("a kick:b yt:@c", "a kick:z", TK);
    assert_eq!(f.hidden("a kick:z", TK), 2);
    f.rename("a kick:z", "a", 1);
    assert_eq!(f.hidden("a", 1), 0);
    assert!(f.render().is_empty());
  }

  #[test]
  fn boot_forgets_tabs_that_are_gone() {
    let mut f = PlatFilter::default();
    f.flip("a kick:b", Platform::Kick, TK);
    f.flip("c kick:d", Platform::Kick, TK);
    assert!(f.keep_only(["a kick:b"]));
    assert_eq!(f.hidden("c kick:d", TK), 0);
    assert_eq!(f.hidden("a kick:b", TK), 2);
  }
}
