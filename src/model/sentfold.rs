// one send to several feeds comes back once per feed, and should read as one line (ext isSentEcho). a send is
// remembered for ten seconds; the first echo of it shows, and every other feed's copy of that same send is
// dropped. an echo is claimed by the oldest send that hasn't taken one from its platform yet, so two sends of
// the same words stay two lines. what arrives is counted, not what was expected: an echo nobody predicted still
// folds instead of showing a second time
use std::collections::VecDeque;

use super::message::Platform;

// the ext's SENT_DEDUP_WINDOW
pub const WINDOW_MS: i64 = 10_000;
const MAX: usize = 16;

struct Entry {
  text: Box<str>,
  at: i64,
  // twitch puts `@name ` in front of a reply's echo
  reply: bool,
  seen: Vec<Platform>,
  shown: bool,
}

impl Entry {
  fn matches(&self, body: &str) -> bool {
    let body = body.trim();
    if *self.text == *body {
      return true;
    }
    // the youtube copy of a reply opens with its own `@name`, twitch's echo with the parent's: one word off
    self.reply
      && body
        .split_once(char::is_whitespace)
        .is_some_and(|(at, rest)| at.starts_with('@') && *self.text == *rest.trim_start())
  }
}

#[derive(Default)]
pub struct Sent(VecDeque<Entry>);

impl Sent {
  // a send that went to several feeds (one feed has nothing to fold). `text` as the echo will read: a /me
  // without its `/me `
  pub fn track(&mut self, now: i64, text: &str, reply: bool) {
    self.0.retain(|e| now - e.at < WINDOW_MS);
    if self.0.len() == MAX {
      self.0.pop_front();
    }
    self.0.push_back(Entry { text: text.trim().into(), at: now, reply, seen: Vec::new(), shown: false });
  }

  // one of your lines arrived from `p`: true = a copy of a send already on screen, don't show it
  pub fn folds(&mut self, now: i64, p: Platform, body: &str) -> bool {
    self.0.retain(|e| now - e.at < WINDOW_MS);
    for e in self.0.iter_mut() {
      // a feed that already echoed this send means the line belongs to a later send of the same words
      if !e.matches(body) || e.seen.contains(&p) {
        continue;
      }
      e.seen.push(p);
      return std::mem::replace(&mut e.shown, true);
    }
    false
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use Platform::*;

  #[test]
  fn one_send_to_all_feeds_shows_once() {
    let mut s = Sent::default();
    s.track(0, "gg wp", false);
    assert!(!s.folds(100, Twitch, "gg wp"), "the first echo shows");
    assert!(s.folds(300, Kick, "gg wp"));
    assert!(s.folds(900, Youtube, "gg wp"));
  }

  #[test]
  fn a_partial_return_still_shows_one() {
    let mut s = Sent::default();
    s.track(0, "gg", false);
    // youtube never comes back (it failed); twitch is first and kick folds
    assert!(!s.folds(50, Twitch, "gg"));
    assert!(s.folds(80, Kick, "gg"));
    // only one echo at all: it shows
    let mut s = Sent::default();
    s.track(0, "hi", false);
    assert!(!s.folds(10, Kick, "hi"));
  }

  #[test]
  fn the_same_words_sent_twice_are_two_lines() {
    let mut s = Sent::default();
    s.track(0, "lol", false);
    s.track(1000, "lol", false);
    assert!(!s.folds(1100, Twitch, "lol"));
    assert!(s.folds(1200, Kick, "lol"));
    // twitch again: the second send's own echo
    assert!(!s.folds(1300, Twitch, "lol"));
    assert!(s.folds(1400, Kick, "lol"));
  }

  #[test]
  fn timing_is_ten_seconds_then_it_is_a_new_line() {
    let mut s = Sent::default();
    s.track(0, "x", false);
    assert!(!s.folds(WINDOW_MS - 1, Twitch, "x"));
    assert!(!s.folds(WINDOW_MS, Kick, "x"), "ten seconds on it is someone repeating you");
    // nothing sent, nothing folded
    assert!(!Sent::default().folds(0, Twitch, "x"));
  }

  #[test]
  fn other_words_and_a_reply_echo() {
    let mut s = Sent::default();
    s.track(0, "gg", false);
    assert!(!s.folds(10, Twitch, "gg ez"), "other words");
    assert!(!s.folds(20, Twitch, "gg"));
    assert!(!s.folds(30, Kick, "@bob gg"), "a stranger's @ is not a reply echo of a plain send");
    let mut s = Sent::default();
    s.track(0, "gg", true);
    assert!(!s.folds(10, Twitch, "@bob gg"));
    assert!(s.folds(20, Youtube, "@bob  gg"));
    assert!(s.folds(30, Kick, "gg"));
  }

  #[test]
  fn bounded() {
    let mut s = Sent::default();
    for i in 0..100 {
      s.track(i, "a", false);
    }
    assert_eq!(s.0.len(), MAX);
  }
}
