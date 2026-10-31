// third-party emote sets (7tv / bttv / ffz), global + per channel, plus twitch's own by id.
// this is the CATALOG of what a channel has enabled — heatsync per-sender emotes live elsewhere
pub mod block;
pub mod live;
pub mod providers;

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
  SevenTv,
  Bttv,
  Ffz,
  // twitch's own globals. never in ORDER: other people's lines get twitch emotes from the
  // irc `emotes` tag (only what they can actually use); this set is for our own echo + completion
  Twitch,
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

pub(crate) const MAX_SET: usize = 5000;
const MAX_NAME: usize = 100;

pub(crate) fn valid_name(n: &str) -> bool {
  !n.is_empty() && n.len() <= MAX_NAME && !n.chars().any(|c| c.is_whitespace() || c.is_control())
}

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

  // every emote once, the provider that wins a shared name first
  fn entries(&self) -> Vec<(&str, &Emote)> {
    let mut seen = std::collections::HashSet::new();
    let order = ORDER.iter().chain(std::iter::once(&Provider::Twitch));
    order
      .filter_map(|p| self.sets.get(p))
      .flat_map(|s| s.iter().map(|(k, v)| (&**k, v)))
      .filter(|(k, _)| seen.insert(*k))
      .collect()
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

  pub fn twitch_global(&self, word: &str) -> Option<&Emote> {
    self.global.sets.get(&Provider::Twitch)?.get(word)
  }

  pub fn channel_names(&self, channel: &str) -> Vec<&str> {
    self.channels.get(channel).map_or_else(Vec::new, Scope::names)
  }

  pub fn channel_entries(&self, channel: &str) -> Vec<(&str, &Emote)> {
    self.channels.get(channel).map_or_else(Vec::new, Scope::entries)
  }

  pub fn global_entries(&self) -> Vec<(&str, &Emote)> {
    self.global.entries()
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
