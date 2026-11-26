// the message box's end of the emote law (src/law, the site's docs/emote-law.md): what a typed word will
// send, and the world tab offers from. the chip in the box, the vouch that leaves with the line and the row
// that comes back are this one answer. `resolve.rs` is its reading twin, and `law::tests` runs the site's
// send cases through `Draft::word`
use std::cell::OnceCell;
use std::collections::HashSet;

use super::block::{Blocked, picture_of};
use super::favorites::Fav;
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
  // every feed the line goes to, `leg`'s among them. a platform's own emote is drawn only on its platform:
  // away from all of its feeds a tile of it goes out as a picture. empty with no room
  pub legs: Vec<Platform>,
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
  // `favs` are the emotes you pinned: each is the copy held here when this room, your own set or the globals
  // hold its picture (at any size), so the law sees one emote and not a look-alike; one nothing here holds
  // is offered from its record, like a search answer. a platform's own emote goes nowhere its platform does
  // not hand it to you: away from a copy held here it could only be sent as text, so it is not offered
  pub fn world(&self, blocked: &Blocked, favs: &'a [Fav]) -> World<'a> {
    let em = self.emotes;
    let mut hidden = HashSet::new();
    let mut pic = |name: &'a str, url: &'a str, zw: bool, p: &'static str, src: Src| {
      let pic = Pic::new(name, url, p, zw, src);
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
    let globals = law::build_globals(&globals);
    let native = room.as_ref().and_then(|r| own.native.get(&r.platform));
    let held = [room.as_ref().map(|r| &r.names), Some(&own.inventory), native, Some(&globals)];
    let mut favorites = Vec::new();
    for f in favs.iter().filter(|f| f.shown) {
      let here = held.iter().flatten().find_map(|idx| idx.get(&*f.name).filter(|e| f.same(e.url)));
      match here {
        Some(e) => favorites.push(e.clone()),
        None if !f.native => {
          let mut p = pic(&f.name, &f.url, f.zero_width, host(&f.url), Src::Favorite);
          p.hash = f.hash();
          favorites.push(p);
        }
        None => {}
      }
    }
    // only the platform of a leg is read: the room's own names are the first leg's, above
    let legs = self
      .legs
      .iter()
      .map(|&p| law::build_room(platform(p), self.chan, law::RoomLists { owner: &[], third: &[], native: &[] }, true))
      .collect();
    World { room, legs, own, globals, favorites, hidden }
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
  // the picture behind the address, whatever size or format it was met at (`block::picture_of`): asked for
  // when two are compared, never for the thousands that are not
  pic: OnceCell<Box<str>>,
  // the server's hash of an upload of ours, where a pin holds one: the same upload met under another host
  pub hash: Option<&'a str>,
}

impl<'a> Pic<'a> {
  fn new(name: &'a str, url: &'a str, p: &'static str, zw: bool, src: Src) -> Self {
    Pic { name, url, p, zw, src, key: format!("{name}\n{url}").into(), pic: OnceCell::new(), hash: None }
  }
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
  // one you pinned that nothing here holds
  Favorite,
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
  // `p` is its address's host, so your saved copy of a twitch or 7tv emote has the host's `p`: the
  // picture is how a copy and the emote it was saved from are told to be one
  fn pic(&self) -> Option<&str> {
    Some(self.pic.get_or_init(|| picture_of(self.url).into()))
  }
  fn hash(&self) -> Option<&str> {
    self.hash
  }
}

pub struct World<'a> {
  pub room: Option<law::Room<Pic<'a>>>,
  // a room for each feed the line goes to, for the platforms it reaches: nothing in it is read
  pub legs: Vec<law::Room<Pic<'a>>>,
  pub own: law::Own<Pic<'a>>,
  pub globals: law::Index<Pic<'a>>,
  // the emotes you pinned, in your order
  pub favorites: Vec<Pic<'a>>,
  // the law keys of the pictures you blocked: never offered, and no look-alike takes their name
  pub hidden: HashSet<String>,
}

// what a tile says (law::mark): the room offers it, and the one corner letter for who sees it if it is sent
// from here (none = everyone, as it is)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Says {
  pub offered: bool,
  pub mark: Option<char>,
}

// the law's send context for the box, asked about tiles one after another
pub struct Judge<'w, 'a> {
  world: &'w World<'a>,
  legs: Vec<&'w law::Room<Pic<'a>>>,
}

impl<'a> World<'a> {
  pub fn judge(&self) -> Judge<'_, 'a> {
    Judge { world: self, legs: self.legs.iter().collect() }
  }

  // the copy of this picture the sets hold, with the provider and place they know it by
  fn held(&self, name: &str, url: &str) -> Option<&Pic<'a>> {
    let native = self.own.native.values();
    let sets = self.room.iter().map(|r| &r.names).chain([&self.own.inventory, &self.own.personal]).chain(native);
    sets.chain([&self.globals]).find_map(|idx| idx.get(name).filter(|e| e.url == url))
  }
}

impl<'w, 'a> Judge<'w, 'a> {
  // an emote the law offered or a set holds. `tier` is where the offer found it
  pub fn says<T: law::Pic>(&self, e: &T, tier: &str) -> Says {
    let w = self.world;
    // nothing is picked: a tile is judged by what its word sends
    let ctx = law::SendCtx {
      room: w.room.as_ref(),
      legs: &self.legs,
      own: &w.own,
      globals: &w.globals,
      picks: None,
      fold: false,
    };
    let s = law::mark::tile_says(&ctx, e, tier);
    Says { offered: s.offered, mark: s.mark }
  }

  // a picker tile, met by its name and address: the copy the sets hold, or else a picture nothing here holds
  // (a search answer, a pin from another room), whose provider is its address's
  pub fn tile(&self, name: &str, url: &str, tier: &str) -> Says {
    match self.world.held(name, url) {
      Some(e) => self.says(e, tier),
      None => self.says(&Pic::new(name, url, host(url), false, Src::Favorite), tier),
    }
  }
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
