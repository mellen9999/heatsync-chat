// third-party emote sets (7tv / bttv / ffz), global + per channel, plus twitch's own by id.
// this is the CATALOG of what a channel has enabled — heatsync per-sender emotes live elsewhere
pub mod providers;

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
  SevenTv,
  Bttv,
  Ffz,
}

#[derive(Clone, Debug)]
pub struct Emote {
  pub url: Box<str>,
  // native 1x size when the provider tells us; 0 = unknown until loaded
  pub w: u16,
  pub h: u16,
  pub zero_width: bool,
}

pub type Set = HashMap<Box<str>, Emote>;

#[derive(Default)]
struct Scope {
  sets: HashMap<Provider, Set>,
}

// which provider wins a name both define (ext setting emoteProviderPriority, default ffz)
const ORDER: [Provider; 3] = [Provider::Ffz, Provider::SevenTv, Provider::Bttv];

#[derive(Default)]
pub struct Emotes {
  global: Scope,
  channels: HashMap<Box<str>, Scope>,
}

impl Emotes {
  pub fn set(&mut self, channel: Option<&str>, p: Provider, set: Set) {
    let scope = match channel {
      Some(c) => self.channels.entry(c.into()).or_default(),
      None => &mut self.global,
    };
    scope.sets.insert(p, set);
  }

  pub fn drop_channel(&mut self, channel: &str) {
    self.channels.remove(channel);
  }

  // channel sets beat globals; within a scope, ORDER decides
  pub fn lookup(&self, channel: &str, word: &str) -> Option<&Emote> {
    let ch = self.channels.get(channel);
    ch.into_iter()
      .chain([&self.global])
      .find_map(|s| ORDER.iter().find_map(|p| s.sets.get(p)?.get(word)))
  }
}

pub fn twitch_url(id: &str) -> String {
  format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/1.0")
}

#[cfg(test)]
mod tests {
  use super::*;

  fn e(url: &str) -> Emote {
    Emote {
      url: url.into(),
      w: 28,
      h: 28,
      zero_width: false,
    }
  }

  #[test]
  fn channel_beats_global_and_priority_holds() {
    let mut em = Emotes::default();
    em.set(None, Provider::Bttv, [("LUL".into(), e("g-bttv"))].into());
    em.set(Some("c"), Provider::SevenTv, [("LUL".into(), e("c-7tv"))].into());
    em.set(Some("c"), Provider::Ffz, [("LUL".into(), e("c-ffz"))].into());
    assert_eq!(&*em.lookup("c", "LUL").unwrap().url, "c-ffz");
    assert_eq!(&*em.lookup("other", "LUL").unwrap().url, "g-bttv");
    assert!(em.lookup("c", "lul").is_none());
  }
}
