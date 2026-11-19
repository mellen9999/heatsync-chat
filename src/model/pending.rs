// a send nobody echoed back (ext input.js pendingSends). every line sent is remembered per leg under its one send
// id; a leg is confirmed when its own echo comes back (or heatsync files it under that send id). past the timeout
// an unconfirmed send is told once, and a retry resends only the legs that never confirmed, under the same id so a
// late one still folds into the row that landed.
// kick and youtube go through heatsync's POST, with no idempotency key: a leg whose POST went through or may
// have is never posted again. only a leg that provably never went out, or a twitch leg (irc takes the same line
// again), is resent. bounded in count and age
use std::collections::VecDeque;

use super::hsorigin::room;
use super::message::Platform;
use crate::net::Reply;

// the ext's PENDING_ECHO_TIMEOUT_MS
pub const TIMEOUT_MS: i64 = 20_000;
// a told send stays retryable this long, then it is forgotten
const KEEP_MS: i64 = 10 * 60 * 1000;
const MAX: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
  // sent, nothing known yet
  Waiting,
  // heatsync took the POST (or may have): in chat or soon, never posted again
  Landed,
  // the POST provably never went out
  Refused,
  // its echo came back
  Done,
}

#[derive(Clone)]
pub struct Leg {
  pub platform: Platform,
  pub chan: Box<str>,
  // what was put on the wire for this feed
  pub text: String,
  pub reply: Option<Reply>,
  // the words as the feed's echo reads them: no `/me `, kick's emote tokens as their names
  want: Box<str>,
  fate: Fate,
}

impl Leg {
  pub fn new(platform: Platform, chan: Box<str>, text: String, reply: Option<Reply>) -> Self {
    let bare = text.strip_prefix("/me ").map_or(text.as_str(), str::trim_start);
    let want = crate::net::relay::unkick(bare).trim().into();
    Leg { platform, chan, text, reply, want, fate: Fate::Waiting }
  }

  fn here(&self, p: Platform, chan: &str) -> bool {
    self.platform == p && room(&self.chan) == room(chan)
  }

  // safe to post again: a twitch line never confirmed, or a heatsync POST that never went out
  fn resendable(&self) -> bool {
    match self.fate {
      Fate::Refused => true,
      Fate::Waiting => self.platform == Platform::Twitch,
      Fate::Landed | Fate::Done => false,
    }
  }

  fn echoed_by(&self, body: &str) -> bool {
    let body = body.trim();
    *self.want == *body || super::origin::unreply(body) == Some(&*self.want)
  }
}

struct Entry {
  id: Box<str>,
  at: i64,
  legs: Vec<Leg>,
  // the notice for this send was shown; a retry clears it so a second miss speaks again
  told: bool,
}

impl Entry {
  fn open(&self) -> bool {
    self.legs.iter().any(|l| l.fate != Fate::Done)
  }
}

// what a retry does: the legs to post again (none when every unconfirmed one may already be in chat)
pub struct Retry {
  pub id: Box<str>,
  pub legs: Vec<Leg>,
}

// a send past its timeout: the legs still unconfirmed
pub struct Missed {
  pub legs: Vec<(Platform, Box<str>)>,
  // at least one can be resent
  pub retry: bool,
}

#[derive(Default)]
pub struct Pending(VecDeque<Entry>);

impl Pending {
  pub fn track(&mut self, now: i64, id: &str, legs: Vec<Leg>) {
    self.0.retain(|e| now - e.at < KEEP_MS && e.open());
    if self.0.len() == MAX {
      self.0.pop_front();
    }
    self.0.push_back(Entry { id: id.into(), at: now, legs, told: false });
  }

  // one of your lines arrived in (p, chan): the oldest send still waiting on that feed for those words is confirmed
  pub fn echo(&mut self, p: Platform, chan: &str, body: &str) -> bool {
    let hit = self
      .0
      .iter_mut()
      .find_map(|e| e.legs.iter_mut().find(|l| l.fate != Fate::Done && l.here(p, chan) && l.echoed_by(body)));
    hit.map(|l| l.fate = Fate::Done).is_some()
  }

  // heatsync filed a line of (p, chan) under send `id`: that leg is on the platform
  pub fn filed(&mut self, p: Platform, chan: &str, id: &str) {
    for e in self.0.iter_mut().filter(|e| *e.id == *id) {
      e.legs.iter_mut().filter(|l| l.here(p, chan)).for_each(|l| l.fate = Fate::Done);
    }
  }

  // a heatsync job finished: its POST was taken (maybe in chat already) or never went out
  pub fn posted(&mut self, id: &str, p: Platform, chan: &str, landed: bool) {
    let fate = if landed { Fate::Landed } else { Fate::Refused };
    for e in self.0.iter_mut().filter(|e| *e.id == *id) {
      for l in e.legs.iter_mut().filter(|l| l.here(p, chan) && l.fate == Fate::Waiting) {
        l.fate = fate;
      }
    }
  }

  // sends past the timeout that still have an unconfirmed leg, each told once
  pub fn due(&mut self, now: i64) -> Vec<(Box<str>, Missed)> {
    self.0.retain(|e| e.open() && now - e.at < KEEP_MS);
    let mut out = Vec::new();
    for e in self.0.iter_mut().filter(|e| !e.told && now - e.at >= TIMEOUT_MS) {
      e.told = true;
      let open: Vec<&Leg> = e.legs.iter().filter(|l| l.fate != Fate::Done).collect();
      let legs = open.iter().map(|l| (l.platform, l.chan.clone())).collect();
      out.push((e.id.clone(), Missed { legs, retry: open.iter().any(|l| l.resendable()) }));
    }
    out
  }

  // when the next send runs out of time
  pub fn next_due(&self) -> Option<i64> {
    self.0.iter().filter(|e| !e.told).map(|e| e.at + TIMEOUT_MS).min()
  }

  // the latest told send with something unconfirmed: its resendable legs go out again (the timer starts over), the
  // rest stay as they are
  pub fn retry(&mut self, now: i64) -> Option<Retry> {
    let e = self.0.iter_mut().rev().find(|e| e.told && e.open())?;
    let legs: Vec<Leg> = e
      .legs
      .iter_mut()
      .filter(|l| l.resendable())
      .map(|l| {
        l.fate = Fate::Waiting;
        l.clone()
      })
      .collect();
    if !legs.is_empty() {
      e.at = now;
      e.told = false;
    }
    Some(Retry { id: e.id.clone(), legs })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use Platform::*;

  fn leg(p: Platform, chan: &str, text: &str) -> Leg {
    Leg::new(p, chan.into(), text.into(), None)
  }

  fn two() -> Pending {
    let mut p = Pending::default();
    p.track(0, "s1", vec![leg(Twitch, "forsen", "gg wp"), leg(Kick, "forsen", "gg wp")]);
    p
  }

  #[test]
  fn an_echo_confirms_its_leg() {
    let mut p = two();
    assert!(p.echo(Twitch, "Forsen", "gg wp"), "case of the room does not matter");
    assert!(!p.echo(Twitch, "forsen", "gg wp"), "a leg confirms once");
    assert!(!p.echo(Kick, "forsen", "gg"), "other words");
    assert!(!p.echo(Youtube, "forsen", "gg wp"), "another feed");
    assert!(p.echo(Kick, "forsen", "gg wp"));
    assert!(p.due(TIMEOUT_MS).is_empty() && p.0.is_empty(), "all confirmed: nothing to tell, nothing kept");
  }

  #[test]
  fn a_reply_echo_and_a_me_line_and_kick_tokens_read_as_sent() {
    let mut p = Pending::default();
    p.track(
      0,
      "s",
      vec![leg(Twitch, "a", "gg"), leg(Twitch, "b", "/me waves"), leg(Kick, "a", "hi [emote:37225:KEKLEO]")],
    );
    assert!(p.echo(Twitch, "a", "@bob gg"), "twitch puts @parent in front of a reply");
    assert!(p.echo(Twitch, "b", "waves"));
    assert!(p.echo(Kick, "a", "hi KEKLEO"));
  }

  #[test]
  fn heatsync_filing_a_leg_under_the_send_confirms_it() {
    let mut p = two();
    p.filed(Kick, "forsen", "other");
    p.filed(Kick, "forsen", "s1");
    assert!(!p.echo(Kick, "forsen", "gg wp"), "already confirmed");
    assert_eq!(p.due(TIMEOUT_MS)[0].1.legs, vec![(Twitch, "forsen".into())]);
  }

  #[test]
  fn past_the_timeout_an_unconfirmed_send_is_told_once() {
    let mut p = two();
    assert_eq!(p.next_due(), Some(TIMEOUT_MS));
    assert!(p.due(TIMEOUT_MS - 1).is_empty());
    p.echo(Twitch, "forsen", "gg wp");
    let due = p.due(TIMEOUT_MS);
    assert_eq!(due.len(), 1);
    assert_eq!((&*due[0].0, due[0].1.legs.clone()), ("s1", vec![(Kick, "forsen".into())]));
    assert!(p.due(TIMEOUT_MS + 1).is_empty(), "told once");
    assert_eq!(p.next_due(), None);
  }

  #[test]
  fn retry_resends_only_the_missing_leg_under_the_same_id() {
    let mut p = two();
    p.posted("s1", Kick, "forsen", false);
    p.echo(Twitch, "forsen", "gg wp");
    assert!(p.retry(5).is_none(), "nothing told yet");
    assert!(p.due(TIMEOUT_MS)[0].1.retry, "the refused kick leg can go again");
    let r = p.retry(TIMEOUT_MS + 5).unwrap();
    assert_eq!(&*r.id, "s1");
    assert_eq!(r.legs.iter().map(|l| (l.platform, &*l.text)).collect::<Vec<_>>(), vec![(Kick, "gg wp")]);

    // the timer starts over: it speaks again if the resend is not echoed either
    assert_eq!(p.next_due(), Some(TIMEOUT_MS + 5 + TIMEOUT_MS));
  }

  #[test]
  fn a_twitch_leg_never_echoed_is_resent() {
    let mut p = two();
    p.echo(Kick, "forsen", "gg wp");
    p.due(TIMEOUT_MS);
    let r = p.retry(TIMEOUT_MS).unwrap();
    assert_eq!(r.legs.iter().map(|l| l.platform).collect::<Vec<_>>(), vec![Twitch]);
  }

  #[test]
  fn a_kick_leg_whose_post_may_have_landed_is_never_resent() {
    let mut p = two();
    p.echo(Twitch, "forsen", "gg wp");
    // waiting on kick's job: not known to be unsent
    let due = p.due(TIMEOUT_MS);
    assert!(!due[0].1.retry, "the notice has nothing to offer");
    let r = p.retry(TIMEOUT_MS).unwrap();
    assert!(r.legs.is_empty());
    // heatsync took the POST: it may be in chat
    p.posted("s1", Kick, "forsen", true);
    let r = p.retry(TIMEOUT_MS).unwrap();
    assert!(r.legs.is_empty());
    // and a later "refused" word cannot turn it back into a resend
    p.posted("s1", Kick, "forsen", false);
    assert!(p.retry(TIMEOUT_MS).unwrap().legs.is_empty());
  }

  #[test]
  fn bounded_in_count_and_age() {
    let mut p = Pending::default();
    for i in 0..100 {
      p.track(i, &format!("s{i}"), vec![leg(Twitch, "a", "x")]);
    }
    assert_eq!(p.0.len(), MAX);
    assert!(p.due(KEEP_MS + 1000).is_empty() && p.0.is_empty(), "old sends are forgotten");
  }
}
