// shared chat's partner rooms by number (ext resolveTwitchLoginById): the `shared:name` chip says which
// channel a line was said in. the rooms you joined are known from their ROOMSTATE; any other is asked
// once from twitch's public web gql and kept in a 500-entry cache (the ext's cap). an ask that failed is
// not repeated, so a session that never resolves costs one request
use std::collections::{HashMap, VecDeque};

use crate::model::ident;

const MAX: usize = 500;
// asks waiting to go out between two frames
const MAX_WANTED: usize = 32;
// a numeric room id: digits, a dozen or so
const MAX_ID: usize = 20;

// a room id we will put in a request
pub fn valid(id: &str) -> bool {
  (1..=MAX_ID).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Default)]
pub struct Rooms {
  // id → login; empty = asked and not found
  names: HashMap<Box<str>, Box<str>>,
  order: VecDeque<Box<str>>,
  wanted: Vec<Box<str>>,
}

impl Rooms {
  // the channel a room id is. an unknown id queues one ask and says nothing yet
  pub fn login(&mut self, id: &str) -> Option<&str> {
    if !self.names.contains_key(id) {
      if !valid(id) || self.wanted.len() >= MAX_WANTED {
        return None;
      }
      self.insert(id, "");
      self.wanted.push(id.into());
    }
    self.names.get(id).map(|n| &**n).filter(|n| !n.is_empty())
  }

  pub fn take_wanted(&mut self) -> Vec<Box<str>> {
    std::mem::take(&mut self.wanted)
  }

  // a joined room's id, from its ROOMSTATE. true when a chip's words may change
  pub fn seat(&mut self, id: &str, login: &str) -> bool {
    self.put(id, Some(login))
  }

  // an answer (None = twitch knows no such room). hostile like everything off the wire: only a numeric id
  // and a plain login are kept. true when a chip's words may change
  pub fn put(&mut self, id: &str, login: Option<&str>) -> bool {
    if !valid(id) {
      return false;
    }
    let name = login.filter(|l| ident::chatter_login(l)).map(str::to_ascii_lowercase).unwrap_or_default();
    // a failed ask never takes a known name away
    if name.is_empty() && self.names.get(id).is_some_and(|n| !n.is_empty()) {
      return false;
    }
    let changed = !name.is_empty() && self.names.get(id).is_none_or(|n| **n != *name);
    self.insert(id, &name);
    changed
  }

  fn insert(&mut self, id: &str, name: &str) {
    if let Some(n) = self.names.get_mut(id) {
      *n = name.into();
      return;
    }
    if self.names.len() >= MAX
      && let Some(old) = self.order.pop_front()
    {
      self.names.remove(&old);
    }
    self.order.push_back(id.into());
    self.names.insert(id.into(), name.into());
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_unknown_room_is_asked_once_and_named_when_it_answers() {
    let mut r = Rooms::default();
    assert_eq!(r.login("1234"), None);
    assert_eq!(r.login("1234"), None, "asking again queues nothing");
    assert_eq!(r.take_wanted(), vec![Box::<str>::from("1234")]);
    assert!(r.put("1234", Some("Partner")));
    assert_eq!(r.login("1234"), Some("partner"));
    assert!(r.take_wanted().is_empty());
  }

  #[test]
  fn a_joined_room_needs_no_ask() {
    let mut r = Rooms::default();
    assert!(r.seat("77", "forsen"));
    assert_eq!(r.login("77"), Some("forsen"));
    assert!(r.take_wanted().is_empty());
  }

  #[test]
  fn a_failed_ask_is_not_repeated_and_never_unnames_a_room() {
    let mut r = Rooms::default();
    r.login("5");
    assert!(!r.put("5", None));
    assert_eq!(r.login("5"), None);
    assert_eq!(r.take_wanted().len(), 1, "one ask, not two");
    r.seat("6", "xqc");
    assert!(!r.put("6", None));
    assert_eq!(r.login("6"), Some("xqc"));
  }

  #[test]
  fn hostile_ids_and_names_are_dropped() {
    let mut r = Rooms::default();
    for id in ["", "12x", "../../x", &"9".repeat(21), "1 2"] {
      assert_eq!(r.login(id), None, "{id}");
      assert!(!r.put(id, Some("bob")), "{id}");
    }
    assert!(r.take_wanted().is_empty());
    r.login("8");
    assert!(!r.put("8", Some("bad name\n")));
    assert!(!r.put("8", Some(&"a".repeat(51))));
    assert_eq!(r.login("8"), None);
  }

  #[test]
  fn the_cache_is_bounded_and_the_oldest_go_first() {
    let mut r = Rooms::default();
    for i in 0..MAX + 10 {
      r.seat(&i.to_string(), "bob");
    }
    assert_eq!(r.names.len(), MAX);
    assert!(!r.names.contains_key("0") && r.names.contains_key((MAX + 9).to_string().as_str()));
  }

  #[test]
  fn a_burst_of_new_rooms_queues_a_bounded_number_of_asks() {
    let mut r = Rooms::default();
    for i in 0..1000 {
      r.login(&i.to_string());
    }
    assert_eq!(r.take_wanted().len(), MAX_WANTED);
  }
}
