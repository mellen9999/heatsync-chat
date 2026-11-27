// your own twitch badges and name colour, per room, for the lines you say on kick and youtube. a send to
// several feeds keeps whichever echo lands first, and kick's copy carries kick's colour and no twitch badges:
// your line on one feed wears the look your twitch self has in that room (ext `_ownBadgesByChannel`, and the
// colour and badges a folded twitch copy lends its row)
use std::collections::HashMap;

use super::message::{Message, Platform};

// rooms held: a tab list never comes near it
const ROOMS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
  pub badges: Box<[(Box<str>, Box<str>)]>,
  pub color: u32,
  // when the line it came from was said: an older line never replaces a newer one
  at: i64,
}

#[derive(Default)]
pub struct Own {
  rooms: HashMap<Box<str>, Look>,
}

impl Own {
  // one of your twitch lines in `room`. true = the look changed (lines already drawn want laying out again)
  pub fn note(&mut self, room: &str, m: &Message) -> bool {
    if m.platform != Platform::Twitch || !m.kind.said() {
      return false;
    }
    let key = room.to_ascii_lowercase();
    let fresh = Look { badges: m.badges.clone(), color: m.color, at: m.ts };
    if let Some(old) = self.rooms.get_mut(key.as_str()) {
      if old.at > m.ts {
        return false;
      }
      let changed = old.color != fresh.color || old.badges != fresh.badges;
      *old = fresh;
      return changed;
    }
    if self.rooms.len() >= ROOMS {
      return false;
    }
    self.rooms.insert(key.into(), fresh);
    true
  }

  pub fn get(&self, room: &str) -> Option<&Look> {
    self.rooms.get(room.to_ascii_lowercase().as_str())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::Kind;

  fn line(p: Platform, ts: i64, color: u32, badges: &[(&str, &str)]) -> Message {
    let mut m = Message::system(ts, p, "x");
    m.kind = Kind::Chat;
    m.color = color;
    m.badges = badges.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect();
    m
  }

  #[test]
  fn the_newest_twitch_line_of_yours_sets_the_look() {
    let mut o = Own::default();
    assert!(o.note("Room", &line(Platform::Twitch, 10, 1, &[("moderator", "1")])));
    assert!(!o.note("room", &line(Platform::Twitch, 20, 1, &[("moderator", "1")])), "nothing new");
    assert!(!o.note("room", &line(Platform::Twitch, 5, 2, &[])), "an older line never wins");
    assert_eq!(o.get("ROOM").map(|l| l.color), Some(1));
    assert!(o.note("room", &line(Platform::Twitch, 30, 2, &[])), "a lost badge and a new colour are news");
    assert!(o.get("room").is_some_and(|l| l.badges.is_empty() && l.color == 2));
  }

  #[test]
  fn only_twitch_lines_count_and_the_table_is_bounded() {
    let mut o = Own::default();
    assert!(!o.note("room", &line(Platform::Kick, 1, 1, &[])));
    assert!(o.get("room").is_none());
    for i in 0..ROOMS + 5 {
      o.note(&format!("r{i}"), &line(Platform::Twitch, 1, 1, &[]));
    }
    assert!(o.get("r0").is_some() && o.get(&format!("r{}", ROOMS + 4)).is_none());
  }
}
