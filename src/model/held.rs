// automod holds (ext automod-queue.js): twitch's automod paused a viewer's message in a room you
// moderate. heatsync's server watches the room for you and pushes each hold (`automod:hold`) and its end
// (`automod:update`) over the socket; the watch call hands back what is still held. the row offers
// allow / deny until a mod settles it or twitch lets it lapse an hour after holding it.
// every frame is hostile: ids are checked, words bounded, control chars dropped
use super::ident;
use crate::i18n::{f, t};
use std::collections::HashMap;

use serde_json::Value;

use crate::model::message::{Extra, Kind, Message, Platform, Seg};
use crate::ui::theme;
use crate::util::time::parse_iso;

// twitch denies a held message on its own an hour after holding it; allow / deny is dead past that
pub const EXPIRE_MS: i64 = 60 * 60_000;
// the same hold can come twice (socket replay, the watch's backlog): one row per id
const SEEN_MS: i64 = 2 * EXPIRE_MS;
const SEEN_CAP: usize = 2000;
const MAX_ID: usize = 200;
const MAX_NAME: usize = 100;
const MAX_TEXT: usize = 2000;
const MAX_TERMS: usize = 10;
// a backlog bigger than this is junk
pub const MAX_PENDING: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hold {
  pub id: Box<str>,
  // the room (twitch login) it was held in
  pub chan: Box<str>,
  pub sender_id: Box<str>,
  pub sender_login: Box<str>,
  pub sender_name: Box<str>,
  pub text: Box<str>,
  pub held_at: i64,
  // why: `blocked term: x`, `swearing (level 2)`, `automod`
  pub why: Box<str>,
}

// how a settled hold ended
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
  Allowed(Option<Box<str>>),
  Denied(Option<Box<str>>),
  Expired,
}

impl End {
  pub fn text(&self) -> String {
    match self {
      End::Allowed(Some(m)) => f("chat1.held.allowed_by", &[("mod", m)]),
      End::Allowed(None) => t("chat1.held.allowed").into(),
      End::Denied(Some(m)) => f("chat1.held.denied_by", &[("mod", m)]),
      End::Denied(None) => t("chat1.held.denied").into(),
      End::Expired => t("chat1.held.expired").into(),
    }
  }
}

// where a hold's row stands
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
  // allow / deny
  Open,
  // asked, no answer yet
  Busy,
  // the ask failed: why, and allow / deny again
  Failed(Box<str>),
  Ended(End),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
  pub id: Box<str>,
  pub held_at: i64,
  pub state: State,
}

impl Row {
  // allow / deny can be pressed
  pub fn open(&self) -> bool {
    matches!(self.state, State::Open | State::Failed(_))
  }
}

// the hold as a line of its room's chat (ext .hs-mc-automod): `automod hold` and why on top, the
// sender's words under them, the buttons under those; no message id, so it can't be replied to or deleted
pub fn line(h: &Hold) -> Message {
  let mut m = Message::system(h.held_at, Platform::Twitch, h.text.clone());
  m.kind = Kind::Chat;
  m.login = h.sender_login.clone();
  m.user_id = h.sender_id.clone();
  m.nick = h.sender_name.clone();
  m.color = theme::WHITE;
  let seg = |t: &str, bold| Seg { text: t.into(), ink: theme::YELLOW, bold };
  m.x = Some(Box::new(Extra {
    chan: "".into(),
    head: Box::new([seg(t("chat1.held.head"), true), seg(&format!("  {}\n", h.why), false)]),
    edge: theme::YELLOW,
    held: Some(Box::new(Row { id: h.id.clone(), held_at: h.held_at, state: State::Open })),
    ..Extra::default()
  }));
  m
}

fn word(v: &Value, k: &str, max: usize) -> String {
  v.get(k)
    .and_then(Value::as_str)
    .unwrap_or("")
    .chars()
    .map(|c| if c.is_control() { ' ' } else { c })
    .take(max)
    .collect::<String>()
    .trim()
    .to_owned()
}

// a message id: what twitch hands out (a uuid), nothing that could break a url or a line
fn valid_id(s: &str) -> bool {
  ident::slug(s, MAX_ID)
}

// a twitch login
fn login(v: &Value, k: &str) -> Option<Box<str>> {
  let s = v.get(k).and_then(Value::as_str)?.to_ascii_lowercase();
  ident::twitch_login(&s).then(|| s.into())
}

fn num_id(v: &Value, k: &str) -> Box<str> {
  let s = v.get(k).and_then(Value::as_str).unwrap_or("");
  if (1..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) { s.into() } else { "".into() }
}

fn why(v: &Value) -> Box<str> {
  if v.get("reason").and_then(Value::as_str) == Some("blocked_term") {
    let term = v
      .get("terms")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .take(MAX_TERMS)
      .filter_map(Value::as_str)
      .map(|t| crate::util::strip(t).take(MAX_NAME).collect::<String>())
      .find(|t| !t.trim().is_empty());
    return term.map_or_else(
      || t("chat1.held.blocked_term").into(),
      |w| f("chat1.held.blocked_term_is", &[("term", w.trim())]).into(),
    );
  }
  let cat = word(v, "category", MAX_NAME);
  let level = v.get("level").and_then(Value::as_u64).filter(|l| (1..=4).contains(l));
  match (cat.is_empty(), level) {
    (true, _) => t("chat1.held.automod").into(),
    (false, Some(l)) => f("chat1.held.level", &[("cat", &cat), ("level", &l.to_string())]).into(),
    (false, None) => cat.into(),
  }
}

// `automod:hold` (and each of the watch's `pending`): None when it can't be shown or acted on
pub fn hold(v: &Value, now: i64) -> Option<Hold> {
  let id = v.get("msgId").and_then(Value::as_str).filter(|s| valid_id(s))?;
  let chan = login(v, "broadcasterLogin")?;
  let sender_login = login(v, "senderLogin").unwrap_or_default();
  let name = word(v, "senderName", MAX_NAME);
  let sender_name: Box<str> = if name.is_empty() {
    if sender_login.is_empty() { "unknown".into() } else { sender_login.clone() }
  } else {
    name.into()
  };
  // an iso stamp from the server (an epoch number is taken too); never later than now
  let held_at = match v.get("heldAt") {
    Some(Value::String(s)) => parse_iso(s),
    Some(n) => n.as_i64(),
    None => None,
  }
  .filter(|t| *t > 0)
  .map_or(now, |t| t.min(now));
  Some(Hold {
    id: id.into(),
    chan,
    sender_id: num_id(v, "senderId"),
    sender_login,
    sender_name,
    text: word(v, "text", MAX_TEXT).into(),
    held_at,
    why: why(v),
  })
}

// `automod:update`: (room, id, how it ended)
pub fn update(v: &Value) -> Option<(Box<str>, Box<str>, End)> {
  let id = v.get("msgId").and_then(Value::as_str).filter(|s| valid_id(s))?;
  let chan = login(v, "broadcasterLogin")?;
  let by = login(v, "modLogin");
  let end = match v.get("status").and_then(Value::as_str) {
    Some("approved") => End::Allowed(by),
    Some("denied") => End::Denied(by),
    _ => End::Expired,
  };
  Some((chan, id.into(), end))
}

// the watch's answer: what is still held there
pub fn pending(v: &Value, now: i64) -> Vec<Hold> {
  v.get("pending")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .take(MAX_PENDING)
    .filter_map(|h| hold(h, now))
    .collect()
}

// which holds have a row, and which were settled (a backlog that missed the settling must not bring
// one back as actionable)
#[derive(Default)]
pub struct Seen {
  by: HashMap<Box<str>, (i64, bool)>,
}

impl Seen {
  // true the first time a live, unsettled hold is seen: it gets a row
  pub fn first(&mut self, h: &Hold, now: i64) -> bool {
    if now - h.held_at >= EXPIRE_MS || self.by.contains_key(&h.id) {
      return false;
    }
    self.put(&h.id, now, false);
    true
  }

  pub fn settled(&mut self, id: &str, now: i64) {
    self.put(id, now, true);
  }

  fn put(&mut self, id: &str, now: i64, settled: bool) {
    if self.by.len() >= SEEN_CAP {
      self.by.retain(|_, e| now - e.0 < SEEN_MS);
      if self.by.len() >= SEEN_CAP
        && let Some(k) = self.by.iter().min_by_key(|e| e.1.0).map(|e| e.0.clone())
      {
        self.by.remove(&k);
      }
    }
    let e = self.by.entry(id.into()).or_insert((now, false));
    e.1 |= settled;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn frame() -> Value {
    json!({"type": "automod:hold", "broadcasterId": "22484632", "broadcasterLogin": "Forsen",
      "msgId": "8f0c3b6e-1d2a-4c1e-9b7a-2f3e4d5c6b7a", "senderId": "77", "senderLogin": "bob",
      "senderName": "Bob", "text": "hello\nthere", "heldAt": "2026-11-03T12:00:00Z",
      "reason": "automod", "category": "swearing", "level": 2, "terms": null})
  }

  #[test]
  fn a_hold_reads_like_the_ext() {
    let now = parse_iso("2026-11-03T12:05:00Z").unwrap();
    let h = hold(&frame(), now).unwrap();
    assert_eq!((&*h.id, &*h.chan, &*h.sender_id), ("8f0c3b6e-1d2a-4c1e-9b7a-2f3e4d5c6b7a", "forsen", "77"));
    assert_eq!((&*h.sender_name, &*h.text, &*h.why), ("Bob", "hello there", "swearing (level 2)"));
    assert_eq!(h.held_at, now - 5 * 60_000);
    let mut v = frame();
    v["reason"] = json!("blocked_term");
    v["terms"] = json!(["", "badword", "x"]);
    assert_eq!(&*hold(&v, now).unwrap().why, "blocked term: badword");
    v["terms"] = json!(null);
    assert_eq!(&*hold(&v, now).unwrap().why, "blocked term");
    v["reason"] = json!("automod");
    v["category"] = json!(null);
    assert_eq!(&*hold(&v, now).unwrap().why, "automod");
  }

  #[test]
  fn hostile_holds_are_bounded_or_dropped() {
    let now = 1_000_000;
    for (k, bad) in [
      ("msgId", json!("a b")),
      ("msgId", json!("x".repeat(201))),
      ("msgId", json!(5)),
      ("broadcasterLogin", json!("no spaces allowed")),
      ("broadcasterLogin", json!(null)),
    ] {
      let mut v = frame();
      v[k] = bad;
      assert!(hold(&v, now).is_none(), "{k}");
    }
    let mut v = frame();
    v["text"] = json!("y".repeat(5000));
    v["senderName"] = json!("");
    v["senderId"] = json!("12ab");
    v["heldAt"] = json!("in the future");
    v["level"] = json!(99);
    let h = hold(&v, now).unwrap();
    assert_eq!(h.text.len(), MAX_TEXT);
    assert_eq!((&*h.sender_name, &*h.sender_id, &*h.why, h.held_at), ("bob", "", "swearing", now));
    // a stamp from the future is now
    v["heldAt"] = json!(now + 99);
    assert_eq!(hold(&v, now).unwrap().held_at, now);
  }

  #[test]
  fn updates_and_backlogs() {
    let u = json!({"broadcasterLogin": "forsen", "msgId": "abc", "status": "approved", "modLogin": "Mod1"});
    assert_eq!(update(&u), Some(("forsen".into(), "abc".into(), End::Allowed(Some("mod1".into())))));
    let u = json!({"broadcasterLogin": "forsen", "msgId": "abc", "status": "denied", "modLogin": null});
    assert_eq!(update(&u).unwrap().2.text(), "denied");
    let u = json!({"broadcasterLogin": "forsen", "msgId": "abc", "status": "weird"});
    assert_eq!(update(&u).unwrap().2, End::Expired);
    assert!(update(&json!({"broadcasterLogin": "forsen"})).is_none());
    let many: Vec<Value> = (0..150).map(|_| frame()).collect();
    assert_eq!(pending(&json!({"ok": true, "pending": many}), 1).len(), MAX_PENDING);
    assert!(pending(&json!({"ok": true}), 1).is_empty());
  }

  #[test]
  fn one_row_per_hold_and_settled_ones_stay_settled() {
    let now = parse_iso("2026-11-03T12:05:00Z").unwrap();
    let h = hold(&frame(), now).unwrap();
    let mut s = Seen::default();
    assert!(s.first(&h, now));
    assert!(!s.first(&h, now + 1), "a replay");
    let other = Hold { id: "other".into(), ..h.clone() };
    s.settled(&other.id, now);
    assert!(!s.first(&other, now + 2), "settled before its row: the backlog must not bring it back");
    let old = Hold { id: "old".into(), held_at: now - EXPIRE_MS, ..h };
    assert!(!s.first(&old, now), "twitch already let it go");
  }
}
