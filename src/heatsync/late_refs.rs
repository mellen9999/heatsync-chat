// heatsync's refs for a twitch line that came in before our own irc copy of it: held briefly, by line
// id, and put on the line when it lands. bounded in count and age; what never lands just expires
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::model::message::HsEmote;

const CAP: usize = 256;
const TTL: Duration = Duration::from_secs(30);

// when it came, the room, the refs
type Held = (Instant, Box<str>, Box<[HsEmote]>);

#[derive(Default)]
pub struct LateRefs {
  // by line id
  map: HashMap<Box<str>, Held>,
}

impl LateRefs {
  pub fn hold(&mut self, chan: Box<str>, id: Box<str>, refs: Box<[HsEmote]>) {
    self.hold_at(Instant::now(), chan, id, refs);
  }

  fn hold_at(&mut self, now: Instant, chan: Box<str>, id: Box<str>, refs: Box<[HsEmote]>) {
    if self.map.len() >= CAP && !self.map.contains_key(&id) {
      self.map.retain(|_, (t, _, _)| now.duration_since(*t) < TTL);
      if self.map.len() >= CAP
        && let Some(old) = self.map.iter().min_by_key(|(_, (t, _, _))| *t).map(|(k, _)| k.clone())
      {
        self.map.remove(&old);
      }
    }
    self.map.insert(id, (now, chan, refs));
  }

  // the refs waiting for this line, if any (and still fresh)
  pub fn take(&mut self, chan: &str, id: &str) -> Option<Box<[HsEmote]>> {
    self.take_at(Instant::now(), chan, id)
  }

  fn take_at(&mut self, now: Instant, chan: &str, id: &str) -> Option<Box<[HsEmote]>> {
    if self.map.is_empty() || id.is_empty() {
      return None;
    }
    let (t, c, _) = self.map.get(id)?;
    if **c != *chan {
      return None;
    }
    let fresh = now.duration_since(*t) < TTL;
    self.map.remove(id).filter(|_| fresh).map(|(_, _, r)| r)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn refs() -> Box<[HsEmote]> {
    Box::new([("kek".into(), "https://cdn.7tv.app/emote/x/1x.webp".into(), false)])
  }

  #[test]
  fn held_by_room_and_id_bounded_and_aged() {
    let (mut l, t) = (LateRefs::default(), Instant::now());
    l.hold_at(t, "forsen".into(), "a".into(), refs());
    assert!(l.take_at(t, "xqc", "a").is_none(), "another room's line of the same id");
    assert!(l.take_at(t, "forsen", "a").is_some());
    assert!(l.take_at(t, "forsen", "a").is_none(), "taken once");
    l.hold_at(t, "forsen".into(), "old".into(), refs());
    assert!(l.take_at(t + TTL, "forsen", "old").is_none(), "expired");
    for n in 0..CAP + 50 {
      l.hold_at(t + Duration::from_millis(n as u64), "forsen".into(), n.to_string().into(), refs());
    }
    assert_eq!(l.map.len(), CAP);
    assert!(l.take_at(t, "forsen", "0").is_none(), "the oldest made way");
    assert!(l.take_at(t, "forsen", &(CAP + 49).to_string()).is_some());
  }
}
