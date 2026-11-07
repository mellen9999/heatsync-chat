// third-party emote sets (7tv / bttv / ffz), global + per channel, plus twitch's own by id.
// this is the CATALOG of what a channel has enabled — heatsync per-sender emotes live elsewhere.
// resolve.rs is the one place a chat word becomes an emote (the privacy chokepoint)
pub mod block;
pub mod entitle;
pub mod live;
pub mod personal;
pub mod providers;
pub mod resolve;

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Provider {
  SevenTv,
  Bttv,
  Ffz,
  // twitch's own globals. never in the order: other people's lines get twitch emotes from the
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
  fn get(&self, order: &[Provider; 3], word: &str) -> Option<&Emote> {
    order.iter().find_map(|p| self.sets.get(p)?.get(word))
  }

  // every name once, with the provider that wins it
  fn names(&self, order: &[Provider; 3]) -> Vec<(&str, Provider, &Emote)> {
    self.provided(order).collect()
  }

  // every emote once, the provider that wins a shared name first
  fn entries(&self, order: &[Provider; 3]) -> Vec<(&str, &Emote)> {
    self.provided(order).map(|(k, _, v)| (k, v)).collect()
  }

  fn provided(&self, order: &[Provider; 3]) -> impl Iterator<Item = (&str, Provider, &Emote)> {
    let mut seen = std::collections::HashSet::new();
    let order = order.iter().chain(std::iter::once(&Provider::Twitch));
    order
      .filter_map(|p| Some((*p, self.sets.get(p)?)))
      .flat_map(|(p, s)| s.iter().map(move |(k, v)| (&**k, p, v)))
      .filter(move |(k, _, _)| seen.insert(*k))
  }
}

// which provider wins a name two of them define: ffz, then 7tv, then bttv (the ext's emoteProviderPriority
// default). only ever among the third-party sets: heatsync inventory emotes never come through here
const ORDER: [Provider; 3] = [Provider::Ffz, Provider::SevenTv, Provider::Bttv];

pub struct Emotes {
  global: Scope,
  channels: HashMap<Box<str>, Scope>,
  order: [Provider; 3],
  // twitch emotes you may send anywhere (your subs'), as heatsync reads them off your twitch account.
  // yours only: tab, the picker, the box and your own lines, never someone else's line
  subs: Set,
}

impl Default for Emotes {
  fn default() -> Self {
    Emotes { global: Scope::default(), channels: HashMap::new(), order: ORDER, subs: Set::new() }
  }
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

  // within a scope, the provider order decides. callers check the channel before globals
  pub fn channel(&self, channel: &str, word: &str) -> Option<&Emote> {
    self.channels.get(channel)?.get(&self.order, word)
  }

  pub fn global(&self, word: &str) -> Option<&Emote> {
    self.global.get(&self.order, word)
  }

  pub fn set_subs(&mut self, subs: Set) {
    self.subs = subs;
  }

  pub fn sub(&self, word: &str) -> Option<&Emote> {
    self.subs.get(word)
  }

  pub fn sub_entries(&self) -> Vec<(&str, &Emote)> {
    let mut v: Vec<_> = self.subs.iter().map(|(k, e)| (&**k, e)).collect();
    v.sort_by_cached_key(|e| e.0.to_lowercase());
    v
  }

  pub fn twitch_global(&self, word: &str) -> Option<&Emote> {
    self.global.sets.get(&Provider::Twitch)?.get(word)
  }

  pub fn channel_names(&self, channel: &str) -> Vec<(&str, Provider, &Emote)> {
    self.channels.get(channel).map_or_else(Vec::new, |c| c.names(&self.order))
  }

  pub fn channel_entries(&self, channel: &str) -> Vec<(&str, &Emote)> {
    self.channels.get(channel).map_or_else(Vec::new, |c| c.entries(&self.order))
  }

  pub fn global_entries(&self) -> Vec<(&str, &Emote)> {
    self.global.entries(&self.order)
  }

  pub fn global_names(&self) -> Vec<(&str, Provider, &Emote)> {
    self.global.names(&self.order)
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

  #[test]
  fn ffz_wins_a_shared_name_then_7tv_then_bttv_in_every_view() {
    let mut em = Emotes::default();
    em.set(Some("c"), Provider::SevenTv, [("Same".into(), e("c-7tv"))].into());
    em.set(Some("c"), Provider::Bttv, [("Same".into(), e("c-bttv"))].into());
    em.set(None, Provider::Ffz, [("G".into(), e("g-ffz"))].into());
    em.set(None, Provider::Bttv, [("G".into(), e("g-bttv"))].into());
    assert_eq!(&*em.channel("c", "Same").unwrap().url, "c-7tv", "7tv before bttv");
    assert_eq!(&*em.global("G").unwrap().url, "g-ffz", "ffz before bttv");
    em.set(Some("c"), Provider::Ffz, [("Same".into(), e("c-ffz"))].into());
    assert_eq!(&*em.channel("c", "Same").unwrap().url, "c-ffz", "ffz before 7tv");
    let names = em.channel_names("c");
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].1, Provider::Ffz);
    assert_eq!(&*em.global_entries()[0].1.url, "g-ffz");
  }
}
