// which messages heatsync says went through heatsync, for anyone's lines: ids per (platform, channel) from the
// seed (`GET /api/chat/origins`) and from `chat:origin` pushes, plus the sends already on screen so one send to
// several feeds reads as one line. keyed by message id, never by words
use std::collections::{HashMap, VecDeque};

use super::message::Platform;

const ROOMS: usize = 256;
const IDS: usize = 4000;
const SENDS: usize = 512;

// a name without the one `@` youtube writes before a handle (the heatsync account stores it bare)
pub fn bare(name: &str) -> &str {
  name.strip_prefix('@').unwrap_or(name)
}

// a room's key as both ends spell it: lowercase, no youtube `@`
pub fn room(chan: &str) -> Box<str> {
  bare(chan).to_ascii_lowercase().into()
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
  // sends whose twitch copy was the one that folded: the row standing for each stands for twitch's leg too
  // (ChatView::lend_owed). newest kept, bounded: a tab nobody opens never settles them
  owed: VecDeque<Box<str>>,
}

const OWED: usize = 64;

impl Sends {
  // true = show this row (first of its send, or the same platform again); false = another platform's copy
  pub fn claim(&mut self, sid: &str, p: Platform) -> bool {
    if sid.is_empty() {
      return true;
    }
    if let Some(q) = self.seen.get(sid) {
      if *q != p && p == Platform::Twitch {
        if self.owed.len() == OWED {
          self.owed.pop_front();
        }
        self.owed.push_back(sid.into());
      }
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

  // the sends whose twitch copy folded since the last call, oldest first
  pub fn take_owed(&mut self) -> Vec<Box<str>> {
    self.owed.drain(..).collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // the row standing for a send reads in the twitch room once twitch's copy folded into it: only that fold is owed
  #[test]
  fn a_twitch_copy_that_folds_is_owed_and_no_other_fold_is() {
    let mut s = Sends::default();
    assert!(s.claim("S", Platform::Kick));
    assert!(!s.claim("S", Platform::Twitch) && !s.claim("S", Platform::Youtube));
    assert_eq!(s.take_owed(), [Box::<str>::from("S")]);
    assert!(s.take_owed().is_empty(), "taken once");
    assert!(s.claim("T", Platform::Twitch) && !s.claim("T", Platform::Kick) && s.claim("T", Platform::Twitch));
    assert!(s.take_owed().is_empty(), "twitch stands in its own room; kick's fold owes nothing");
    for i in 0..OWED + 10 {
      s.claim(&format!("k{i}"), Platform::Kick);
      s.claim(&format!("k{i}"), Platform::Twitch);
    }
    assert_eq!(s.take_owed().len(), OWED, "a tab nobody opens keeps the newest only");
  }

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
