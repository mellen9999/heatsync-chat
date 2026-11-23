// a sender's picks, as their own app told the room (heatsync's `emote:broadcast`, sent when a line with a
// pick leaves): "the KEKW in my line is this picture". a pick is the line's emote, over the room's of that
// name (resolve.rs). the line and its vouch travel apart (the platform's chat and the heatsync socket), so
// either can land first: a vouch waits here for its line, by the sender's lowercase login. one vouch is one
// send: the first line that names the word takes it. bounded in count and age; what never pairs expires
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::model::message::HsEmote;

// senders held at once, and picks waiting per sender
const CAP: usize = 500;
const PER: usize = 20;
// how long a vouch waits for its line (the site's and the ext's 90s)
const TTL: Duration = Duration::from_secs(90);

#[derive(Default)]
pub struct Vouches {
  map: HashMap<Box<str>, Vec<(Instant, HsEmote)>>,
}

impl Vouches {
  pub fn hold(&mut self, login: &str, pick: HsEmote) {
    self.hold_at(Instant::now(), login, pick);
  }

  fn hold_at(&mut self, now: Instant, login: &str, pick: HsEmote) {
    if self.map.len() >= CAP && !self.map.contains_key(login) {
      self.map.retain(|_, v| v.iter().any(|(t, _)| now.duration_since(*t) < TTL));
      if self.map.len() >= CAP
        && let Some(old) = self.map.iter().min_by_key(|(_, v)| v.iter().map(|(t, _)| *t).max()).map(|(k, _)| k.clone())
      {
        self.map.remove(&old);
      }
    }
    let held = self.map.entry(login.into()).or_default();
    held.retain(|(t, _)| now.duration_since(*t) < TTL);
    if held.len() >= PER {
      held.remove(0);
    }
    held.push((now, pick));
  }

  // the picks waiting for a line `login` just said: one per word the line names, oldest first, each taken
  // once. a word the line does not hold leaves its vouch waiting
  pub fn take(&mut self, login: &str, body: &str) -> Vec<HsEmote> {
    self.take_at(Instant::now(), login, body)
  }

  fn take_at(&mut self, now: Instant, login: &str, body: &str) -> Vec<HsEmote> {
    let Some(held) = self.map.get_mut(login) else { return Vec::new() };
    held.retain(|(t, _)| now.duration_since(*t) < TTL);
    let mut out: Vec<HsEmote> = Vec::new();
    let mut i = 0;
    while i < held.len() {
      let name = &held[i].1.0;
      if out.iter().all(|p| p.0 != *name) && names(body, name) {
        out.push(held.remove(i).1);
      } else {
        i += 1;
      }
    }
    if held.is_empty() {
      self.map.remove(login);
    }
    out
  }
}

// `body` holds `name` as a whole word, exact case
pub fn names(body: &str, name: &str) -> bool {
  body.split(' ').any(|w| w == name)
}

// `add` joins a line's picks: a word that already has one keeps it (the first vouch for a send is the send's)
pub fn merge(into: &mut Box<[HsEmote]>, add: Vec<HsEmote>) -> bool {
  let fresh: Vec<HsEmote> = add.into_iter().filter(|a| into.iter().all(|h| h.0 != a.0)).collect();
  if fresh.is_empty() {
    return false;
  }
  let mut v = std::mem::take(into).into_vec();
  v.extend(fresh);
  *into = v.into();
  true
}

#[cfg(test)]
mod tests {
  use super::*;

  fn pick(name: &str, id: &str) -> HsEmote {
    (name.into(), format!("https://cdn.7tv.app/emote/{id}/1x.webp").into(), false)
  }

  #[test]
  fn a_vouch_waits_for_the_line_that_names_its_word_and_is_taken_once() {
    let mut v = Vouches::default();
    let t = Instant::now();
    v.hold_at(t, "bob", pick("KEKW", "a"));
    assert!(v.take_at(t, "bob", "hello there").is_empty(), "a line without the word leaves it waiting");
    assert!(v.take_at(t, "carol", "KEKW").is_empty(), "someone else's KEKW is not bob's pick");
    assert!(v.take_at(t, "bob", "kekw KEKWW").is_empty(), "a whole word, exact case");
    assert_eq!(v.take_at(t, "bob", "so KEKW KEKW"), [pick("KEKW", "a")]);
    assert!(v.take_at(t, "bob", "KEKW").is_empty(), "one vouch is one send: the next KEKW is typed by hand");
    assert!(v.map.is_empty(), "nothing left held");
  }

  #[test]
  fn two_sends_of_one_name_pair_in_order_and_a_line_takes_one_pick_per_word() {
    let mut v = Vouches::default();
    let t = Instant::now();
    v.hold_at(t, "bob", pick("KEKW", "first"));
    v.hold_at(t, "bob", pick("KEKW", "second"));
    v.hold_at(t, "bob", pick("Pog", "p"));
    assert_eq!(v.take_at(t, "bob", "KEKW Pog"), [pick("KEKW", "first"), pick("Pog", "p")]);
    assert_eq!(v.take_at(t, "bob", "KEKW"), [pick("KEKW", "second")]);
  }

  #[test]
  fn a_vouch_expires_and_the_store_is_bounded() {
    let mut v = Vouches::default();
    let t = Instant::now();
    v.hold_at(t, "bob", pick("KEKW", "a"));
    assert!(v.take_at(t + TTL, "bob", "KEKW").is_empty(), "too old to be this line's");
    for i in 0..CAP + 50 {
      v.hold_at(t + Duration::from_millis(i as u64), &format!("u{i}"), pick("KEKW", "a"));
    }
    assert!(v.map.len() <= CAP);
    assert!(v.map.contains_key(&*format!("u{}", CAP + 49)), "the newest is kept");
    for i in 0..PER + 5 {
      v.hold_at(t, "flood", pick(&format!("E{i}"), "a"));
    }
    assert_eq!(v.map["flood"].len(), PER);
  }

  #[test]
  fn merge_keeps_the_pick_a_word_already_has() {
    let mut picks: Box<[HsEmote]> = Box::new([pick("KEKW", "first")]);
    assert!(!merge(&mut picks, vec![pick("KEKW", "late")]));
    assert!(merge(&mut picks, vec![pick("KEKW", "late"), pick("Pog", "p")]));
    assert_eq!(&*picks, [pick("KEKW", "first"), pick("Pog", "p")]);
  }
}
