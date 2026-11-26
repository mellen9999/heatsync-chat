//! who sees a picker tile's picture if you send it from this room — the one answer behind a tile's corner
//! mark and whether the tile is offered at all.
//!
//! a port of the site's `shared/emote-mark.ts` (MARK_VERSION 1). the contract is
//! `src/law/emote-mark.vectors.json`, vendored byte for byte; `src/law/tests/mark.rs` runs every case.
//!
//!   mark   none = everyone · T K Y = that platform's viewers · 7 B F = that addon's users · H = heatsync users
//!   offer  a room answers a typed word with ONE picture: every other picture under that name is not a tile
//!
//! the mark describes what SENDING the tile from this room ships, as the law's send answers it: a click pins
//! the tile's own picture, so your copy of the room's own 7tv picture goes out as the room's (7) and a
//! different picture under the room's name is no tile at all. pure: it reads the law's own send context.

use super::{Pic, Platform, SendCtx, SendFrom, key_of, send_word};

pub const MARK_VERSION: u32 = 1;

/// everyone: a platform's own emote, emoji, chatters · ext: an addon's users · heatsync: heatsync users only
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
  Everyone,
  Ext,
  Heatsync,
}

impl Kind {
  pub fn as_str(self) -> &'static str {
    match self {
      Kind::Everyone => "everyone",
      Kind::Ext => "ext",
      Kind::Heatsync => "heatsync",
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Audience<'a> {
  pub kind: Kind,
  /// the one corner letter; none = everyone sees it as it is
  pub mark: Option<char>,
  /// whose picture it goes out as: the provider the words name ("all twitch", "7tv users"); `hs` = heatsync only
  pub as_: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileSays<'a> {
  /// false = this room answers the tile's word with another picture: the tile is not drawn, in the picker or in tab
  pub offered: bool,
  /// where what it ships sits (room, own, global, catalog, emoji): the answer's tier, the tile's own when its
  /// word has no answer
  pub tier: &'a str,
  pub kind: Kind,
  pub mark: Option<char>,
  pub as_: &'a str,
}

const HEATSYNC: Audience<'static> = Audience { kind: Kind::Heatsync, mark: Some('H'), as_: "hs" };

// a client's own names for our uploads and your saved copies
fn canon(provider: &str) -> &str {
  match provider {
    "heatsync" | "inventory" => "hs",
    p => p,
  }
}

fn silent(p: &str) -> bool {
  p == "emoji" || p == "chatter"
}

fn native(p: &str) -> Option<char> {
  match p {
    "twitch" => Some('T'),
    "kick" => Some('K'),
    "youtube" => Some('Y'),
    _ => None,
  }
}

fn addon(p: &str) -> Option<char> {
  match p {
    "7tv" => Some('7'),
    "bttv" => Some('B'),
    "ffz" => Some('F'),
    _ => None,
  }
}

/// who sees a picture of `provider` that sits in `tier` (where the law's offer found it), with no room to
/// answer for it. a tile asks `tile_says`, which first finds what the room would really send.
///   emoji, chatters           everyone, no mark
///   not loaded here           heatsync users only (a search answer, a favorite from another room)
///   a platform's own emote    that platform's viewers
///   yours, or a heatsync one  heatsync users only
///   an addon's emote          that addon's users (an unknown provider: an addon's, with no letter)
pub fn audience<'a>(provider: &'a str, tier: Option<&str>) -> Audience<'a> {
  let p = canon(provider);
  if silent(p) {
    return Audience { kind: Kind::Everyone, mark: None, as_: p };
  }
  if tier == Some("catalog") {
    return HEATSYNC;
  }
  if let Some(mark) = native(p) {
    return Audience { kind: Kind::Everyone, mark: Some(mark), as_: p };
  }
  if tier == Some("own") || p == "hs" {
    return HEATSYNC;
  }
  Audience { kind: Kind::Ext, mark: addon(p), as_: if p.is_empty() { "ext" } else { p } }
}

/// one picture: the same `pic` (default: the law's key), or the same upload by its hash
pub fn same_picture<E: Pic>(a: &E, b: &E) -> bool {
  let pic = |e: &E| e.pic().map_or_else(|| key_of(e), String::from);
  pic(a) == pic(b) || a.hash().is_some_and(|h| !h.is_empty() && Some(h) == b.hash())
}

/// where the answer to a typed word sits, as a tier. a platform's own emote you may use (native) sits where
/// the tile does, but a catalog tile it answers is yours
fn sits(from: SendFrom, tier: &str) -> &str {
  match from {
    SendFrom::Room => "room",
    SendFrom::Global => "global",
    SendFrom::Own | SendFrom::Personal => "own",
    SendFrom::Native if tier == "catalog" => "own",
    SendFrom::Native | SendFrom::Pick => tier,
  }
}

/// what a tile says. `ctx` is the law's send context for the box the line is written in (its room, every leg,
/// your sets, the globals; picks are never read: a tile is judged by what its word sends). `tier` is where the
/// law's offer found the tile.
///   offered   no room, or the room's answer for the word is this very picture (or the word has no answer)
///   mark      that answer's class, never the copy the tile happens to be: your copy of the room's own picture
///             is the room's; an answer from your own set ships as yours (H)
///   a platform's own emote is drawn only on that platform: where the send reaches none of its legs it travels
///   as a picture and only heatsync users see it
pub fn tile_says<'a, E: Pic>(ctx: &SendCtx<'a, E>, e: &'a E, tier: &'a str) -> TileSays<'a> {
  let says = |offered, tier, a: Audience<'a>| TileSays { offered, tier, kind: a.kind, mark: a.mark, as_: a.as_ };
  if silent(canon(e.provider())) {
    return says(true, tier, audience(e.provider(), None));
  }
  let hit = if ctx.room.is_some() && !e.name().is_empty() {
    let mut own = ctx.clone();
    own.picks = None;
    send_word(e.name(), &own)
  } else {
    None
  };
  let same = hit.as_ref().is_some_and(|h| same_picture(h.emote, e));
  let (at, mut said) = match &hit {
    Some(h) if same => {
      let at = sits(h.from, tier);
      let a = if matches!(h.from, SendFrom::Own | SendFrom::Personal) {
        HEATSYNC
      } else {
        audience(h.emote.provider(), Some(at))
      };
      (at, a)
    }
    _ => (tier, audience(e.provider(), Some(tier))),
  };
  // a platform's own emote reaches only that platform: no leg of it in this send, so it goes out as a picture
  if said.kind == Kind::Everyone
    && let Some(plat) = Platform::parse(said.as_)
  {
    let reaches = if ctx.legs.is_empty() {
      ctx.room.is_none_or(|r| r.platform == plat)
    } else {
      ctx.legs.iter().any(|r| r.platform == plat)
    };
    if !reaches {
      said = HEATSYNC;
    }
  }
  says(hit.is_none() || same, at, said)
}
