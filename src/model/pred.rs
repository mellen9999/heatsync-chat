// a channel's twitch prediction (ext pred-view.js + twitch-api.js fetchPrediction / updateChatBanners),
// read only: the open one, else the locked one, else one that ended in the last 5 minutes. twitch's
// public web gql answers this without a login; every field is bounded and a junk answer reads as nothing.
// polls have no public query (the ext only sees them in twitch's own page traffic), so there are none here
use crate::i18n::{f, t, tn};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::ui::theme;
use crate::util::time::parse_iso;

pub const MAX_OUTCOMES: usize = 10;
const MAX_TITLE: usize = 140;
const MAX_NAME: usize = 60;
const MAX_ID: usize = 64;
// the ext shows an ended prediction for 5 minutes
const SHOW_ENDED_MS: i64 = 300_000;
// the ext's default window when twitch omits it
const WINDOW_DEFAULT: u64 = 120;
const WINDOW_MAX: u64 = 1800;
// asking: every 15s while one is up (ext), a slow look otherwise, longer after failures
pub const LIVE_EVERY: Duration = Duration::from_secs(15);
pub const IDLE_EVERY: Duration = Duration::from_secs(45);
const BACKOFF_MAX: Duration = Duration::from_secs(180);
// a request that never came back is forgotten after this
const LOST: Duration = Duration::from_secs(30);
// width of the share bar, in cells
pub const BAR: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
  Active,
  Locked,
  Resolved,
  Canceled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
  pub title: Box<str>,
  pub points: u64,
  pub users: u64,
  pub ink: u32,
  pub won: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pred {
  pub id: Box<str>,
  pub title: Box<str>,
  pub state: State,
  // when it opened and how long bets are taken (ms, s), and when it ended (ms; 0 = unknown)
  pub opened: i64,
  pub window: u32,
  pub ended: i64,
  pub outcomes: Vec<Outcome>,
}

pub fn query(login: &str) -> Value {
  const F: &str = "id title status createdAt endedAt predictionWindowSeconds winningOutcome{id} \
                   outcomes{id title totalPoints totalUsers color}";
  serde_json::json!({
    "query": format!(
      "query($login:String!){{user(login:$login){{channel{{activePredictionEvents{{{F}}} \
       lockedPredictionEvents{{{F}}} resolvedPredictionEvents(first:1){{edges{{node{{{F}}}}}}}}}}}}}"
    ),
    "variables": { "login": login },
  })
}

// bounded plain text: control and direction-bending characters out, cut to `max` characters
fn text(v: Option<&Value>, max: usize) -> Box<str> {
  let s = v.and_then(Value::as_str).unwrap_or("");
  let bidi =
    |c: char| matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}');
  s.chars()
    .filter(|c| !bidi(*c))
    .map(|c| if c.is_control() { ' ' } else { c })
    .take(max)
    .collect::<String>()
    .trim()
    .into()
}

// the ext's outcomeColor, in our palette: pink magenta, blue and anything unknown white
fn ink(name: &str) -> u32 {
  match name {
    "PINK" | "PURPLE" => theme::MAGENTA,
    "GREEN" => theme::GREEN,
    "TEAL" | "LIGHT_BLUE" => theme::CYAN,
    "YELLOW" | "BROWN" => theme::YELLOW,
    "RED" => theme::RED,
    _ => theme::WHITE,
  }
}

fn outcome(o: &Value, winner: Option<&str>) -> Option<Outcome> {
  let title = text(o.get("title"), MAX_NAME);
  if title.is_empty() {
    return None;
  }
  let id = o.get("id").and_then(Value::as_str).filter(|i| i.len() <= MAX_ID);
  Some(Outcome {
    title,
    points: o.get("totalPoints").and_then(Value::as_u64).unwrap_or(0),
    users: o.get("totalUsers").and_then(Value::as_u64).unwrap_or(0),
    ink: ink(o.get("color").and_then(Value::as_str).unwrap_or("")),
    won: id.is_some() && id == winner,
  })
}

// one event; None = not one we can show (unknown state, no id or time, fewer than two outcomes)
fn event(e: &Value) -> Option<Pred> {
  let id = e.get("id").and_then(Value::as_str).filter(|i| !i.is_empty() && i.len() <= MAX_ID)?;
  let state = match e.get("status").and_then(Value::as_str)? {
    "ACTIVE" => State::Active,
    "LOCKED" => State::Locked,
    "RESOLVED" => State::Resolved,
    "CANCELED" => State::Canceled,
    _ => return None,
  };
  let opened = e.get("createdAt").and_then(Value::as_str).and_then(parse_iso)?;
  let winner = e.pointer("/winningOutcome/id").and_then(Value::as_str);
  let outcomes: Vec<Outcome> =
    e.get("outcomes")?.as_array()?.iter().take(MAX_OUTCOMES).filter_map(|o| outcome(o, winner)).collect();
  if outcomes.len() < 2 {
    return None;
  }
  let title = text(e.get("title"), MAX_TITLE);
  if title.is_empty() {
    return None;
  }
  Some(Pred {
    id: id.into(),
    title,
    state,
    opened,
    window: e.get("predictionWindowSeconds").and_then(Value::as_u64).unwrap_or(WINDOW_DEFAULT).clamp(1, WINDOW_MAX)
      as u32,
    ended: e.get("endedAt").and_then(Value::as_str).and_then(parse_iso).unwrap_or(0),
    outcomes,
  })
}

// the answer: None = junk (ask again later), Some(None) = nothing to show, Some(Some(p)) = this one.
// priority is the ext's: open > locked > ended lately
pub fn parse(v: &Value, now: i64) -> Option<Option<Pred>> {
  if v.get("errors").is_some_and(|e| !e.is_null()) {
    return None;
  }
  let u = v.pointer("/data/user")?;
  if u.is_null() {
    return Some(None);
  }
  let ch = u.get("channel").filter(|c| c.is_object())?;
  let list = |k: &str| -> Vec<Pred> {
    ch.get(k)
      .and_then(Value::as_array)
      .map_or_else(Vec::new, |a| a.iter().take(MAX_OUTCOMES).filter_map(event).collect())
  };
  let active = list("activePredictionEvents");
  let pick = active
    .iter()
    .find(|p| p.state == State::Active)
    .or(active.first())
    .cloned()
    .or_else(|| list("lockedPredictionEvents").into_iter().next())
    .or_else(|| {
      let n = ch.pointer("/resolvedPredictionEvents/edges/0/node")?;
      event(n).filter(|p| p.fresh(now))
    });
  Some(pick)
}

impl Pred {
  pub fn live(&self) -> bool {
    matches!(self.state, State::Active | State::Locked)
  }

  // still worth showing: live, or ended less than 5 minutes ago
  pub fn fresh(&self, now: i64) -> bool {
    self.live() || now - if self.ended > 0 { self.ended } else { self.opened } < SHOW_ENDED_MS
  }

  // when bets close (ms)
  pub fn closes(&self) -> i64 {
    self.opened.saturating_add(i64::from(self.window) * 1000)
  }

  pub fn points(&self) -> u64 {
    self.outcomes.iter().fold(0u64, |a, o| a.saturating_add(o.points))
  }

  pub fn users(&self) -> u64 {
    self.outcomes.iter().fold(0u64, |a, o| a.saturating_add(o.users))
  }

  // outcome `i`'s share of the points, in percent
  pub fn pct(&self, i: usize) -> u32 {
    self.share(i, 100) as u32
  }

  // outcome `i`'s share of the points scaled to `of`, rounded
  pub fn share(&self, i: usize, of: u64) -> u64 {
    let (t, p) = (u128::from(self.points()), u128::from(self.outcomes.get(i).map_or(0, |o| o.points)));
    (p * u128::from(of) + t / 2).checked_div(t).unwrap_or(0) as u64
  }

  // what the state says right now: the countdown while bets are open, else the state's word
  pub fn when(&self, now: i64) -> String {
    match self.state {
      State::Active => left(self.closes() - now),
      State::Locked => t("mod.pred.locked").into(),
      State::Resolved => t("mod.pred.ended").into(),
      State::Canceled => t("mod.pred.refunded").into(),
    }
  }

  // ms until the countdown reads differently (None = it isn't counting)
  pub fn tick_in(&self, now: i64) -> Option<u64> {
    let r = self.closes().saturating_sub(now);
    (self.state == State::Active && r > 0).then(|| ((r - 1).rem_euclid(1000) + 1) as u64)
  }
}

// "1:23", "45s", or "closing" once the time is up (ext)
pub fn left(ms: i64) -> String {
  let s = ms.saturating_add(999).div_euclid(1000);
  match s {
    ..=0 => t("mod.pred.closing").into(),
    1..=59 => format!("{s}s"),
    _ => format!("{}:{:02}", s / 60, s % 60),
  }
}

// 1500 -> "1.5K", 2000000 -> "2M" (ext formatPoints)
pub fn points(n: u64) -> String {
  let (div, unit) = if n >= 1_000_000 {
    (1_000_000, "M")
  } else if n >= 1000 {
    (1000, "K")
  } else {
    return n.to_string();
  };
  let tenths = n / (div / 10);
  if tenths.is_multiple_of(10) {
    format!("{}{unit}", tenths / 10)
  } else {
    format!("{}.{}{unit}", tenths / 10, tenths % 10)
  }
}

pub fn bettors(n: u64) -> String {
  tn("mod.pred.bettors", n, &[("count", &points(n))])
}

// the share bar: `n` of BAR cells filled
pub fn bar(n: u64) -> String {
  let n = (n as usize).min(BAR);
  format!("{}{}", "█".repeat(n), "░".repeat(BAR - n))
}

// the banner (ext updateChatBanners): shows while bets are open or locked. label, then
// "title · a 45% vs b 55%", then the countdown or "locked"
pub struct Banner {
  pub body: String,
  pub tail: String,
}

pub fn banner(p: &Pred, now: i64) -> Option<Banner> {
  if !p.live() {
    return None;
  }
  let parts: Vec<String> = p.outcomes.iter().enumerate().map(|(i, o)| format!("{} {}%", o.title, p.pct(i))).collect();
  Some(Banner { body: format!("{} · {}", p.title, parts.join(t("mod.pred.vs"))), tail: p.when(now) })
}

// what we know about one channel's prediction and when to ask again
#[derive(Default)]
pub struct Entry {
  // None until twitch has answered once
  pub pred: Option<Option<Pred>>,
  failed: u32,
  asked: Option<Instant>,
  inflight: bool,
}

impl Entry {
  fn gap(&self) -> Duration {
    if self.failed > 0 {
      IDLE_EVERY.saturating_mul(1 << self.failed.min(4)).min(BACKOFF_MAX)
    } else if self.pred.as_ref().is_some_and(|p| p.as_ref().is_some_and(Pred::live)) {
      LIVE_EVERY
    } else {
      IDLE_EVERY
    }
  }

  // when to ask next: now if never asked, never while a request is out
  fn next(&self, now: Instant) -> Option<Instant> {
    match self.asked {
      None => Some(now),
      Some(at) if self.inflight => (now.duration_since(at) >= LOST).then_some(now),
      Some(at) => Some(at + self.gap()),
    }
  }

  // twitch hasn't made sense lately: what shows may be old
  pub fn failed(&self) -> bool {
    self.failed > 0
  }

  // what to show at `now`: the prediction, if there is one worth showing
  pub fn shown(&self, now_ms: i64) -> Option<&Pred> {
    self.pred.as_ref()?.as_ref().filter(|p| p.fresh(now_ms))
  }
}

#[derive(Default)]
pub struct Preds {
  map: HashMap<Box<str>, Entry>,
  // per channel: the id of the prediction last seen (None = none), present once the channel has been asked
  told: HashMap<Box<str>, Option<Box<str>>>,
}

impl Preds {
  pub fn get(&self, chan: &str) -> Option<&Entry> {
    self.map.get(chan)
  }

  // when `chan` is next due (None = nothing to watch)
  pub fn due(&self, chan: &str, now: Instant) -> Option<Instant> {
    self.map.get(chan).map_or(Some(now), |e| e.next(now))
  }

  pub fn asked(&mut self, chan: &str, now: Instant) {
    let e = self.map.entry(chan.into()).or_default();
    (e.asked, e.inflight) = (Some(now), true);
  }

  // the ext's maybeBroadcastNewPrediction: the words of a `new prediction up` line when this answer shows a new
  // OPEN prediction, never for the first answer of a channel (a tab opened mid-prediction is not news)
  pub fn starts(&mut self, chan: &str, p: Option<&Pred>) -> Option<String> {
    let id = p.map(|p| p.id.clone());
    let seen = self.told.contains_key(chan);
    if self.told.get(chan) == Some(&id) {
      return None;
    }
    self.told.insert(chan.into(), id);
    let p = p.filter(|p| seen && p.state == State::Active)?;
    Some(f("mod.pred.new", &[("title", &p.title)]))
  }

  // an answer came (Err = twitch didn't make sense)
  pub fn got(&mut self, chan: &str, res: Result<Option<Pred>, ()>) {
    let e = self.map.entry(chan.into()).or_default();
    e.inflight = false;
    match res {
      Ok(p) => (e.pred, e.failed) = (Some(p), 0),
      Err(()) => e.failed = e.failed.saturating_add(1),
    }
  }

  // ask again now (r), unless a request is already out
  pub fn expire(&mut self, chan: &str) {
    if let Some(e) = self.map.get_mut(chan).filter(|e| !e.inflight) {
      e.asked = None;
    }
  }

  // forget channels no tab holds any more
  pub fn keep(&mut self, open: impl Fn(&str) -> bool) {
    self.map.retain(|c, _| open(c));
    self.told.retain(|c, _| open(c));
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  const NOW: i64 = 1_790_000_000_000;

  fn ev(status: &str, created: &str) -> Value {
    json!({"id": "e1", "title": "Who\nwins?", "status": status, "createdAt": created, "endedAt": null,
      "predictionWindowSeconds": 120, "winningOutcome": null,
      "outcomes": [{"id": "a", "title": "yes", "totalPoints": 3000, "totalUsers": 4, "color": "BLUE"},
                   {"id": "b", "title": "no", "totalPoints": 1000, "totalUsers": 1, "color": "PINK"}]})
  }

  fn answer(active: Value, locked: Value, resolved: Value) -> Value {
    json!({"data": {"user": {"channel": {"activePredictionEvents": active, "lockedPredictionEvents": locked,
      "resolvedPredictionEvents": {"edges": resolved}}}}})
  }

  fn iso(ms: i64) -> String {
    use crate::util::time::{utc_day, utc_stamp};
    format!("{}T{}Z", utc_day(ms), &utc_stamp(ms)[6..])
  }

  #[test]
  fn reads_an_open_prediction() {
    let v = answer(json!([ev("ACTIVE", &iso(NOW - 30_000))]), json!([]), json!([]));
    let p = parse(&v, NOW).unwrap().unwrap();
    assert_eq!((&*p.title, p.state, p.window), ("Who wins?", State::Active, 120));
    assert_eq!((p.points(), p.users(), p.pct(0), p.pct(1)), (4000, 5, 75, 25));
    assert_eq!((p.outcomes[0].ink, p.outcomes[1].ink), (theme::WHITE, theme::MAGENTA));
    assert_eq!(p.when(NOW), "1:30");
    // the clock changes when it crosses a whole second
    assert_eq!(p.tick_in(NOW), Some(1000));
    assert_eq!(p.tick_in(NOW + 1), Some(999));
    assert_eq!(p.tick_in(NOW - 1), Some(1));
    assert_eq!(p.tick_in(NOW + 90_000), None, "closing");
    assert_eq!((bar(p.share(0, BAR as u64)).chars().filter(|c| *c == '█').count()), 15);
  }

  #[test]
  fn priority_is_open_then_locked_then_lately_ended() {
    let open = ev("ACTIVE", &iso(NOW));
    let lock = ev("LOCKED", &iso(NOW));
    let mut done = ev("RESOLVED", &iso(NOW - 400_000));
    done["endedAt"] = json!(iso(NOW - 100_000));
    done["winningOutcome"] = json!({"id": "b"});
    let edge = json!([{"node": done.clone()}]);
    let state = |v: Value| parse(&v, NOW).unwrap().map(|p| p.state);
    assert_eq!(state(answer(json!([open]), json!([lock.clone()]), edge.clone())), Some(State::Active));
    assert_eq!(state(answer(json!([]), json!([lock]), edge.clone())), Some(State::Locked));
    assert_eq!(state(answer(json!([]), json!([]), edge.clone())), Some(State::Resolved));
    let p = parse(&answer(json!([]), json!([]), edge), NOW).unwrap().unwrap();
    assert!(p.outcomes[1].won && !p.outcomes[0].won);
    // ended more than 5 minutes ago: nothing
    done["endedAt"] = json!(iso(NOW - 301_000));
    assert_eq!(state(answer(json!([]), json!([]), json!([{"node": done}]))), None);
    assert_eq!(state(answer(json!([]), json!([]), json!([]))), None);
  }

  #[test]
  fn hostile_shapes_read_as_nothing_or_junk() {
    // junk: ask again later
    for v in [
      json!("x"),
      json!(null),
      json!({"errors": [{"message": "no"}]}),
      json!({"data": {}}),
      json!({"data": {"user": {}}}),
    ] {
      assert!(parse(&v, NOW).is_none(), "{v}");
    }
    // no such login: nothing to show
    assert_eq!(parse(&json!({"data": {"user": null}}), NOW), Some(None));
    let t = iso(NOW);
    let mut bad = vec![];
    let mut e = ev("ACTIVE", &t);
    e["status"] = json!("WEIRD");
    bad.push(e);
    let mut e = ev("ACTIVE", &t);
    e["createdAt"] = json!("yesterday");
    bad.push(e);
    let mut e = ev("ACTIVE", &t);
    e["outcomes"] = json!([{"title": "only", "totalPoints": 1}]);
    bad.push(e);
    let mut e = ev("ACTIVE", &t);
    e["id"] = json!("x".repeat(500));
    bad.push(e);
    let mut e = ev("ACTIVE", &t);
    e["outcomes"] = json!("nope");
    bad.push(e);
    for e in bad {
      assert_eq!(parse(&answer(json!([e.clone()]), json!([]), json!([])), NOW), Some(None), "{e}");
    }
    // bounds: titles cut and cleaned, 10 outcomes at most, numbers saturate, junk numbers are 0
    let mut e = ev("ACTIVE", &t);
    e["title"] = json!(format!("a\u{202e}b\u{7}{}", "z".repeat(1000)));
    e["predictionWindowSeconds"] = json!(u64::MAX);
    e["outcomes"] = json!((0..50)
      .map(|i| json!({"id": format!("o{i}"), "title": format!("o{i}"), "totalPoints": u64::MAX, "totalUsers": "many", "color": 7}))
      .collect::<Vec<_>>());
    let p = parse(&answer(json!([e]), json!([]), json!([])), NOW).unwrap().unwrap();
    assert!(p.title.starts_with("ab z") && p.title.chars().count() == MAX_TITLE);
    assert_eq!((p.outcomes.len(), p.window, p.points(), p.users()), (MAX_OUTCOMES, 1800, u64::MAX, 0));
    assert!(p.pct(0) <= 100 && p.outcomes[0].ink == theme::WHITE);
    // nothing bet yet: no division by zero
    let mut z = ev("ACTIVE", &t);
    z["outcomes"][0]["totalPoints"] = json!(0);
    z["outcomes"][1]["totalPoints"] = json!(0);
    let p = parse(&answer(json!([z]), json!([]), json!([])), NOW).unwrap().unwrap();
    assert_eq!((p.pct(0), p.share(1, 20)), (0, 0));
  }

  #[test]
  fn time_left_and_points_read_like_the_ext() {
    assert_eq!(left(83_000), "1:23");
    assert_eq!(left(83_001), "1:24");
    assert_eq!(left(45_000), "45s");
    assert_eq!(left(60_000), "1:00");
    assert_eq!((left(0), left(-5000), left(i64::MIN)), ("closing".into(), "closing".into(), "closing".into()));
    assert_eq!(
      (points(999), points(1000), points(1500), points(2_000_000), points(1_250_000)),
      ("999".into(), "1K".into(), "1.5K".into(), "2M".into(), "1.2M".into())
    );
    assert_eq!((bettors(1), bettors(2500)), ("1 bettor".into(), "2.5K bettors".into()));
    assert_eq!(bar(0).chars().count(), BAR);
    assert_eq!(bar(99).chars().filter(|c| *c == '█').count(), BAR);
  }

  #[test]
  fn the_banner_shows_only_while_bets_are_open_or_locked() {
    let v = answer(json!([ev("ACTIVE", &iso(NOW - 30_000))]), json!([]), json!([]));
    let mut p = parse(&v, NOW).unwrap().unwrap();
    let b = banner(&p, NOW).unwrap();
    assert_eq!((b.body.as_str(), b.tail.as_str()), ("Who wins? · yes 75% vs no 25%", "1:30"));
    assert_eq!(banner(&p, NOW + 200_000).unwrap().tail, "closing");
    p.state = State::Locked;
    assert_eq!(banner(&p, NOW).unwrap().tail, "locked");
    p.state = State::Resolved;
    assert!(banner(&p, NOW).is_none());
    assert_eq!(p.when(NOW), "ended");
    p.state = State::Canceled;
    assert_eq!(p.when(NOW), "refunded");
  }

  #[test]
  fn asking_follows_the_ext_and_backs_off() {
    let t0 = Instant::now();
    let mut ps = Preds::default();
    // never asked: now
    assert_eq!(ps.due("a", t0), Some(t0));
    ps.asked("a", t0);
    assert_eq!(ps.due("a", t0 + Duration::from_secs(1)), None);
    // one in flight: not again (until it is lost)
    assert_eq!(ps.due("a", t0 + Duration::from_secs(5)), None);
    assert_eq!(ps.due("a", t0 + LOST), Some(t0 + LOST));
    // nothing live: the slow look
    ps.got("a", Ok(None));
    assert_eq!(ps.due("a", t0), Some(t0 + IDLE_EVERY));
    // one live: 15s
    let v = answer(json!([ev("ACTIVE", &iso(NOW))]), json!([]), json!([]));
    ps.asked("a", t0);
    ps.got("a", Ok(parse(&v, NOW).unwrap()));
    assert_eq!(ps.due("a", t0), Some(t0 + LIVE_EVERY));
    assert!(ps.get("a").unwrap().shown(NOW).is_some());
    // failures back off, capped; success resets
    for _ in 0..10 {
      ps.asked("a", t0);
      ps.got("a", Err(()));
    }
    assert_eq!(ps.due("a", t0), Some(t0 + BACKOFF_MAX));
    ps.asked("a", t0);
    ps.got("a", Ok(None));
    assert_eq!(ps.due("a", t0), Some(t0 + IDLE_EVERY));
    // r: ask now, unless out
    ps.expire("a");
    assert_eq!(ps.due("a", t0), Some(t0));
    ps.asked("a", t0);
    ps.expire("a");
    assert_eq!(ps.due("a", t0 + Duration::from_secs(1)), None);
    // closed tabs are forgotten
    ps.keep(|c| c == "b");
    assert!(ps.get("a").is_none());
  }
}
