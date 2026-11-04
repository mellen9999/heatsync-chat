// why you can't chat in a room right now, kept so the message box can say it before you type
// (the ext only toasts when a send bounces: auth-irc.js TWITCH_SEND_REJECT_NOTICES, words from its locale).
// signals: twitch's CLEARCHAT aimed at you (a ban, or a timeout with its length), the NOTICE a refused line
// gets (msg_banned, msg_timedout, msg_followersonly*, msg_subsonly, msg_emoteonly, msg_slowmode) and kick's
// refusal text. cleared by: the time running out, ROOMSTATE turning that mode off, or a line of yours getting through.
// every number comes off the wire, so each is clamped
use std::collections::HashMap;

use super::message::Platform;
use super::modes::{Mode, SLOW_DEFAULT, SLOW_MAX, SLOW_MIN, Set};

// twitch's longest timeout: two weeks
pub const MAX_TIMEOUT_SECS: u32 = 1_209_600;
// rooms we sit in are few; this only bounds a flood of junk
const MAX_LOCKS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
  Banned,
  // seconds left, as twitch told them
  Timeout(u32),
  Followers,
  FollowLonger,
  FollowFirst,
  Subs,
  Emote,
  Slow,
}

impl Cause {
  // the room mode that, once off, frees you
  fn mode(self) -> Option<Mode> {
    match self {
      Cause::Followers | Cause::FollowLonger | Cause::FollowFirst => Some(Mode::Followers),
      Cause::Subs => Some(Mode::Subs),
      Cause::Emote => Some(Mode::Emote),
      _ => None,
    }
  }

  // a ban or timeout outranks a mode: a later mode notice never hides it
  fn rank(self) -> u8 {
    match self {
      Cause::Banned => 3,
      Cause::Timeout(_) => 2,
      _ => 1,
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lock {
  pub cause: Cause,
  // ms epoch it ends on its own (timeouts, slow mode); none = until something says otherwise
  pub until: Option<i64>,
}

impl Lock {
  pub fn new(cause: Cause, now: i64, slow: Option<u32>) -> Self {
    let secs = match cause {
      Cause::Timeout(n) => Some(n.clamp(1, MAX_TIMEOUT_SECS)),
      Cause::Slow => Some(slow.unwrap_or(SLOW_DEFAULT).clamp(SLOW_MIN, SLOW_MAX)),
      _ => None,
    };
    Lock { cause, until: secs.map(|s| now + i64::from(s) * 1000) }
  }

  fn live(&self, now: i64) -> bool {
    self.until.is_none_or(|u| u > now)
  }

  // a timeout has an end we know, so a send is held back; the rest only tell (twitch is the judge: you may have
  // been unbanned or followed since, and nothing would ever tell us)
  pub fn blocks(&self) -> bool {
    matches!(self.cause, Cause::Timeout(_))
  }

  // red = you are out; yellow = the room's rule you may not meet
  pub fn out(&self) -> bool {
    self.cause.rank() > 1
  }

  // the ext's words, lowercased; a countdown rides on the ones with an end
  pub fn text(&self, now: i64) -> String {
    let left = self.until.map(|u| left(u - now));
    match (self.cause, left) {
      (Cause::Banned, _) => "you are banned from this channel".into(),
      (Cause::Timeout(_), Some(l)) => format!("you are timed out · {l} left"),
      (Cause::Timeout(_), None) => "you are timed out".into(),
      (Cause::Followers, _) => "followers-only mode — follow the channel to chat".into(),
      (Cause::FollowLonger, _) => "follow the channel a bit longer to chat".into(),
      (Cause::FollowFirst, _) => "followers-only — you need to follow first".into(),
      (Cause::Subs, _) => "subscribers-only — sub to chat here".into(),
      (Cause::Emote, _) => "emote-only mode — message must be all emotes".into(),
      (Cause::Slow, Some(l)) => format!("slow mode — wait {l}"),
      (Cause::Slow, None) => "slow mode — please wait a moment".into(),
    }
  }

  // ms until the countdown shows a different number (always > 0 while live)
  pub fn flips_in(&self, now: i64) -> Option<i64> {
    let ms = self.until? - now;
    (ms > 0).then(|| (ms - 1) % 1000 + 1)
  }
}

// "45s", "4m32s", "2h05m", "3d"
fn left(ms: i64) -> String {
  let s = (ms.max(0) + 999) / 1000;
  match (s / 86_400, s % 86_400 / 3600, s % 3600 / 60, s % 60) {
    (0, 0, 0, s) => format!("{s}s"),
    (0, 0, m, s) => format!("{m}m{s:02}s"),
    (0, h, m, _) => format!("{h}h{m:02}m"),
    (d, h, ..) => format!("{d}d{h:02}h"),
  }
}

// the first number in a notice's words ("you are timed out for 577 more seconds"), if sane
fn first_number(s: &str) -> Option<u32> {
  let digits: String = s.chars().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).take(9).collect();
  digits.parse().ok()
}

// a twitch NOTICE's msg-id (and words) that says a line of ours was refused for a reason that lasts
pub fn from_notice(id: &str, text: &str) -> Option<Cause> {
  Some(match id {
    "msg_banned" => Cause::Banned,
    "msg_timedout" => Cause::Timeout(first_number(text).filter(|n| *n > 0)?.min(MAX_TIMEOUT_SECS)),
    "msg_followersonly" => Cause::Followers,
    "msg_followersonly_followed" => Cause::FollowLonger,
    "msg_followersonly_zero" => Cause::FollowFirst,
    "msg_subsonly" => Cause::Subs,
    "msg_emoteonly" => Cause::Emote,
    "msg_slowmode" => Cause::Slow,
    _ => return None,
  })
}

// heatsync's kick send refusal is free text: only the plain words count
pub fn from_kick(why: &str) -> Option<Cause> {
  let w = why.to_ascii_lowercase();
  if w.contains("banned") {
    Some(Cause::Banned)
  } else if w.contains("followers") {
    Some(Cause::Followers)
  } else if w.contains("subscriber") {
    Some(Cause::Subs)
  } else {
    None
  }
}

#[derive(Default)]
pub struct Locks(HashMap<(Platform, Box<str>), Lock>);

impl Locks {
  // told by the room itself (CLEARCHAT at you, kick's answer): it replaces whatever was there
  pub fn put(&mut self, p: Platform, chan: &str, lock: Lock, now: i64) {
    self.0.retain(|_, l| l.live(now));
    if self.0.len() < MAX_LOCKS || self.0.contains_key(&(p, chan.into())) {
      self.0.insert((p, chan.into()), lock);
    }
  }

  // told by a bounced send: never hides a ban or timeout we already know of
  pub fn offer(&mut self, p: Platform, chan: &str, lock: Lock, now: i64) {
    if self.get(p, chan, now).is_some_and(|l| l.cause.rank() > lock.cause.rank()) {
      return;
    }
    self.put(p, chan, lock, now);
  }

  pub fn get(&self, p: Platform, chan: &str, now: i64) -> Option<&Lock> {
    self.0.get(&(p, chan.into())).filter(|l| l.live(now))
  }

  pub fn clear(&mut self, p: Platform, chan: &str) -> bool {
    self.0.remove(&(p, chan.into())).is_some()
  }

  pub fn clear_all(&mut self) {
    self.0.clear();
  }

  // ROOMSTATE: a mode that went off frees the lock it caused
  pub fn modes(&mut self, chan: &str, sets: &[Set]) -> bool {
    let key = (Platform::Twitch, Box::<str>::from(chan));
    let Some(l) = self.0.get(&key) else { return false };
    let freed = l.cause.mode().is_some_and(|m| sets.iter().any(|s| s.mode == m && s.on.is_none()));
    if freed {
      self.0.remove(&key);
    }
    freed
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const T: Platform = Platform::Twitch;

  fn at(c: Cause) -> Lock {
    Lock::new(c, 0, None)
  }

  #[test]
  fn notices_name_their_cause() {
    assert_eq!(from_notice("msg_banned", ""), Some(Cause::Banned));
    assert_eq!(from_notice("msg_timedout", "You are timed out for 577 more seconds."), Some(Cause::Timeout(577)));
    assert_eq!(from_notice("msg_followersonly_zero", "x"), Some(Cause::FollowFirst));
    assert_eq!(from_notice("msg_followersonly_followed", "x"), Some(Cause::FollowLonger));
    assert_eq!(from_notice("msg_subsonly", "x"), Some(Cause::Subs));
    assert_eq!(from_notice("msg_emoteonly", "x"), Some(Cause::Emote));
    assert_eq!(from_notice("msg_slowmode", "x"), Some(Cause::Slow));
    // not a lasting reason, or no length to count down: nothing is locked
    assert_eq!(from_notice("msg_duplicate", "x"), None);
    assert_eq!(from_notice("msg_timedout", "You are timed out."), None);
    assert_eq!(from_notice("msg_timedout", "timed out for 0 more seconds"), None);
    assert_eq!(from_notice("msg_rejected", "x"), None);
  }

  #[test]
  fn hostile_lengths_are_clamped() {
    assert_eq!(from_notice("msg_timedout", "for 99999999 seconds"), Some(Cause::Timeout(MAX_TIMEOUT_SECS)));
    assert_eq!(from_notice("msg_timedout", "for 99999999999999999999 s"), Some(Cause::Timeout(MAX_TIMEOUT_SECS)));
    assert_eq!(Lock::new(Cause::Timeout(u32::MAX), 0, None).until, Some(i64::from(MAX_TIMEOUT_SECS) * 1000));
    assert_eq!(Lock::new(Cause::Timeout(0), 0, None).until, Some(1000));
    assert_eq!(Lock::new(Cause::Slow, 0, Some(9999)).until, Some(120_000));
    assert_eq!(Lock::new(Cause::Slow, 0, None).until, Some(30_000));
  }

  #[test]
  fn kick_words() {
    assert_eq!(from_kick("You are banned from this channel"), Some(Cause::Banned));
    assert_eq!(from_kick("followers only chat"), Some(Cause::Followers));
    assert_eq!(from_kick("subscribers only"), Some(Cause::Subs));
    assert_eq!(from_kick("slow down"), None);
  }

  #[test]
  fn timeout_counts_down_and_ends() {
    let mut ls = Locks::default();
    ls.put(T, "c", Lock::new(Cause::Timeout(272), 1000, None), 1000);
    let l = *ls.get(T, "c", 1000).unwrap();
    assert_eq!(l.text(1000), "you are timed out · 4m32s left");
    assert_eq!(l.text(1000 + 1500), "you are timed out · 4m31s left");
    assert!(l.blocks() && l.out());
    assert_eq!(l.flips_in(1000), Some(1000));
    assert_eq!(l.flips_in(1400), Some(600));
    assert!(ls.get(T, "c", 1000 + 272_000).is_none(), "gone the moment it ends");
    assert!(ls.get(T, "other", 1000).is_none(), "per room");
    assert!(ls.get(Platform::Kick, "c", 1000).is_none(), "per platform");
  }

  #[test]
  fn units_read_plainly() {
    assert_eq!(left(1), "1s");
    assert_eq!(left(45_000), "45s");
    assert_eq!(left(60_000), "1m00s");
    assert_eq!(left(3_900_000), "1h05m");
    assert_eq!(left(1_209_600_000), "14d00h");
  }

  #[test]
  fn a_mode_notice_never_hides_a_ban_but_the_room_can_replace_it() {
    let mut ls = Locks::default();
    ls.put(T, "c", at(Cause::Banned), 0);
    ls.offer(T, "c", at(Cause::Subs), 0);
    assert_eq!(ls.get(T, "c", 0).unwrap().cause, Cause::Banned);
    ls.offer(T, "c", at(Cause::Timeout(60)), 0);
    assert_eq!(ls.get(T, "c", 0).unwrap().cause, Cause::Banned);
    // CLEARCHAT at you after an unban is the room speaking
    ls.put(T, "c", at(Cause::Timeout(60)), 0);
    assert_eq!(ls.get(T, "c", 0).unwrap().cause, Cause::Timeout(60));
  }

  #[test]
  fn only_a_timeout_holds_a_send() {
    for c in [Cause::Banned, Cause::Followers, Cause::Subs, Cause::Emote, Cause::Slow] {
      assert!(!at(c).blocks(), "{c:?}");
    }
    assert!(!at(Cause::Subs).out() && at(Cause::Banned).out());
  }

  #[test]
  fn room_modes_off_free_the_mode_lock_only() {
    let mut ls = Locks::default();
    ls.put(T, "c", at(Cause::Subs), 0);
    assert!(!ls.modes("c", &[Set::new(Mode::Followers, None)]));
    assert!(!ls.modes("c", &[Set::new(Mode::Subs, Some(0))]), "still on");
    assert!(ls.modes("c", &[Set::new(Mode::Subs, None)]));
    assert!(ls.get(T, "c", 0).is_none());
    ls.put(T, "c", at(Cause::Banned), 0);
    assert!(!ls.modes("c", &[Set::new(Mode::Subs, None), Set::new(Mode::Followers, None)]));
    assert!(ls.get(T, "c", 0).is_some());
  }

  #[test]
  fn a_sent_line_clears_and_junk_is_bounded() {
    let mut ls = Locks::default();
    ls.put(T, "c", at(Cause::Banned), 0);
    assert!(ls.clear(T, "c") && !ls.clear(T, "c"));
    for i in 0..500 {
      ls.put(T, &format!("r{i}"), at(Cause::Banned), 0);
    }
    assert!(ls.0.len() <= MAX_LOCKS);
    // expired ones make room
    let mut ls = Locks::default();
    for i in 0..MAX_LOCKS {
      ls.put(T, &format!("r{i}"), at(Cause::Timeout(1)), 0);
    }
    ls.put(T, "new", at(Cause::Banned), 5000);
    assert!(ls.get(T, "new", 5000).is_some());
  }
}
