// which messages heatsync says went through heatsync, for anyone's lines: ids per (platform, channel) from the
// seed (`GET /api/chat/origins`) and from `chat:origin` pushes, plus the sends already on screen so one send to
// several feeds reads as one line. keyed by message id, never by words
use std::collections::{HashMap, VecDeque};

use super::message::Platform;

const ROOMS: usize = 256;
const IDS: usize = 4000;
const SENDS: usize = 512;

// a room's key as both ends spell it: lowercase, no youtube `@`
pub fn room(chan: &str) -> Box<str> {
  chan.trim_start_matches('@').to_ascii_lowercase().into()
}

#[derive(Default)]
struct Ids {
  map: HashMap<Box<str>, Box<str>>,
  order: VecDeque<Box<str>>,
}

#[derive(Default)]
pub struct Heat {
  rooms: HashMap<(Platform, Box<str>), Ids>,
}

impl Heat {
  // `sid` is the send id, empty when the server gave none
  pub fn note(&mut self, p: Platform, chan: &str, id: &str, sid: &str) {
    let key = (p, room(chan));
    if !self.rooms.contains_key(&key) && self.rooms.len() >= ROOMS {
      return;
    }
    let r = self.rooms.entry(key).or_default();
    if r.map.insert(id.into(), sid.into()).is_none() {
      r.order.push_back(id.into());
      if r.order.len() > IDS
        && let Some(old) = r.order.pop_front()
      {
        r.map.remove(&old);
      }
    }
  }

  // Some(send id, "" when none) when `id` is a heatsync line in that room
  pub fn get(&self, p: Platform, chan: &str, id: &str) -> Option<&str> {
    self.rooms.get(&(p, room(chan)))?.map.get(id).map(|s| &**s)
  }
}

// the first platform to show a row of each send. a later row of the same send from another platform folds
#[derive(Default)]
pub struct Sends {
  seen: HashMap<Box<str>, Platform>,
  order: VecDeque<Box<str>>,
}

impl Sends {
  // true = show this row (first of its send, or the same platform again); false = another platform's copy
  pub fn claim(&mut self, sid: &str, p: Platform) -> bool {
    if sid.is_empty() {
      return true;
    }
    if let Some(q) = self.seen.get(sid) {
      return *q == p;
    }
    self.seen.insert(sid.into(), p);
    self.order.push_back(sid.into());
    if self.order.len() > SENDS
      && let Some(old) = self.order.pop_front()
    {
      self.seen.remove(&old);
    }
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ids_are_found_by_room_and_bounded() {
    let mut h = Heat::default();
    h.note(Platform::Youtube, "@MrBeast", "a", "s1");
    assert_eq!(h.get(Platform::Youtube, "mrbeast", "a"), Some("s1"), "the @ and the case do not matter");
    assert_eq!(h.get(Platform::Twitch, "mrbeast", "a"), None, "another platform's room");
    for i in 0..IDS + 10 {
      h.note(Platform::Twitch, "x", &i.to_string(), "");
    }
    assert!(
      h.get(Platform::Twitch, "x", "0").is_none() && h.get(Platform::Twitch, "x", &(IDS + 9).to_string()).is_some()
    );
  }

  #[test]
  fn one_send_shows_on_one_platform() {
    let mut s = Sends::default();
    assert!(s.claim("S", Platform::Twitch));
    assert!(s.claim("S", Platform::Twitch), "the same platform's other tab");
    assert!(!s.claim("S", Platform::Kick), "the other feed's copy folds");
    assert!(s.claim("", Platform::Kick) && s.claim("", Platform::Youtube), "no send id never folds");
    assert!(s.claim("T", Platform::Kick), "a different send");
  }
}
