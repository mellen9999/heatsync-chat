// stream state lines (ext main.js stream-event handler + lib/stream-event-text.js): "went live — game",
// "switched to game", "went offline", and the chat-spike line. pure, clock passed in, so the pairing
// and the ext's anti-wall guards are testable. the ui decides which kinds to show (ext toggles)
use super::ident;
use std::collections::{HashMap, HashSet};

use crate::model::message::Platform;

// the ext's guards against the "went live" wall on connect: nothing in the first 90s, one per session
const GRACE_MS: i64 = 90_000;
// a go-live arrives as stream:update (game, no previous) + stream:online: wait for its twin
const HOLD_MS: i64 = 2_000;
const ECHO_MS: i64 = 30_000;
const DEDUPE_MS: i64 = 60_000;
const MOMENT_GAP_MS: i64 = 10 * 60_000;
const MAX_KEYS: usize = 256;
const MAX_GAME: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
  Online,
  Offline,
  Switch,
  Moment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
  Online,
  Offline,
  Update,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ev {
  pub kind: Kind,
  pub platform: Platform,
  pub channel: Box<str>,
  pub game: Box<str>,
  pub prev_game: Box<str>,
  // how the streamer writes their name, and their color, when the frame says (else the channel key)
  pub name: Box<str>,
  pub color: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
  pub platform: Platform,
  pub channel: Box<str>,
  // the whole line as words (`[ch] ◆ went live — game`): what dedupes it and what tests read
  pub text: Box<str>,
  pub tone: Tone,
  // the line as drawn (ext .hs-mc-stream-event): the name in their own color or paint, `◆ went live — `
  // in the tone's color, the game in white
  pub name: Box<str>,
  pub color: Option<u32>,
  pub verb: Box<str>,
  pub game: Box<str>,
}

// `follow:colors`: followed streamers' twitch colors, {login: "#hex"}. hostile: bounded count, valid
// logins, real colors; anything else is skipped
const MAX_COLORS: usize = 2000;

pub fn follow_colors(v: &serde_json::Value) -> Vec<(Box<str>, u32)> {
  let Some(map) = v.get("colors").and_then(serde_json::Value::as_object) else { return Vec::new() };
  map
    .iter()
    .take(MAX_COLORS)
    .filter(|(k, _)| ident::twitch_login(k))
    .filter_map(|(k, c)| Some((k.to_ascii_lowercase().into(), crate::model::color::parse_hex(c.as_str()?)?)))
    .collect()
}

pub fn clean(s: &str) -> Box<str> {
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_GAME).collect::<String>().trim().into()
}

// lib/stream-event-text.js streamEventBody
fn body(kind: Kind, game: &str, prev: &str) -> Option<String> {
  match kind {
    Kind::Update if game.is_empty() || prev == game => None,
    Kind::Update => Some(format!("switched to {game}")),
    Kind::Online if game.is_empty() => Some("went live".into()),
    Kind::Online => Some(format!("went live — {game}")),
    Kind::Offline => Some("went offline".into()),
  }
}

type Key = (u8, Box<str>);

fn key(e: &Ev) -> Key {
  (e.platform as u8, e.channel.clone())
}

#[derive(Default)]
pub struct Gate {
  start: Option<i64>,
  went_live: HashSet<Key>,
  held: Vec<(i64, Ev)>,
  last_online: HashMap<Key, (Box<str>, i64)>,
  shown: HashMap<String, i64>,
}

impl Gate {
  fn line(&mut self, now: i64, e: &Ev, tone: Tone, what: &str) -> Option<Line> {
    let text = format!("[{}] ◆ {what}", e.channel);
    // same event within a minute is one line (several sources report the same thing)
    let k = format!("{} {text}", e.channel);
    if self.shown.get(&k).is_some_and(|t| now - t < DEDUPE_MS) {
      return None;
    }
    if self.shown.len() >= MAX_KEYS {
      self.shown.retain(|_, t| now - *t < DEDUPE_MS);
    }
    self.shown.insert(k, now);
    let at = ["went live — ", "switched to "].iter().find_map(|v| what.strip_prefix(v).map(|g| (v.len(), g)));
    let (verb, game) = match at {
      Some((n, g)) => (&what[..n], g),
      None => (what, ""),
    };
    Some(Line {
      platform: e.platform,
      channel: e.channel.clone(),
      text: text.into(),
      tone,
      name: if e.name.is_empty() { e.channel.clone() } else { e.name.clone() },
      color: e.color,
      verb: format!("◆ {verb}").into(),
      game: game.into(),
    })
  }

  // the grace clock runs from launch, like the ext's content-script load. left to the first event it
  // would start late: the first go-live of a long session would count as "just connected" and vanish
  pub fn start(&mut self, now: i64) {
    self.start.get_or_insert(now);
  }

  pub fn event(&mut self, now: i64, e: Ev) -> Option<Line> {
    let start = *self.start.get_or_insert(now);
    let k = key(&e);
    match e.kind {
      Kind::Update => {
        let what = body(Kind::Update, &e.game, &e.prev_game)?;
        if !e.prev_game.is_empty() {
          return self.line(now, &e, Tone::Switch, &what);
        }
        if self.last_online.get(&k).is_some_and(|(g, at)| *g == e.game && now - at <= ECHO_MS) {
          return None;
        }
        self.held.retain(|(_, h)| key(h) != k);
        if self.held.len() < MAX_KEYS {
          self.held.push((now + HOLD_MS, e));
        }
        None
      }
      Kind::Online => {
        self.held.retain(|(_, h)| !(key(h) == k && h.game == e.game));
        if self.last_online.len() >= MAX_KEYS {
          self.last_online.clear();
        }
        self.last_online.insert(k.clone(), (e.game.clone(), now));
        // once per session per channel, and never inside the connect-snapshot window
        if self.went_live.contains(&k) {
          return None;
        }
        if self.went_live.len() >= MAX_KEYS {
          self.went_live.clear();
        }
        self.went_live.insert(k);
        if now - start < GRACE_MS {
          return None;
        }
        let what = body(Kind::Online, &e.game, "")?;
        self.line(now, &e, Tone::Online, &what)
      }
      Kind::Offline => {
        self.held.retain(|(_, h)| key(h) != k);
        self.last_online.remove(&k);
        self.went_live.remove(&k);
        let what = body(Kind::Offline, "", "")?;
        self.line(now, &e, Tone::Offline, &what)
      }
    }
  }

  // held game switches whose twin never came
  pub fn due(&mut self, now: i64) -> Vec<Line> {
    let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held).into_iter().partition(|(at, _)| *at <= now);
    self.held = rest;
    ready
      .into_iter()
      .filter_map(|(_, e)| {
        let what = body(Kind::Update, &e.game, &e.prev_game)?;
        self.line(now, &e, Tone::Switch, &what)
      })
      .collect()
  }

  pub fn next_due(&self) -> Option<i64> {
    self.held.iter().map(|(at, _)| *at).min()
  }
}

// ext handleMomentSpike text: "<ch> chat is exploding — N msgs/30s (usually ~B)"
pub fn moment_text(channel: &str, rate: f64, baseline: f64) -> Option<String> {
  let ok = |x: f64| x.is_finite() && (0.0..1e7).contains(&x);
  (ok(rate) && ok(baseline)).then(|| {
    format!("{channel} chat is exploding — {} msgs/30s (usually ~{})", rate.round(), baseline.round().max(1.0))
  })
}

// one spike line per channel per 10 minutes (the server's own cooldown), bounded
#[derive(Default)]
pub struct Moments(HashMap<Key, i64>);

impl Moments {
  pub fn allow(&mut self, now: i64, platform: Platform, channel: &str) -> bool {
    let k = (platform as u8, Box::<str>::from(channel));
    if self.0.get(&k).is_some_and(|t| now - t < MOMENT_GAP_MS) {
      return false;
    }
    if self.0.len() >= 200 {
      self.0.retain(|_, t| now - *t < MOMENT_GAP_MS);
    }
    self.0.insert(k, now);
    true
  }
}

#[cfg(test)]
mod tests {
  #[test]
  fn follow_colors_survive_hostile_frames() {
    use serde_json::json;
    let got = super::follow_colors(
      &json!({"colors": {"Knut": "#ff0000", "bad name": "#00ff00", "x": "nope", "y": 5, "": "#fff", "ok_1": "#8888ff"}}),
    );
    let mut got: Vec<_> = got.into_iter().map(|(k, c)| (k.to_string(), c)).collect();
    got.sort();
    assert_eq!(got, vec![("knut".to_string(), 0xff0000), ("ok_1".to_string(), 0x8888ff)]);
    for v in [json!({}), json!({"colors": []}), json!({"colors": "x"}), json!({"colors": null}), json!(7)] {
      assert!(super::follow_colors(&v).is_empty());
    }
    let big: serde_json::Map<String, serde_json::Value> =
      (0..5000).map(|i| (format!("u{i}"), json!("#ff0000"))).collect();
    assert_eq!(super::follow_colors(&json!({"colors": big})).len(), 2000);
  }

  use super::*;

  fn ev(kind: Kind, game: &str, prev: &str) -> Ev {
    let (name, color) = ("Cyr".into(), None);
    Ev {
      kind,
      platform: Platform::Twitch,
      channel: "cyr".into(),
      game: game.into(),
      prev_game: prev.into(),
      name,
      color,
    }
  }
  const T0: i64 = 1_000_000;
  const LATE: i64 = T0 + GRACE_MS + 1;

  #[test]
  fn go_live_pairs_with_its_starting_game() {
    let mut g = Gate::default();
    g.event(T0, ev(Kind::Offline, "", "")); // starts the clock
    // update (no previous game) is held; the online for the same game replaces it
    assert!(g.event(LATE, ev(Kind::Update, "Chess", "")).is_none());
    assert_eq!(g.next_due(), Some(LATE + HOLD_MS));
    let l = g.event(LATE + 10, ev(Kind::Online, "Chess", "")).unwrap();
    assert_eq!(&*l.text, "[cyr] ◆ went live — Chess");
    assert_eq!((&*l.name, &*l.verb, &*l.game), ("Cyr", "◆ went live — ", "Chess"), "drawn in three parts");
    assert_eq!(l.tone, Tone::Online);
    assert!(g.due(LATE + 5000).is_empty());
    // an update echoing the go-live right after is dropped
    assert!(g.event(LATE + 20, ev(Kind::Update, "Chess", "")).is_none());
    assert!(g.next_due().is_none());
  }

  #[test]
  fn first_go_live_of_a_long_session_shows() {
    let mut g = Gate::default();
    g.start(T0);
    // nothing for ten minutes, then riotgames goes live: that is the first event, and it must show
    let l = g.event(T0 + 600_000, ev(Kind::Online, "Valorant", "")).unwrap();
    assert_eq!(&*l.text, "[cyr] ◆ went live — Valorant");
    // inside the first 90s it is still the connect snapshot
    let mut early = Gate::default();
    early.start(T0);
    assert!(early.event(T0 + 30_000, ev(Kind::Online, "Valorant", "")).is_none());
  }

  #[test]
  fn lone_first_game_shows_after_the_hold_and_real_switch_is_immediate() {
    let mut g = Gate::default();
    g.event(T0, ev(Kind::Offline, "", ""));
    assert!(g.event(LATE, ev(Kind::Update, "Chess", "")).is_none());
    assert!(g.due(LATE + 1999).is_empty());
    let l = g.due(LATE + 2000);
    assert_eq!(&*l[0].text, "[cyr] ◆ switched to Chess");
    assert_eq!(l[0].tone, Tone::Switch);
    let l = g.event(LATE + 100_000, ev(Kind::Update, "Go", "Chess")).unwrap();
    assert_eq!(&*l.text, "[cyr] ◆ switched to Go");
    // same game as before = nothing to say
    assert!(g.event(LATE + 200_000, ev(Kind::Update, "Go", "Go")).is_none());
    assert!(g.event(LATE + 200_000, ev(Kind::Update, "", "Go")).is_none());
  }

  #[test]
  fn online_waits_out_the_grace_and_shows_once_per_session() {
    let mut g = Gate::default();
    // the connect-time snapshot
    assert!(g.event(T0, ev(Kind::Online, "Chess", "")).is_none());
    // later re-broadcast of the same go-live: already seen this session
    assert!(g.event(LATE, ev(Kind::Online, "Chess", "")).is_none());
    // a real offline then a real go-live shows again
    assert!(g.event(LATE + 1000, ev(Kind::Offline, "", "")).is_some());
    let l = g.event(LATE + 200_000, ev(Kind::Online, "", "")).unwrap();
    assert_eq!(&*l.text, "[cyr] ◆ went live");
    // a fresh channel after the grace shows at once
    let mut other = ev(Kind::Online, "Go", "");
    other.channel = "xqc".into();
    assert!(g.event(LATE + 300_000, other).is_some());
  }

  #[test]
  fn identical_lines_dedupe_for_a_minute() {
    let mut g = Gate::default();
    g.event(T0, ev(Kind::Offline, "", ""));
    assert!(g.event(LATE, ev(Kind::Offline, "", "")).is_some());
    assert!(g.event(LATE + 1000, ev(Kind::Offline, "", "")).is_none());
    assert!(g.event(LATE + 61_000, ev(Kind::Offline, "", "")).is_some());
  }

  #[test]
  fn moments_text_and_cooldown() {
    assert_eq!(moment_text("cyr", 120.4, 0.2).as_deref(), Some("cyr chat is exploding — 120 msgs/30s (usually ~1)"));
    assert!(moment_text("cyr", f64::NAN, 3.0).is_none());
    assert!(moment_text("cyr", 1e12, 3.0).is_none());
    let mut m = Moments::default();
    assert!(m.allow(0, Platform::Twitch, "cyr"));
    assert!(!m.allow(MOMENT_GAP_MS - 1, Platform::Twitch, "cyr"));
    assert!(m.allow(0, Platform::Kick, "cyr"));
    assert!(m.allow(MOMENT_GAP_MS, Platform::Twitch, "cyr"));
  }
}
