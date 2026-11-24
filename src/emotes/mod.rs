// third-party emote sets (7tv / bttv / ffz), global + per channel, plus twitch's own by id and a merged
// tab's kick channel (its own + kick's globals).
// this is the CATALOG of what a channel has enabled, the streamer's own heatsync emotes included (`owners`)
// — heatsync per-sender emotes live elsewhere.
// resolve.rs is the one place a chat word becomes an emote (the privacy chokepoint)
pub mod block;
pub mod draft;
pub mod entitle;
pub mod favorites;
pub mod live;
pub mod mods;
pub mod personal;
pub mod providers;
pub mod resolve;

use std::collections::HashMap;

use crate::model::message::Platform;

// a channel on one platform: a twitch "ninja" and a kick "ninja" are different people
pub type Leg = (Platform, Box<str>);

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
// a twitch line's walk of its room: the channel's three third-party sets, then twitch's own
const TWITCH_ROOM: [Provider; 4] = [Provider::Ffz, Provider::SevenTv, Provider::Bttv, Provider::Twitch];

#[derive(Clone, Debug)]
pub struct Emote {
  pub url: Box<str>,
  // native 1x size when the provider tells us; 0 = unknown until loaded
  pub w: u16,
  pub h: u16,
  pub zero_width: bool,
}

pub type Set = HashMap<Box<str>, Emote>;

// `name0` read as the name it overlays (the emote law): only tried when nothing answers the word as written
pub fn overlay_base(word: &str) -> Option<&str> {
  word.strip_suffix('0').filter(|b| !b.is_empty())
}

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

  // every emote of every set: a name two sets hold comes twice (the law names the winner, emotes/draft.rs)
  fn all(&self) -> impl Iterator<Item = (&str, Provider, &Emote)> {
    self.sets.iter().flat_map(|(p, s)| s.iter().map(move |(k, v)| (&**k, *p, v)))
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
  // each streamer's own heatsync emotes (the site's owner route), whatever provider a picture came from.
  // heatsync's, so the heatsync emotes switch gates them where they are read. a room draws on its
  // platforms' through `route_owner`
  owners: HashMap<Leg, Set>,
  // the channels each room (a tab's chat key) is
  legs: HashMap<Box<str>, Vec<Leg>>,
  // a kick or youtube channel's own 7tv and bttv emotes, by channel (heatsync's read of them). a line reads
  // its own platform's channel only: a kick line in a twitch + kick tab never draws the twitch channel's
  third: HashMap<Leg, Scope>,
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

  // which channels each room is, rebuilt whole from the tabs like `route_kick`: a closed tab's sets go with
  // it. answers (the channels new to the route, the ones that left it): what to ask for, what to stop
  pub fn route_owner<'a>(&mut self, rooms: impl IntoIterator<Item = (&'a str, &'a [Leg])>) -> (Vec<Leg>, Vec<Leg>) {
    let old: Vec<Leg> = std::mem::take(&mut self.legs).into_values().flatten().collect();
    for (room, legs) in rooms {
      let seat = self.legs.entry(room.into()).or_default();
      for l in legs {
        if !seat.contains(l) {
          seat.push(l.clone());
        }
      }
    }
    let mut new: Vec<Leg> = Vec::new();
    for l in self.legs.values().flatten() {
      if !old.contains(l) && !new.contains(l) {
        new.push(l.clone());
      }
    }
    let mut gone: Vec<Leg> = Vec::new();
    for l in old {
      if !self.routed(&l) && !gone.contains(&l) {
        gone.push(l);
      }
    }
    let legs = &self.legs;
    self.owners.retain(|l, _| legs.values().flatten().any(|x| x == l));
    self.third.retain(|l, _| legs.values().flatten().any(|x| x == l));
    (new, gone)
  }

  fn routed(&self, leg: &Leg) -> bool {
    self.legs.values().flatten().any(|l| l == leg)
  }

  // the channels in some tab that a heatsync account's key names (`kick:<name>`, `yt:<handle>`, `ytc:<id>`
  // as the server spells them: lowercase, no `@`). an older kick account `some_name` lives at the address
  // `some-name`, and a tab holds the address: a match only re-reads, and the server says whose it is
  pub fn legs_named(&self, p: Platform, name: &str) -> Vec<Leg> {
    let at = |l: &str| p == Platform::Kick && l.contains('-') && l.replace('-', "_").eq_ignore_ascii_case(name);
    let is = |l: &&Leg| l.0 == p && (l.1.trim_start_matches('@').eq_ignore_ascii_case(name) || at(&l.1));
    let mut out: Vec<Leg> = Vec::new();
    for l in self.legs.values().flatten().filter(is) {
      if !out.contains(l) {
        out.push(l.clone());
      }
    }
    out
  }

  // false = no tab is this channel any more: the answer is nobody's
  pub fn set_owner(&mut self, leg: Leg, set: Set) -> bool {
    let on = self.routed(&leg);
    if on {
      self.owners.insert(leg, set);
    }
    on
  }

  // a kick or youtube channel's third-party sets: each provider given replaces that provider's last answer
  // whole, an empty one included (an emote the channel dropped goes); one not given keeps what it had.
  // false = no tab is this channel any more
  pub fn set_leg(&mut self, leg: Leg, sets: Vec<(Provider, Set)>) -> bool {
    let on = leg.0 != Platform::Twitch && self.routed(&leg);
    if on {
      self.third.entry(leg).or_default().sets.extend(sets);
    }
    on
  }

  // the kick and youtube channels whose third-party sets have been read: nothing pushes a change to those,
  // so a reconnect reads them again
  pub fn legs_read(&self) -> Vec<Leg> {
    self.third.keys().cloned().collect()
  }

  // the channels whose streamer's set has been read: what a reconnect reads again (one still being asked
  // for the first time is already in flight)
  pub fn owners_read(&self) -> Vec<Leg> {
    self.owners.keys().cloned().collect()
  }

  // the streamer's own emote of a name on one platform's lines: the room's first answer (the law's read
  // and send order), and only that platform's streamer's
  pub fn owner_on(&self, room: &str, p: Platform, word: &str) -> Option<&Emote> {
    self.legs.get(room)?.iter().filter(|l| l.0 == p).find_map(|l| self.owners.get(l)?.get(word))
  }

  // every emote of a room's streamer on one platform
  pub fn owner_all(&self, room: &str, p: Platform) -> Vec<(&str, &Emote)> {
    let legs = self.legs.get(room).into_iter().flatten().filter(|l| l.0 == p);
    legs.filter_map(|l| self.owners.get(l)).flat_map(|s| s.iter().map(|(k, v)| (&**k, v))).collect()
  }

  // `text` as kick's wire wants it: a word that is one of `slug`'s kick emotes becomes its
  // `[emote:<id>:<name>]` token, or kick shows the bare word to everyone. a word the line already answered
  // as another emote (`other`) stays bare, so it keeps its one meaning. a message the tokens push past
  // kick's limit goes as words: delivered without images beats rejected
  pub fn kickify(&self, slug: &str, text: &str, other: impl Fn(&str) -> bool) -> String {
    const KICK_MAX: usize = 500;
    let Some(set) = self.kick.get(slug) else { return text.to_owned() };
    let out: Vec<String> = text
      .split(' ')
      .map(|w| match set.get(w).filter(|_| !other(w)).and_then(|e| kick_id(&e.url)) {
        Some(id) => format!("[emote:{id}:{w}]"),
        None => w.to_owned(),
      })
      .collect();
    let out = out.join(" ");
    if out.chars().count() > KICK_MAX && text.chars().count() <= KICK_MAX { text.to_owned() } else { out }
  }

  // within a scope, the provider order decides. callers check the channel before globals
  #[cfg(test)]
  pub fn channel(&self, channel: &str, word: &str) -> Option<&Emote> {
    self.channels.get(channel)?.get(&CHANNEL_ORDER, word)
  }

  // a room's sets as one platform's lines walk them (the law's "a line has a room", one room per platform):
  // a twitch line its channel's ffz, 7tv, bttv, then twitch's own; a kick or youtube line that channel's own
  // third-party sets, a kick line then kick's. never another platform's channel in the same tab
  fn sets_on<'a>(&'a self, room: &str, p: Platform) -> impl Iterator<Item = (Provider, &'a Set)> + 'a {
    let own = self.channels.get(room).map(|s| &s.sets);
    let twitch = own.filter(|_| p == Platform::Twitch).into_iter();
    let twitch = twitch.flat_map(|s| TWITCH_ROOM.iter().filter_map(move |q| Some((*q, s.get(q)?))));
    let legs = self.legs.get(room).into_iter().flatten().filter(move |l| l.0 == p);
    let third = legs.filter_map(|l| self.third.get(l));
    let third = third.flat_map(|s| CHANNEL_ORDER.iter().filter_map(move |q| Some((*q, s.sets.get(q)?))));
    let kick = own.filter(|_| p == Platform::Kick).and_then(|s| Some((Provider::Kick, s.get(&Provider::Kick)?)));
    twitch.chain(third).chain(kick)
  }

  // the room's emote of a name on one platform's lines
  pub fn channel_on(&self, room: &str, p: Platform, word: &str) -> Option<&Emote> {
    self.sets_on(room, p).find_map(|(_, s)| s.get(word))
  }

  // every emote a room holds for one platform's lines, each provider's copy of a shared name included
  pub fn channel_all(&self, room: &str, p: Platform) -> Vec<(&str, Provider, &Emote)> {
    self.sets_on(room, p).flat_map(|(q, s)| s.iter().map(move |(k, v)| (&**k, q, v))).collect()
  }

  pub fn global_all(&self) -> Vec<(&str, Provider, &Emote)> {
    self.global.all().collect()
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

  // every emote of a room on one platform's lines, once: the set that wins a shared name first
  pub fn channel_entries(&self, room: &str, p: Platform) -> Vec<(&str, &Emote)> {
    let mut seen = std::collections::HashSet::new();
    let all = self.sets_on(room, p).flat_map(|(_, s)| s.iter().map(|(k, v)| (&**k, v)));
    all.filter(|(k, _)| seen.insert(*k)).collect()
  }

  pub fn global_entries(&self) -> Vec<(&str, &Emote)> {
    self.global.entries(&GLOBAL_ORDER)
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
    assert_eq!(&*em.global_entries()[0].1.url, "g-7tv");
    assert_eq!(em.global_all().len(), 3, "every provider's copy is kept: the law names the winner for tab");
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
    assert!(
      em.channel_on("c", Platform::Twitch, "peepoDJ").is_none(),
      "a twitch line never reads the tab's kick channel"
    );
    assert_eq!(&*em.channel_on("c", Platform::Kick, "peepoDJ").unwrap().url, &kick_url(37245), "a kick line does");
    assert_eq!(
      &*em.channel_on("c", Platform::Kick, "Clap").unwrap().url,
      &kick_url(1),
      "and never the twitch channel's 7tv"
    );
    assert_eq!(&*em.channel_on("c", Platform::Twitch, "Clap").unwrap().url, "c-7tv");
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

  // one room per platform (the emote law): a tab on twitch + kick + youtube holds three channels, and a
  // line reads its own. a kick or youtube channel's 7tv and bttv emotes are its own, read through heatsync
  #[test]
  fn a_line_reads_its_own_platforms_channel_and_never_the_tabs_other_ones() {
    let leg = |p, k: &str| -> Leg { (p, k.into()) };
    let (kick, yt) = (leg(Platform::Kick, "k"), leg(Platform::Youtube, "UCaaaaaaaaaaaaaaaaaaaaaa"));
    let mut em = Emotes::default();
    em.set(Some("c"), Provider::SevenTv, [("Clap".into(), e("tw-7tv")), ("TwOnly".into(), e("tw-only"))].into());
    assert!(!em.set_leg(kick.clone(), vec![(Provider::SevenTv, Set::new())]), "no tab is that channel yet");
    let legs = [leg(Platform::Twitch, "c"), kick.clone(), yt.clone()];
    em.route_owner([("c", &legs[..])]);
    assert!(!em.set_leg(leg(Platform::Twitch, "c"), vec![]), "a twitch channel's sets come from the providers");
    em.set_kick("k", [("Clap".into(), e(&kick_url(1))), ("peepoDJ".into(), e(&kick_url(2)))].into());
    em.route_kick([("c", "k")]);
    // before the kick channel's own answer: kick's emotes only, never the twitch channel's 7tv
    assert_eq!(&*em.channel_on("c", Platform::Kick, "Clap").unwrap().url, &kick_url(1));
    assert!(em.channel_on("c", Platform::Kick, "TwOnly").is_none());
    assert!(em.channel_on("c", Platform::Youtube, "Clap").is_none(), "a youtube line reads no other channel's");
    let seven = |id: &str| (Provider::SevenTv, Set::from([("Clap".into(), e(id)), ("KickOnly".into(), e("k-only"))]));
    assert!(em.set_leg(kick.clone(), vec![seven("k-7tv")]));
    assert!(em.set_leg(yt.clone(), vec![seven("y-7tv"), (Provider::Bttv, [("Clap".into(), e("y-bttv"))].into())]));
    assert_eq!(&*em.channel_on("c", Platform::Kick, "Clap").unwrap().url, "k-7tv", "its 7tv leads kick's own");
    assert_eq!(&*em.channel_on("c", Platform::Kick, "peepoDJ").unwrap().url, &kick_url(2), "kick's own still answer");
    assert_eq!(&*em.channel_on("c", Platform::Youtube, "Clap").unwrap().url, "y-7tv", "7tv before bttv");
    assert_eq!(&*em.channel_on("c", Platform::Twitch, "Clap").unwrap().url, "tw-7tv");
    assert!(em.channel_on("c", Platform::Twitch, "KickOnly").is_none(), "a twitch line never reads the kick channel's");
    // tab, the picker and the chips ask the same room: one platform's names, a shared name once
    let names = |p| -> Vec<&str> {
      let mut v: Vec<&str> = em.channel_entries("c", p).into_iter().map(|x| x.0).collect();
      v.sort_unstable();
      v
    };
    assert_eq!(names(Platform::Twitch), ["Clap", "TwOnly"]);
    assert_eq!(names(Platform::Kick), ["Clap", "KickOnly", "peepoDJ"]);
    assert_eq!(names(Platform::Youtube), ["Clap", "KickOnly"]);
    assert_eq!(em.channel_all("c", Platform::Youtube).len(), 3, "every provider's copy of a name is kept");
    let mut read = em.legs_read();
    read.sort_by_key(|l| l.0.name());
    assert_eq!(read, [kick.clone(), yt.clone()]);
    // a provider's answer replaces its last one whole (an emote the channel dropped goes); one not in the
    // answer keeps what it had (heatsync could not reach it: a copy, or nothing, came instead)
    assert!(em.set_leg(yt.clone(), vec![(Provider::Bttv, Set::new())]));
    assert_eq!(&*em.channel_on("c", Platform::Youtube, "Clap").unwrap().url, "y-7tv");
    assert_eq!(em.channel_all("c", Platform::Youtube).len(), 2, "bttv's copy of the name went");
    assert!(em.set_leg(kick.clone(), vec![(Provider::SevenTv, Set::new())]));
    assert_eq!(&*em.channel_on("c", Platform::Kick, "Clap").unwrap().url, &kick_url(1));
    // the tab lost its youtube leg: its sets go with it
    em.route_owner([("c", &legs[..2])]);
    assert!(em.channel_on("c", Platform::Youtube, "Clap").is_none() && em.legs_read() == [kick]);
  }

  // the streamer's own heatsync emotes: asked for once per channel, read by its own platform's lines only,
  // kick: an older account some_name lives at the address some-name, and a tab holds the address
  #[test]
  fn a_kick_account_named_with_an_underscore_names_its_dashed_address() {
    let leg = |p, k: &str| -> Leg { (p, k.into()) };
    let (dash, plain, tw) =
      (leg(Platform::Kick, "Some-Name"), leg(Platform::Kick, "somename"), leg(Platform::Twitch, "some-name"));
    let mut em = Emotes::default();
    let all = [dash.clone(), plain.clone(), tw.clone()];
    em.route_owner([("t", &all[..])]);
    assert_eq!(em.legs_named(Platform::Kick, "some_name"), vec![dash.clone()]);
    assert_eq!(em.legs_named(Platform::Kick, "some-name"), vec![dash], "the address itself still names it");
    assert_eq!(em.legs_named(Platform::Kick, "somename"), vec![plain]);
    assert!(em.legs_named(Platform::Twitch, "some_name").is_empty(), "only kick writes an address this way");
    assert!(em.legs_named(Platform::Youtube, "some_name").is_empty());
  }

  // and gone with the last tab that is the channel
  #[test]
  fn a_streamers_set_is_its_platforms_and_follows_the_tabs() {
    let leg = |p, k: &str| -> Leg { (p, k.into()) };
    let (tw, kick) = (leg(Platform::Twitch, "ninja"), leg(Platform::Kick, "ninja"));
    let mut em = Emotes::default();
    assert!(!em.set_owner(tw.clone(), [("Hi".into(), e("early"))].into()), "no tab is the channel: nobody's");
    let both = [tw.clone(), kick.clone()];
    assert_eq!(em.route_owner([("ninja", &both[..])]), (both.to_vec(), vec![]));
    assert_eq!(em.route_owner([("ninja", &both[..]), ("other", &both[..1])]), (vec![], vec![]), "asked once");
    assert!(em.set_owner(tw.clone(), [("Hi".into(), e("tw"))].into()));
    assert!(em.set_owner(kick.clone(), [("Hi".into(), e("kick")), ("Yo".into(), e("yo"))].into()));
    assert_eq!(&*em.owner_on("ninja", Platform::Twitch, "Hi").unwrap().url, "tw");
    assert_eq!(&*em.owner_on("ninja", Platform::Kick, "Hi").unwrap().url, "kick");
    assert!(em.owner_on("ninja", Platform::Twitch, "Yo").is_none(), "the kick streamer's emote, a twitch line");
    assert!(em.owner_on("ninja", Platform::Youtube, "Hi").is_none());
    assert!(em.owner_on("ninja", Platform::Twitch, "hi").is_none(), "case is exact");
    assert_eq!(&*em.owner_on("other", Platform::Twitch, "Hi").unwrap().url, "tw", "two tabs, one channel");
    assert_eq!(em.owner_all("ninja", Platform::Kick).len(), 2);
    assert_eq!(em.legs_named(Platform::Kick, "Ninja"), vec![kick.clone()], "once, whatever tabs hold it");
    assert_eq!(em.legs_named(Platform::Twitch, "ninja"), vec![tw.clone()]);
    assert!(em.legs_named(Platform::Youtube, "ninja").is_empty());
    // the kick leg came off the tab: its set goes, the twitch one stays
    assert_eq!(em.route_owner([("ninja", &both[..1])]), (vec![], vec![kick.clone()]));
    assert!(em.owner_on("ninja", Platform::Kick, "Hi").is_none() && !em.owners.contains_key(&kick));
    assert_eq!(&*em.owner_on("ninja", Platform::Twitch, "Hi").unwrap().url, "tw");
    assert_eq!(em.route_owner([]), (vec![], vec![tw.clone()]));
    assert!(em.owners.is_empty());
  }

  #[test]
  fn kickify_tokens_kick_emotes_only_and_never_overflows() {
    let mut em = Emotes::default();
    em.set_kick("k", [("peepoDJ".into(), e(&kick_url(37245))), ("odd".into(), e("https://cdn.7tv.app/x"))].into());
    let no = |_: &str| false;
    assert_eq!(em.kickify("k", "peepoDJ turn  it up odd", no), "[emote:37245:peepoDJ] turn  it up odd");
    assert_eq!(em.kickify("other", "peepoDJ", no), "peepoDJ", "another channel's emotes stay words");
    assert_eq!(em.kickify("k", "peepodj [emote:1:x]", no), "peepodj [emote:1:x]");
    assert_eq!(em.kickify("k", "peepoDJ hi", |w| w == "peepoDJ"), "peepoDJ hi", "a word that means another emote");
    let long = "peepoDJ ".repeat(60);
    assert_eq!(em.kickify("k", &long, no), long, "500 chars of words beat a rejected send");
  }

  #[test]
  fn ffz_wins_a_shared_name_in_a_channel_then_7tv_then_bttv_in_every_view() {
    let mut em = Emotes::default();
    em.set(Some("c"), Provider::SevenTv, [("Same".into(), e("c-7tv"))].into());
    em.set(Some("c"), Provider::Bttv, [("Same".into(), e("c-bttv"))].into());
    assert_eq!(&*em.channel("c", "Same").unwrap().url, "c-7tv", "7tv before bttv");
    em.set(Some("c"), Provider::Ffz, [("Same".into(), e("c-ffz"))].into());
    assert_eq!(&*em.channel("c", "Same").unwrap().url, "c-ffz", "ffz before 7tv");
    assert_eq!(em.channel_entries("c", Platform::Twitch).len(), 1);
    assert_eq!(em.channel_all("c", Platform::Twitch).len(), 3);
  }
}
