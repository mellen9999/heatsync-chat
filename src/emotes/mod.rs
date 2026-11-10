// third-party emote sets (7tv / bttv / ffz), global + per channel, plus twitch's own by id and a merged
// tab's kick channel (its own + kick's globals).
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
  // twitch's own globals. always last in a walk: other people's lines get twitch emotes from the
  // irc `emotes` tag (only what they can actually use); this set is for our own echo + completion
  Twitch,
  // the tab's kick channel, its own emotes and kick's globals: after every twitch-side set, so a twitch
  // channel's own name always wins. kick lines carry their emotes as tokens; this is for the bare word
  // on any line in the tab, the picker, and the box's kick copy (`kickify`)
  Kick,
}

// the sets that close every walk, in order
const LAST: [Provider; 2] = [Provider::Twitch, Provider::Kick];

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
  // the three third-party sets in `order`, then twitch's own, then kick's (the same walk as `provided`)
  fn get(&self, order: &[Provider; 3], word: &str) -> Option<&Emote> {
    order.iter().chain(&LAST).find_map(|p| self.sets.get(p)?.get(word))
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
    let order = order.iter().chain(&LAST);
    order
      .filter_map(|p| Some((*p, self.sets.get(p)?)))
      .flat_map(|(p, s)| s.iter().map(move |(k, v)| (&**k, p, v)))
      .filter(move |(k, _, _)| seen.insert(*k))
  }
}

// which provider wins a name two of them define (emote rule 3). a channel's sets: ffz, 7tv, bttv. the
// globals: 7tv, bttv, ffz. twitch's own set, then kick's, always last. only ever among these sets:
// heatsync inventory emotes never come through here
const CHANNEL_ORDER: [Provider; 3] = [Provider::Ffz, Provider::SevenTv, Provider::Bttv];
const GLOBAL_ORDER: [Provider; 3] = [Provider::SevenTv, Provider::Bttv, Provider::Ffz];

#[derive(Default)]
pub struct Emotes {
  global: Scope,
  channels: HashMap<Box<str>, Scope>,
  // twitch emotes you may send anywhere (your subs'), as heatsync reads them off your twitch account.
  // yours only: tab, the picker, the box and your own lines, never someone else's line
  subs: Set,
  // each joined kick channel's set, by slug. a room draws on one through `route_kick`
  kick: HashMap<Box<str>, Set>,
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

  pub fn set_kick(&mut self, slug: &str, set: Set) {
    self.kick.insert(slug.into(), set);
  }

  pub fn drop_kick(&mut self, slug: &str) {
    self.kick.remove(slug);
  }

  // which kick channel each room (a tab's chat key) draws on, rebuilt whole from the tabs, so a closed or
  // edited tab can't leave a stale set behind. a room two tabs share takes the kick channel either names
  pub fn route_kick<'a>(&mut self, rooms: impl IntoIterator<Item = (&'a str, &'a str)>) {
    for s in self.channels.values_mut() {
      s.sets.remove(&Provider::Kick);
    }
    for (room, slug) in rooms {
      if let Some(set) = self.kick.get(slug) {
        self.channels.entry(room.into()).or_default().sets.entry(Provider::Kick).or_insert_with(|| set.clone());
      }
    }
    self.channels.retain(|_, s| !s.sets.is_empty());
  }

  // `text` as kick's wire wants it: a word that is one of `slug`'s kick emotes becomes its
  // `[emote:<id>:<name>]` token, or kick shows the bare word to everyone. a message the tokens push past
  // kick's limit goes as words: delivered without images beats rejected
  pub fn kickify(&self, slug: &str, text: &str) -> String {
    const KICK_MAX: usize = 500;
    let Some(set) = self.kick.get(slug) else { return text.to_owned() };
    let out: Vec<String> = text
      .split(' ')
      .map(|w| match set.get(w).and_then(|e| kick_id(&e.url)) {
        Some(id) => format!("[emote:{id}:{w}]"),
        None => w.to_owned(),
      })
      .collect();
    let out = out.join(" ");
    if out.chars().count() > KICK_MAX && text.chars().count() <= KICK_MAX { text.to_owned() } else { out }
  }

  // within a scope, the provider order decides. callers check the channel before globals
  pub fn channel(&self, channel: &str, word: &str) -> Option<&Emote> {
    self.channels.get(channel)?.get(&CHANNEL_ORDER, word)
  }

  pub fn global(&self, word: &str) -> Option<&Emote> {
    self.global.get(&GLOBAL_ORDER, word)
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
    self.channels.get(channel).map_or_else(Vec::new, |c| c.names(&CHANNEL_ORDER))
  }

  pub fn channel_entries(&self, channel: &str) -> Vec<(&str, &Emote)> {
    self.channels.get(channel).map_or_else(Vec::new, |c| c.entries(&CHANNEL_ORDER))
  }

  pub fn global_entries(&self) -> Vec<(&str, &Emote)> {
    self.global.entries(&GLOBAL_ORDER)
  }

  pub fn global_names(&self) -> Vec<(&str, Provider, &Emote)> {
    self.global.names(&GLOBAL_ORDER)
  }
}

pub fn kick_url(id: u64) -> String {
  format!("https://files.kick.com/emotes/{id}/fullsize")
}

fn kick_id(url: &str) -> Option<u64> {
  url.strip_prefix("https://files.kick.com/emotes/")?.strip_suffix("/fullsize")?.parse().ok()
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

  // rule 3: a channel walks ffz, 7tv, bttv; the globals walk 7tv, bttv, ffz, twitch. the two orders differ
  // on purpose, so the same three-way collision gives a different picture in each scope
  #[test]
  fn channel_walks_ffz_7tv_bttv_and_globals_walk_7tv_bttv_ffz_twitch() {
    let mut em = Emotes::default();
    for (p, tag) in [(Provider::SevenTv, "7tv"), (Provider::Bttv, "bttv"), (Provider::Ffz, "ffz")] {
      em.set(Some("c"), p, [("Same".into(), e(&format!("c-{tag}")))].into());
      em.set(None, p, [("Same".into(), e(&format!("g-{tag}")))].into());
    }
    em.set(None, Provider::Twitch, [("Same".into(), e("g-twitch")), ("Only".into(), e("g-twitch-only"))].into());
    assert_eq!(&*em.channel("c", "Same").unwrap().url, "c-ffz");
    assert_eq!(&*em.global("Same").unwrap().url, "g-7tv");
    assert_eq!(&*em.global("Only").unwrap().url, "g-twitch-only", "twitch last, but reached");
    let mut em = Emotes::default();
    em.set(None, Provider::Ffz, [("G".into(), e("g-ffz"))].into());
    em.set(None, Provider::Bttv, [("G".into(), e("g-bttv"))].into());
    assert_eq!(&*em.global("G").unwrap().url, "g-bttv", "bttv before ffz");
    assert_eq!(&*em.global_entries()[0].1.url, "g-bttv");
    em.set(None, Provider::SevenTv, [("G".into(), e("g-7tv"))].into());
    assert_eq!(em.global_names()[0].1, Provider::SevenTv);
  }

  #[test]
  fn kick_comes_after_every_twitch_side_set_and_follows_the_tabs() {
    let mut em = Emotes::default();
    em.set(Some("c"), Provider::SevenTv, [("Clap".into(), e("c-7tv"))].into());
    em.set_kick("k", [("Clap".into(), e(&kick_url(1))), ("peepoDJ".into(), e(&kick_url(37245)))].into());
    assert!(em.channel("c", "peepoDJ").is_none(), "not routed yet");
    em.route_kick([("c", "k")]);
    assert_eq!(&*em.channel("c", "Clap").unwrap().url, "c-7tv", "the twitch channel's own name wins");
    assert_eq!(&*em.channel("c", "peepoDJ").unwrap().url, &kick_url(37245));
    assert_eq!(em.channel_names("c").iter().find(|n| n.0 == "peepoDJ").unwrap().1, Provider::Kick);
    // a kick-only room has no twitch sets of its own
    em.route_kick([("c", "k"), ("kickonly", "k")]);
    assert!(em.channel("kickonly", "peepoDJ").is_some());
    // the tab closed or lost its kick leg: rerouted without it, nothing stays behind
    em.route_kick([]);
    assert!(em.channel("c", "peepoDJ").is_none());
    assert!(em.channel("c", "Clap").is_some(), "the twitch sets stay");
    assert!(!em.channels.contains_key("kickonly"));
    em.drop_kick("k");
    em.route_kick([("c", "k")]);
    assert!(em.channel("c", "peepoDJ").is_none(), "a left kick channel routes nothing");
  }

  #[test]
  fn kickify_tokens_kick_emotes_only_and_never_overflows() {
    let mut em = Emotes::default();
    em.set_kick("k", [("peepoDJ".into(), e(&kick_url(37245))), ("odd".into(), e("https://cdn.7tv.app/x"))].into());
    assert_eq!(em.kickify("k", "peepoDJ turn  it up odd"), "[emote:37245:peepoDJ] turn  it up odd");
    assert_eq!(em.kickify("other", "peepoDJ"), "peepoDJ", "another channel's emotes stay words");
    assert_eq!(em.kickify("k", "peepodj [emote:1:x]"), "peepodj [emote:1:x]");
    let long = "peepoDJ ".repeat(60);
    assert_eq!(em.kickify("k", &long), long, "500 chars of words beat a rejected send");
  }

  #[test]
  fn ffz_wins_a_shared_name_in_a_channel_then_7tv_then_bttv_in_every_view() {
    let mut em = Emotes::default();
    em.set(Some("c"), Provider::SevenTv, [("Same".into(), e("c-7tv"))].into());
    em.set(Some("c"), Provider::Bttv, [("Same".into(), e("c-bttv"))].into());
    assert_eq!(&*em.channel("c", "Same").unwrap().url, "c-7tv", "7tv before bttv");
    em.set(Some("c"), Provider::Ffz, [("Same".into(), e("c-ffz"))].into());
    assert_eq!(&*em.channel("c", "Same").unwrap().url, "c-ffz", "ffz before 7tv");
    let names = em.channel_names("c");
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].1, Provider::Ffz);
  }
}
