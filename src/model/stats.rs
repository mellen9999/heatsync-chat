// a channel tab's stream stats (ext stream-stats.js): lines, chatters, mentions and the busiest second
// counted from when this window first saw the tab's chat (reset when the channel goes live), with the
// top chatters and emotes. read by the summary screen (/summary). bounded: past 5000 chatters the
// smallest are dropped (the count says "at least"), past 2000 emotes the rarest
use std::collections::HashMap;

pub const TOP: usize = 5;
const MAX_CHATTERS: usize = 5000;
const KEEP_CHATTERS: usize = 1000;
const MAX_EMOTES: usize = 2000;
const KEEP_EMOTES: usize = 500;
// the words of a line looked at for emotes
pub const MAX_WORDS: usize = 50;
pub const MAX_WORD: usize = 30;

#[derive(Debug, Default)]
pub struct Stats {
  pub msgs: u64,
  pub mentions: u64,
  pub since: i64,
  pub peak: u32,
  sec: i64,
  in_sec: u32,
  chatters: HashMap<Box<str>, u32>,
  // chatters seen before a trim: the real count is at least this
  floor: usize,
  emotes: HashMap<Box<str>, u32>,
}

fn trim(m: &mut HashMap<Box<str>, u32>, keep: usize) {
  let mut v: Vec<(Box<str>, u32)> = m.drain().collect();
  v.sort_unstable_by_key(|a| std::cmp::Reverse(a.1));
  v.truncate(keep);
  m.extend(v);
}

fn top(m: &HashMap<Box<str>, u32>) -> Vec<(&str, u32)> {
  let mut v: Vec<(&str, u32)> = m.iter().map(|(k, n)| (&**k, *n)).collect();
  // ties by name, so the list holds still between redraws
  v.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
  v.truncate(TOP);
  v
}

impl Stats {
  pub fn new(now_ms: i64) -> Self {
    Stats { since: now_ms, ..Stats::default() }
  }

  // one line: who said it (display name), whether it pinged you, and the emotes in it
  pub fn count<'a>(&mut self, now_ms: i64, who: &str, mention: bool, emotes: impl Iterator<Item = &'a str>) {
    self.msgs += 1;
    self.mentions += u64::from(mention);
    let sec = now_ms.div_euclid(1000);
    if sec != self.sec {
      (self.sec, self.in_sec) = (sec, 0);
    }
    self.in_sec += 1;
    self.peak = self.peak.max(self.in_sec);
    if !who.is_empty() {
      match self.chatters.get_mut(who) {
        Some(n) => *n += 1,
        None => {
          self.chatters.insert(who.into(), 1);
        }
      }
      if self.chatters.len() > MAX_CHATTERS {
        self.floor = self.floor.max(self.chatters.len());
        trim(&mut self.chatters, KEEP_CHATTERS);
      }
    }
    for e in emotes {
      match self.emotes.get_mut(e) {
        Some(n) => *n += 1,
        None => {
          self.emotes.insert(e.into(), 1);
        }
      }
    }
    if self.emotes.len() > MAX_EMOTES {
      trim(&mut self.emotes, KEEP_EMOTES);
    }
  }

  // `1,234` or `5,000+` once trimmed
  pub fn chatters(&self) -> String {
    let n = self.chatters.len();
    if self.floor > n { format!("{}+", group(self.floor as u64)) } else { group(n as u64) }
  }

  pub fn top_chatters(&self) -> Vec<(&str, u32)> {
    top(&self.chatters)
  }

  pub fn top_emotes(&self) -> Vec<(&str, u32)> {
    top(&self.emotes)
  }
}

// 1234567 → "1,234,567"
pub fn group(n: u64) -> String {
  let s = n.to_string();
  let mut out = String::with_capacity(s.len() + s.len() / 3);
  for (i, c) in s.chars().enumerate() {
    if i > 0 && (s.len() - i).is_multiple_of(3) {
      out.push(',');
    }
    out.push(c);
  }
  out
}

// how long since: `47m`, `2h 5m`
pub fn span(ms: i64) -> String {
  let m = ms.max(0) / 60_000;
  if m >= 60 { format!("{}h {}m", m / 60, m % 60) } else { format!("{m}m") }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn counts_like_the_ext() {
    let mut s = Stats::new(0);
    s.count(1000, "bob", false, ["Kappa", "Kappa"].into_iter());
    s.count(1500, "bob", true, std::iter::empty());
    s.count(1900, "amy", false, ["LUL"].into_iter());
    s.count(2100, "amy", false, std::iter::empty());
    assert_eq!((s.msgs, s.mentions, s.peak), (4, 1, 3));
    assert_eq!(s.chatters(), "2");
    assert_eq!(s.top_chatters(), [("amy", 2), ("bob", 2)]);
    assert_eq!(s.top_emotes(), [("Kappa", 2), ("LUL", 1)]);
  }

  #[test]
  fn it_stays_bounded() {
    let mut s = Stats::new(0);
    for i in 0..=MAX_CHATTERS {
      s.count(0, &format!("u{i}"), false, std::iter::empty());
    }
    assert_eq!(s.chatters.len(), KEEP_CHATTERS);
    assert_eq!(s.chatters(), "5,001+");
    let words: Vec<String> = (0..=MAX_EMOTES).map(|i| format!("e{i}")).collect();
    s.count(0, "x", false, words.iter().map(String::as_str));
    assert_eq!(s.emotes.len(), KEEP_EMOTES);
  }

  #[test]
  fn numbers_read_plainly() {
    assert_eq!(
      (group(0), group(999), group(1000), group(1234567)),
      ("0".into(), "999".into(), "1,000".into(), "1,234,567".into())
    );
    assert_eq!((span(47 * 60_000), span(125 * 60_000), span(-5)), ("47m".into(), "2h 5m".into(), "0m".into()));
  }
}
