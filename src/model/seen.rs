// first-chatter glow bookkeeping: who has spoken in this tab this session.
// ext: per-channel set, 5000 names, the oldest 1000 drop when it overflows
use std::collections::{HashSet, VecDeque};

const CAP: usize = 5000;
const DROP: usize = 1000;

#[derive(Default)]
pub struct Seen {
  set: HashSet<Box<str>>,
  order: VecDeque<Box<str>>,
}

impl Seen {
  // true the first time a (case-insensitive) name shows up
  pub fn first(&mut self, name: &str) -> bool {
    if name.is_empty() {
      return false;
    }
    let key: Box<str> = name.to_ascii_lowercase().into();
    if self.set.contains(&key) {
      return false;
    }
    self.order.push_back(key.clone());
    self.set.insert(key);
    if self.set.len() > CAP {
      for old in self.order.drain(..DROP) {
        self.set.remove(&old);
      }
    }
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn once_per_name_any_case() {
    let mut s = Seen::default();
    assert!(s.first("Bob"));
    assert!(!s.first("bob"));
    assert!(!s.first(""));
    assert!(s.first("al"));
  }

  #[test]
  fn bounded_and_oldest_drop_first() {
    let mut s = Seen::default();
    for i in 0..=CAP {
      s.first(&format!("u{i}"));
    }
    assert_eq!(s.set.len(), CAP + 1 - DROP);
    assert!(s.first("u0"));
    assert!(!s.first(&format!("u{CAP}")));
  }
}
