// the channel's pinned line (ws `pin:set|clear`, `GET /api/pin`). hostile input: bounded + cleaned.
// "hide" is per pin id for this session, like the ext's sessionStorage mark: a new pin shows again
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::model::color;
use crate::model::message::Platform;
use crate::ui::theme;

const MAX_TEXT: usize = 500;
const MAX_NAME: usize = 64;
const MAX_ROOMS: usize = 64;
const MAX_HIDDEN: usize = 256;
const MAX_MSG_ID: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
  pub id: Box<str>,
  // the chat line it was made from (empty when the server didn't say): "is this row the pin"
  pub msg_id: Box<str>,
  pub nick: Box<str>,
  pub color: u32,
  pub text: Box<str>,
  pub at: i64,
}

fn clean(s: &str, max: usize) -> Box<str> {
  let mut out = String::with_capacity(s.len().min(max));
  for c in s.chars() {
    let c = if c.is_control() { ' ' } else { c };
    if out.len() + c.len_utf8() > max {
      break;
    }
    out.push(c);
  }
  out.trim().into()
}

pub fn parse(v: &Value) -> Option<Pin> {
  let id = match v.get("id")? {
    Value::String(s) => s.clone(),
    Value::Number(n) => n.to_string(),
    _ => return None,
  };
  if id.is_empty() || id.len() > 100 {
    return None;
  }
  let text = clean(v.get("content").and_then(Value::as_str)?, MAX_TEXT);
  if text.is_empty() {
    return None;
  }
  let login = v.get("username").and_then(Value::as_str).unwrap_or("");
  let nick = clean(v.get("display_name").and_then(Value::as_str).unwrap_or(login), MAX_NAME);
  let color = v.get("color").and_then(Value::as_str).and_then(color::parse_hex).map_or(theme::WHITE, color::readable);
  let at = v.get("pinned_at").and_then(Value::as_i64).unwrap_or(0);
  let msg_id = v.get("message_id").and_then(Value::as_str).filter(|m| m.len() <= MAX_MSG_ID).unwrap_or("");
  Some(Pin { id: id.into(), msg_id: msg_id.into(), nick, color, text, at })
}

#[derive(Default)]
pub struct Pins {
  rooms: HashMap<(u8, Box<str>), Pin>,
  hidden: HashSet<Box<str>>,
}

impl Pins {
  pub fn set(&mut self, p: Platform, key: &str, pin: Option<Pin>) {
    let k = (p as u8, Box::<str>::from(key));
    match pin {
      Some(pin) if self.rooms.len() < MAX_ROOMS || self.rooms.contains_key(&k) => {
        self.rooms.insert(k, pin);
      }
      Some(_) => {}
      None => {
        self.rooms.remove(&k);
      }
    }
  }

  // a room we left: its pin must not hold a slot
  pub fn forget(&mut self, p: Platform, key: &str) {
    self.rooms.remove(&(p as u8, Box::<str>::from(key)));
  }

  pub fn hide(&mut self, id: &str) {
    if self.hidden.len() >= MAX_HIDDEN {
      self.hidden.clear();
    }
    self.hidden.insert(id.into());
  }

  // the newest pin among a tab's feeds that the viewer hasn't hidden
  pub fn current<'a>(&'a self, sources: &[(Platform, Box<str>)]) -> Option<&'a Pin> {
    self.current_in(sources).map(|(_, _, pin)| pin)
  }

  // the same, with the room it lives in
  pub fn current_in(&self, sources: &[(Platform, Box<str>)]) -> Option<(Platform, Box<str>, &Pin)> {
    sources
      .iter()
      .filter(|(p, _)| matches!(p, Platform::Twitch | Platform::Kick))
      .filter_map(|(p, k)| self.rooms.get(&(*p as u8, k.clone())).map(|pin| (*p, k.clone(), pin)))
      .filter(|(_, _, pin)| !self.hidden.contains(&pin.id))
      .max_by_key(|(_, _, pin)| pin.at)
  }

  // the pin a room holds right now (hidden or not): what a row's menu compares its line to
  pub fn in_room(&self, p: Platform, key: &str) -> Option<&Pin> {
    self.rooms.get(&(p as u8, Box::<str>::from(key)))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn pin(id: &str, at: i64) -> Pin {
    parse(&json!({"id": id, "content": "hi", "username": "bob", "pinned_at": at})).unwrap()
  }

  #[test]
  fn parses_and_bounds() {
    let p =
      parse(&json!({"id": 7, "content": "a\nb\u{7}c", "username": "bob", "display_name": "Bob", "color": "#f00"}))
        .unwrap();
    assert_eq!((&*p.id, &*p.text, &*p.nick), ("7", "a b c", "Bob"));
    assert_eq!(&*p.msg_id, "");
    let m = parse(&json!({"id": "7", "content": "x", "message_id": "abc-1"})).unwrap();
    assert_eq!(&*m.msg_id, "abc-1");
    assert_eq!(&*parse(&json!({"id": "7", "content": "x", "message_id": "m".repeat(129)})).unwrap().msg_id, "");
    let long = "é".repeat(600);
    assert!(parse(&json!({"id": "x", "content": long})).unwrap().text.len() <= MAX_TEXT);
    assert!(parse(&json!({"id": "", "content": "x"})).is_none());
    assert!(parse(&json!({"id": "x", "content": "  "})).is_none());
    assert!(parse(&json!({"id": "x"})).is_none());
    assert!(parse(&json!({"id": ["x"], "content": "x"})).is_none());
    // a bad color falls back to white, never garbage
    assert_eq!(parse(&json!({"id": "x", "content": "x", "color": "red"})).unwrap().color, theme::WHITE);
  }

  #[test]
  fn newest_visible_wins_and_hide_is_per_id() {
    let mut s = Pins::default();
    let src = vec![(Platform::Twitch, "a".into()), (Platform::Kick, "b".into()), (Platform::Youtube, "c".into())];
    assert!(s.current(&src).is_none());
    s.set(Platform::Twitch, "a", Some(pin("1", 10)));
    s.set(Platform::Kick, "b", Some(pin("2", 20)));
    s.set(Platform::Youtube, "c", Some(pin("3", 30)));
    assert_eq!(&*s.current(&src).unwrap().id, "2");
    s.hide("2");
    assert_eq!(&*s.current(&src).unwrap().id, "1");
    s.set(Platform::Kick, "b", Some(pin("4", 40)));
    assert_eq!(&*s.current(&src).unwrap().id, "4");
    s.set(Platform::Kick, "b", None);
    s.set(Platform::Twitch, "a", None);
    assert!(s.current(&src).is_none());
  }

  #[test]
  fn a_left_room_frees_its_slot() {
    let mut s = Pins::default();
    let src = vec![(Platform::Twitch, "a".into())];
    s.set(Platform::Twitch, "a", Some(pin("1", 10)));
    s.forget(Platform::Twitch, "a");
    assert!(s.current(&src).is_none());
    assert!(s.rooms.is_empty());
  }
}
