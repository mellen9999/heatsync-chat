// the message box's end of the emote law (src/law, the site's docs/emote-law.md): what a typed word will
// send, and the world tab offers from. the chip in the box, the vouch that leaves with the line and the row
// that comes back are this one answer. `resolve.rs` is its reading twin, and `law::tests` runs the site's
// send cases through `Draft::word`
use std::collections::HashSet;

use super::block::Blocked;
use super::personal::Personal;
use super::{Emote, Emotes, Provider};
use crate::heatsync::senders::Senders;
use crate::law;
use crate::model::catalog::Registry;
use crate::model::message::Platform;

// url, native size (0 = unknown), zero-width overlay
pub type Found = (Box<str>, (u16, u16), bool);

// which rule answered a word
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum From {
  // twitch marks the word itself: its own emote, on twitch
  Native,
  Pick,
  Personal,
  Room,
  Own,
  Global,
}

// everything a draft is read against. nothing here is the open tab's by accident: `leg` and `chan` are the
// room the line goes to
pub struct Draft<'a> {
  pub emotes: &'a Emotes,
  pub senders: &'a Senders,
  pub personal: &'a Personal,
  // our twitch id: whose heatsync and 7tv personal emotes "yours" are
  pub me: Option<u64>,
  // the heatsync emotes switch (`renderInventoryEmotes`): off, yours never answer
  pub inventory: bool,
  // the platform the line resolves against: of the feeds it goes to, twitch, else kick, else youtube.
  // None = no room (a post, a reply, a whisper): pick, yours, globals
  pub leg: Option<Platform>,
  // the room's key (the tab's)
  pub chan: &'a str,
  pub now: i64,
}

impl<'a> Draft<'a> {
  // the law's send order, first hit wins:
  //   1. on twitch, twitch's own emote of the name you can use: twitch marks it whatever else we attach
  //   2. the pick made in this draft
  //   3. your 7tv personal emote
  //   4. the room's: the streamer's own heatsync emotes, ffz, 7tv, bttv, and on kick the kick channel's own
  //      (with kick's globals: the one set the app holds for a kick channel, all of it free to send; a
  //      sub-only emote is never loaded). kick draws only what goes out in its format, so anything above
  //      can win a name there (`kickify`)
  //   5. yours
  //   6. a global
  // case is exact. `picks` None = the word as hand-typed
  //
  // `name0`: the word as written goes through every step first; only when nothing answers it is it `name`
  // as an overlay (and twitch marks the exact word only, so `Kappa0` is never twitch's)
  pub fn word(&self, picks: Option<&Registry>, word: &str) -> Option<(Found, From)> {
    self.named(picks, word).map(|(_, found, from)| (found, from))
  }

  // the same answer with the name that gave it: the word itself, or its base when it is read as `name0`
  pub fn named<'w>(&self, picks: Option<&Registry>, word: &'w str) -> Option<(&'w str, Found, From)> {
    if let Some((found, from)) = self.exact(picks, word, false) {
      return Some((word, found, from));
    }
    let base = super::overlay_base(word)?;
    let ((url, size, _), from) = self.exact(picks, base, true)?;
    Some((base, (url, size, true), from))
  }

  fn exact(&self, picks: Option<&Registry>, word: &str, stripped: bool) -> Option<(Found, From)> {
    let e = |e: &Emote, from| Some(((e.url.clone(), (e.w, e.h), e.zero_width), from));
    let em = self.emotes;
    if !stripped
      && self.leg == Some(Platform::Twitch)
      && let Some(x) = em.twitch_global(word).or_else(|| em.sub(word))
    {
      return e(x, From::Native);
    }
    if let Some(r) = picks.and_then(|p| p.get(word)) {
      return Some(((r.url.clone(), (0, 0), r.zero_width), From::Pick));
    }
    if let Some(x) = self.me.and_then(|id| self.personal.peek(id, word)) {
      return e(x, From::Personal);
    }
    let owner = |p| em.owner_on(self.chan, p, word).filter(|_| self.inventory);
    let room = |p| owner(p).or_else(|| em.channel_on(self.chan, p, word));
    if let Some(x) = self.leg.and_then(room) {
      return e(x, From::Room);
    }
    if let Some(x) = self.me.filter(|_| self.inventory).and_then(|id| self.senders.peek(id, word, self.now)) {
      return Some(((x.url.clone(), (x.w, x.h), x.zero_width), From::Own));
    }
    em.global(word).and_then(|x| e(x, From::Global))
  }

  // the same stores as the law's own types, for `law::offer`: built when tab is pressed, never per frame
  pub fn world(&self, blocked: &Blocked) -> World<'a> {
    let em = self.emotes;
    let mut hidden = HashSet::new();
    let mut pic = |name: &'a str, url: &'a str, zw: bool, p: &'static str, src: Src| {
      let pic = Pic { name, url, p, zw, src, key: format!("{name}\n{url}").into() };
      if blocked.contains(name, url) {
        hidden.insert(law::key_of(&pic));
      }
      pic
    };
    let room = self.leg.map(|leg| {
      // this platform's channel only: its third-party sets, and on kick the channel's own
      let all = em.channel_all(self.chan, leg);
      let third: Vec<Pic> = all
        .iter()
        .filter(|(_, p, _)| *p != Provider::Kick)
        .map(|(n, p, e)| pic(n, &e.url, e.zero_width, tag(*p), Src::Channel(*p)))
        .collect();
      let kick = all.iter().filter(|(_, p, _)| *p == Provider::Kick);
      let native: Vec<Pic> = kick.map(|(n, p, e)| pic(n, &e.url, e.zero_width, tag(*p), Src::Channel(*p))).collect();
      // the streamer's own: one list whatever provider each picture came from
      let owner = if self.inventory { em.owner_all(self.chan, leg) } else { Vec::new() };
      let owner: Vec<Pic> =
        owner.into_iter().map(|(n, e)| pic(n, &e.url, e.zero_width, host(&e.url), Src::Owner)).collect();
      law::build_room(platform(leg), self.chan, law::RoomLists { owner: &owner, third: &third, native: &native }, true)
    });
    let all = em.global_all().into_iter();
    let globals: Vec<Pic> = all.map(|(n, p, e)| pic(n, &e.url, e.zero_width, tag(p), Src::Global(p))).collect();
    // what twitch marks by itself for you: its globals and your sub emotes
    let mut twitch: Vec<Pic> = globals.iter().filter(|g| g.src == Src::Global(Provider::Twitch)).cloned().collect();
    twitch.extend(em.sub_entries().into_iter().map(|(n, e)| pic(n, &e.url, e.zero_width, "twitch", Src::Sub)));
    let personal = self.me.map_or_else(Vec::new, |id| self.personal.entries(id));
    let personal: Vec<Pic> =
      personal.into_iter().map(|(n, e)| pic(n, &e.url, e.zero_width, "7tv", Src::Personal)).collect();
    // your own copy of a picture the room or the globals hold is that same picture: its provider is its
    // address's, so it is offered once
    let mine = self.me.filter(|_| self.inventory).map_or_else(Vec::new, |id| self.senders.entries(id, self.now));
    let inventory: Vec<Pic> =
      mine.into_iter().map(|(n, x)| pic(n, &x.url, x.zero_width, host(&x.url), Src::Inventory)).collect();
    let own = law::build_own(&inventory, &personal, &[(law::Platform::Twitch, twitch)]);
    World { room, own, globals: law::build_globals(&globals), hidden }
  }
}

// one emote as the law sees it: an offer is a name on a picture, so one picture under two names is two
#[derive(Clone, Debug)]
pub struct Pic<'a> {
  pub name: &'a str,
  pub url: &'a str,
  pub p: &'static str,
  pub zw: bool,
  pub src: Src,
  // the name and the address
  key: Box<str>,
}

// where a picture was found: what tab's tooltip says about it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
  // the streamer's own heatsync emote
  Owner,
  Channel(Provider),
  Global(Provider),
  Inventory,
  Personal,
  Sub,
}

impl law::Pic for Pic<'_> {
  fn provider(&self) -> &str {
    self.p
  }
  fn id(&self) -> &str {
    &self.key
  }
  fn name(&self) -> &str {
    self.name
  }
  fn zero_width(&self) -> bool {
    self.zw
  }
}

pub struct World<'a> {
  pub room: Option<law::Room<Pic<'a>>>,
  pub own: law::Own<Pic<'a>>,
  pub globals: law::Index<Pic<'a>>,
  // the law keys of the pictures you blocked: never offered, and no look-alike takes their name
  pub hidden: HashSet<String>,
}

fn platform(p: Platform) -> law::Platform {
  match p {
    Platform::Twitch => law::Platform::Twitch,
    Platform::Kick => law::Platform::Kick,
    Platform::Youtube => law::Platform::Youtube,
  }
}

// a set's provider, as the law orders them
pub fn tag(p: Provider) -> &'static str {
  match p {
    Provider::SevenTv => "7tv",
    Provider::Bttv => "bttv",
    Provider::Ffz => "ffz",
    Provider::Twitch => "twitch",
    Provider::Kick => "kick",
  }
}

// the provider a picture's address is on. anything else is heatsync's
pub fn host(url: &str) -> &'static str {
  const HOSTS: [(&str, &str); 5] = [
    ("https://cdn.7tv.app/", "7tv"),
    ("https://cdn.betterttv.net/", "bttv"),
    ("https://cdn.frankerfacez.com/", "ffz"),
    ("https://static-cdn.jtvnw.net/", "twitch"),
    ("https://files.kick.com/", "kick"),
  ];
  HOSTS.iter().find(|(h, _)| url.starts_with(h)).map_or("heatsync", |(_, p)| p)
}
