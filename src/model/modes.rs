// a twitch room's chat modes (ext chat-modes.js + input.js CHAT_MODES): emote-only, followers-only,
// subs-only, slow and unique chat. what they are comes from twitch's ROOMSTATE (live, free); a mod
// changes them through heatsync (POST /api/mod/chat-settings, helix's names), since twitch retired
// the irc commands. /slow 30, /slow off, /slowoff, /followers 10m, /emoteonly, /unique off …
use crate::i18n::{f, t};
use serde_json::{Value, json};

// helix's bounds (and the ext's): slow 3-120s, followers up to 90 days in minutes
pub const SLOW_MIN: u32 = 3;
pub const SLOW_MAX: u32 = 120;
pub const SLOW_DEFAULT: u32 = 30;
pub const FOLLOW_MAX: u32 = 129_600;
// the steps h / l walk through on the status screen
pub const SLOW_STEPS: [u32; 7] = [3, 5, 10, 20, 30, 60, 120];
pub const FOLLOW_STEPS: [u32; 8] = [0, 10, 30, 60, 1440, 10_080, 43_200, 129_600];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
  Emote,
  Followers,
  Subs,
  Slow,
  Unique,
}

pub const ALL: [Mode; 5] = [Mode::Emote, Mode::Followers, Mode::Subs, Mode::Slow, Mode::Unique];

impl Mode {
  pub fn label(self) -> &'static str {
    match self {
      Mode::Emote => t("mod.mode.emote"),
      Mode::Followers => t("mod.mode.followers"),
      Mode::Subs => t("mod.mode.subs"),
      Mode::Slow => t("mod.mode.slow"),
      Mode::Unique => t("mod.mode.unique"),
    }
  }

  pub fn cmd(self) -> &'static str {
    match self {
      Mode::Emote => "emoteonly",
      Mode::Followers => "followers",
      Mode::Subs => "subscribers",
      Mode::Slow => "slow",
      Mode::Unique => "unique",
    }
  }
}

// what a room's modes are; a number = on with that wait (slow: seconds, followers: minutes followed)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
  pub emote: bool,
  pub followers: Option<u32>,
  pub subs: bool,
  pub slow: Option<u32>,
  pub unique: bool,
}

impl Modes {
  pub fn get(&self, m: Mode) -> Option<u32> {
    match m {
      Mode::Emote => self.emote.then_some(0),
      Mode::Followers => self.followers,
      Mode::Subs => self.subs.then_some(0),
      Mode::Slow => self.slow,
      Mode::Unique => self.unique.then_some(0),
    }
  }

  pub fn put(&mut self, s: Set) {
    match s.mode {
      Mode::Emote => self.emote = s.on.is_some(),
      Mode::Followers => self.followers = s.on,
      Mode::Subs => self.subs = s.on.is_some(),
      Mode::Slow => self.slow = s.on,
      Mode::Unique => self.unique = s.on.is_some(),
    }
  }
}

// one change: a mode on (with its number; 0 for the plain switches) or off
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Set {
  pub mode: Mode,
  pub on: Option<u32>,
}

impl Set {
  // clamped to what twitch takes
  pub fn new(mode: Mode, on: Option<u32>) -> Self {
    let on = match mode {
      Mode::Slow => on.filter(|n| *n > 0).map(|n| n.clamp(SLOW_MIN, SLOW_MAX)),
      Mode::Followers => on.map(|n| n.min(FOLLOW_MAX)),
      _ => on.map(|_| 0),
    };
    Set { mode, on }
  }

  // the POST's `settings` (helix names; a wait rides only with its mode on)
  pub fn body(self) -> Value {
    match (self.mode, self.on) {
      (Mode::Emote, on) => json!({ "emote_mode": on.is_some() }),
      (Mode::Subs, on) => json!({ "subscriber_mode": on.is_some() }),
      (Mode::Unique, on) => json!({ "unique_chat_mode": on.is_some() }),
      (Mode::Slow, Some(n)) => json!({ "slow_mode": true, "slow_mode_wait_time": n }),
      (Mode::Slow, None) => json!({ "slow_mode": false }),
      (Mode::Followers, Some(n)) => json!({ "follower_mode": true, "follower_mode_duration": n }),
      (Mode::Followers, None) => json!({ "follower_mode": false }),
    }
  }

  // "slow mode on (30s)", "followers-only on (10m)", "emote-only off"
  pub fn done(self) -> String {
    match (self.mode, self.on) {
      (m, None) => f("mod.mode.off", &[("mode", m.label())]),
      (Mode::Slow, Some(n)) => f("mod.mode.on_secs", &[("mode", self.mode.label()), ("n", &n.to_string())]),
      (Mode::Followers, Some(0)) => f("mod.mode.on_any", &[("mode", self.mode.label())]),
      (Mode::Followers, Some(n)) => f("mod.mode.on_span", &[("mode", self.mode.label()), ("span", &minutes(n))]),
      (m, Some(_)) => f("mod.mode.on", &[("mode", m.label())]),
    }
  }
}

// 90 → "1h 30m", 1440 → "1d"
pub fn minutes(n: u32) -> String {
  let (d, h, m) = (n / 1440, n % 1440 / 60, n % 60);
  let parts: Vec<String> =
    [(d, "d"), (h, "h"), (m, "m")].into_iter().filter(|(v, _)| *v > 0).map(|(v, u)| format!("{v}{u}")).collect();
  if parts.is_empty() { "0m".into() } else { parts.join(" ") }
}

// a status screen row: "slow mode · 30s", "followers-only · any follower", "emote-only"
pub fn row(m: Mode, v: Option<u32>) -> String {
  match (m, v) {
    (Mode::Slow, Some(n)) => f("mod.mode.row_secs", &[("mode", m.label()), ("n", &n.to_string())]),
    (Mode::Followers, Some(0)) => f("mod.mode.row_any", &[("mode", m.label())]),
    (Mode::Followers, Some(n)) => f("mod.mode.row_span", &[("mode", m.label()), ("span", &minutes(n))]),
    _ => m.label().into(),
  }
}

// a ROOMSTATE's mode tags as changes: only the ones it carries (a change sends just that one)
pub fn roomstate(tag: impl Fn(&str) -> Option<String>) -> Vec<Set> {
  let num = |k: &str| tag(k).and_then(|v| v.parse::<i64>().ok());
  let mut out = Vec::new();
  for (k, mode) in [("emote-only", Mode::Emote), ("subs-only", Mode::Subs), ("r9k", Mode::Unique)] {
    if let Some(v) = num(k) {
      out.push(Set { mode, on: (v == 1).then_some(0) });
    }
  }
  if let Some(v) = num("slow") {
    out.push(Set { mode: Mode::Slow, on: (v > 0).then(|| v.min(86_400) as u32) });
  }
  // -1 = off, 0 = any follower
  if let Some(v) = num("followers-only") {
    out.push(Set { mode: Mode::Followers, on: (v >= 0).then(|| v.min(i64::from(FOLLOW_MAX)) as u32) });
  }
  out
}

// a typed duration (ext _parseModeDuration): seconds for slow (bare = s; m, h), minutes for followers
// (bare = m; h, d, w; s rounds up to a minute)
fn duration(arg: &str, secs: bool) -> Option<u32> {
  let a = arg.trim();
  let digits = a.bytes().take_while(u8::is_ascii_digit).count();
  let n: u64 = a.get(..digits)?.parse().ok()?;
  let k: u64 = match (secs, a[digits..].trim()) {
    (true, "" | "s") => 1,
    (true, "m") => 60,
    (true, "h") => 3600,
    (false, "" | "m") => 1,
    (false, "h") => 60,
    (false, "d") => 1440,
    (false, "w") => 10_080,
    (false, "s") => return Some(n.div_ceil(60).min(u64::from(FOLLOW_MAX)) as u32),
    _ => return None,
  };
  Some(n.saturating_mul(k).min(if secs { 86_400 } else { u64::from(FOLLOW_MAX) }) as u32)
}

// a typed mode command: Some(Ok(change)) / Some(Err(usage)) for ours, None for anything else
pub fn command(text: &str) -> Option<Result<Set, String>> {
  let t = text.trim();
  let rest = t.strip_prefix('/')?;
  let (word, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  let word = word.to_ascii_lowercase();
  let (mode, off) = match word.as_str() {
    "emoteonly" | "emote" | "emoteonlymode" => (Mode::Emote, false),
    "emoteonlyoff" => (Mode::Emote, true),
    "followers" | "followersonly" | "followeronly" => (Mode::Followers, false),
    "followersoff" => (Mode::Followers, true),
    "subscribers" | "subonly" | "subsonly" | "subscribersonly" | "subs" => (Mode::Subs, false),
    "subscribersoff" => (Mode::Subs, true),
    "slow" | "slowmode" => (Mode::Slow, false),
    "slowoff" => (Mode::Slow, true),
    "unique" | "uniquechat" | "r9k" | "r9kbeta" => (Mode::Unique, false),
    "uniquechatoff" => (Mode::Unique, true),
    _ => return None,
  };
  let arg = arg.trim().to_ascii_lowercase();
  if off || arg == "off" {
    return Some(Ok(Set::new(mode, None)));
  }
  let on = match mode {
    _ if arg.is_empty() || arg == "on" => Some(if mode == Mode::Slow { SLOW_DEFAULT } else { 0 }),
    Mode::Slow => duration(&arg, true),
    Mode::Followers => duration(&arg, false),
    _ => None,
  };
  let usage = || f("mod.mode.usage", &[("cmd", mode.cmd())]);
  Some(match on {
    Some(0) if mode == Mode::Slow => Ok(Set::new(mode, None)),
    Some(n) => Ok(Set::new(mode, Some(n))),
    None => Err(usage()),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn typed_commands_read_like_the_ext() {
    let ok = |t: &str| command(t).unwrap().unwrap();
    assert_eq!(ok("/slow"), Set { mode: Mode::Slow, on: Some(30) });
    assert_eq!(ok("/slow 2m"), Set { mode: Mode::Slow, on: Some(120) });
    assert_eq!(ok("/slow 1"), Set { mode: Mode::Slow, on: Some(3) }, "twitch's least");
    assert_eq!(ok("/slow off"), Set { mode: Mode::Slow, on: None });
    assert_eq!(ok("/slowoff"), Set { mode: Mode::Slow, on: None });
    assert_eq!(ok("/followers"), Set { mode: Mode::Followers, on: Some(0) });
    assert_eq!(ok("/followers 1d"), Set { mode: Mode::Followers, on: Some(1440) });
    assert_eq!(ok("/followers 90s"), Set { mode: Mode::Followers, on: Some(2) });
    assert_eq!(ok("/followers 99w"), Set { mode: Mode::Followers, on: Some(FOLLOW_MAX) });
    assert_eq!(ok("/R9K"), Set { mode: Mode::Unique, on: Some(0) });
    assert_eq!(ok("/emoteonlyoff"), Set { mode: Mode::Emote, on: None });
    assert_eq!(ok("/subs off"), Set { mode: Mode::Subs, on: None });
    assert_eq!(command("/slow soon"), Some(Err("usage: /slow [duration|off]".into())));
    assert!(command("/slowpoke").is_none() && command("slow").is_none());
  }

  #[test]
  fn bodies_and_words() {
    assert_eq!(Set::new(Mode::Slow, Some(30)).body(), json!({"slow_mode": true, "slow_mode_wait_time": 30}));
    assert_eq!(Set::new(Mode::Followers, None).body(), json!({"follower_mode": false}));
    assert_eq!(Set::new(Mode::Unique, Some(9)).body(), json!({"unique_chat_mode": true}));
    assert_eq!(Set::new(Mode::Followers, Some(90)).done(), "followers-only on (1h 30m)");
    assert_eq!(Set::new(Mode::Followers, Some(0)).done(), "followers-only on (any follower)");
    assert_eq!(Set::new(Mode::Emote, None).done(), "emote-only off");
    assert_eq!((row(Mode::Slow, Some(10)), row(Mode::Slow, None)), ("slow mode · 10s".into(), "slow mode".into()));
  }

  #[test]
  fn roomstate_changes_only_what_it_carries() {
    let read = |tags: &[(&str, &str)]| {
      let t: Vec<(String, String)> = tags.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
      roomstate(move |k| t.iter().find(|x| x.0 == k).map(|x| x.1.clone()))
    };
    let mut m = Modes::default();
    let full = read(&[("emote-only", "0"), ("followers-only", "10"), ("r9k", "1"), ("slow", "0"), ("subs-only", "0")]);
    full.into_iter().for_each(|s| m.put(s));
    assert_eq!(m, Modes { followers: Some(10), unique: true, ..Modes::default() });
    read(&[("slow", "30")]).into_iter().for_each(|s| m.put(s));
    assert_eq!(m, Modes { followers: Some(10), unique: true, slow: Some(30), ..Modes::default() });
    read(&[("followers-only", "-1")]).into_iter().for_each(|s| m.put(s));
    assert_eq!(m.followers, None);
    assert!(read(&[("room-id", "1")]).is_empty());
  }
}
