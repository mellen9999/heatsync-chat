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

impl Scope {
  fn get(&self, word: &str) -> Option<&Emote> {
    ORDER.iter().find_map(|p| self.sets.get(p)?.get(word))
  }

  fn names(&self) -> Vec<&str> {
    self.sets.values().flat_map(|s| s.keys().map(|k| &**k)).collect()
  }
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

  // within a scope, ORDER decides. callers check the channel before globals
  pub fn channel(&self, channel: &str, word: &str) -> Option<&Emote> {
    self.channels.get(channel)?.get(word)
  }

  pub fn global(&self, word: &str) -> Option<&Emote> {
    self.global.get(word)
  }

  pub fn channel_names(&self, channel: &str) -> Vec<&str> {
    self.channels.get(channel).map_or_else(Vec::new, Scope::names)
  }

  pub fn global_names(&self) -> Vec<&str> {
    self.global.names()
  }
}

pub fn twitch_url(id: &str) -> String {
  format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/1.0")
}

#[cfg(test)]
mod tests {
  use super::*;

  fn e(url: &str) -> Emote {
    Emote { url: url.into(), w: 28, h: 28, zero_width: false }
  }

  #[test]
  fn channel_beats_global_and_priority_holds() {
    let mut em = Emotes::default();
    em.set(None, Provider::Bttv, [("LUL".into(), e("g-bttv"))].into());
    em.set(Some("c"), Provider::SevenTv, [("LUL".into(), e("c-7tv"))].into());
    em.set(Some("c"), Provider::Ffz, [("LUL".into(), e("c-ffz"))].into());
    let find = |c, w| em.channel(c, w).or_else(|| em.global(w));
    assert_eq!(&*find("c", "LUL").unwrap().url, "c-ffz");
    assert_eq!(&*find("other", "LUL").unwrap().url, "g-bttv");
    assert!(find("c", "lul").is_none());
  }
}
