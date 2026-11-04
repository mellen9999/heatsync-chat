// shared render resources: fonts/glyphs, decoded images, emote catalogs, sender inventories
use crate::badges::Badges;
use crate::cosmetics::Cosmetics;
use crate::emotes::Emotes;
use crate::emotes::block::Blocked;
use crate::emotes::resolve::Sources;
use crate::heatsync::senders::Senders;
use crate::images::Images;
use crate::model::chatters::Chatters;
use crate::model::color;
use crate::model::settings::{Animate, Settings, TimeFmt};

use super::logo::Logo;
use super::name_art::NameArts;
use super::text::Text;

pub const EMOTE_PX: u16 = 28;
pub const BADGE_PX: u16 = 18;
// the picture beside a name (ext .hs-mc-avatar: 18px square)
pub const AVATAR_PX: u16 = 18;
// tallest inline picture in a chat line (the ext's max-height)
pub const EMBED_PX: u16 = 220;
// a feed thread's picture, and each of several in a grid (ext .hs-feed-media, .hs-feed-media-multi)
pub const MEDIA_PX: u16 = 320;
pub const MEDIA_MULTI_PX: u16 = 180;
// a link preview card's still
pub const CARD_PX: u16 = 64;

// how chat lines look, from the settings: one value, rebuilt whenever one of them moves
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
  pub timestamps: bool,
  pub twelve: bool,
  pub zebra: bool,
  pub readable: bool,
  pub tags: bool,
  // times the base emote size
  pub emote_mul: u16,
  // times the font size for unicode emoji in a line's words
  pub emoji_mul: u16,
  // times the base line height
  pub spacing: f32,
  pub animate_emotes: bool,
  pub dim_timeouts: bool,
  pub bttv_badges: bool,
  pub ffz_badges: bool,
  pub chatterino_badges: bool,
  // badges as text chips, not pictures
  pub text_badges: bool,
  // a picture beside each name
  pub avatars: bool,
  // emotes from a heatsync inventory draw (renderInventoryEmotes)
  pub inventory_emotes: bool,
}

impl Look {
  pub fn of(s: &Settings) -> Self {
    Self {
      timestamps: s.timestamps,
      twelve: s.time_fmt == TimeFmt::H12,
      zebra: s.zebra,
      readable: s.readable,
      tags: s.platform_tags,
      emote_mul: s.emote_size.mul(),
      emoji_mul: s.emoji_size.mul(),
      spacing: s.line_height.ratio(),
      animate_emotes: s.animate_emotes == Animate::Always,
      dim_timeouts: s.dim_timeouts,
      bttv_badges: s.bttv_badges,
      ffz_badges: s.ffz_badges,
      chatterino_badges: s.chatterino_badges,
      text_badges: s.text_badges,
      avatars: s.avatars,
      inventory_emotes: s.render_inventory,
    }
  }

  // which per-chatter badge sets show
  pub fn third(&self) -> crate::badges::Third {
    crate::badges::Third { bttv: self.bttv_badges, ffz: self.ffz_badges, chatterino: self.chatterino_badges }
  }

  // the emote box height in chat, before the display scale
  pub fn emote_px(&self) -> u16 {
    EMOTE_PX * self.emote_mul
  }

  // a name's color as drawn: lifted until it reads on black, or as the platform sent it
  pub fn ink(&self, rgb: u32) -> u32 {
    if self.readable { color::readable(rgb) } else { rgb }
  }
}

impl Default for Look {
  fn default() -> Self {
    Self::of(&Settings::default())
  }
}

pub struct Res {
  pub text: Text,
  pub images: Images,
  pub emotes: Emotes,
  pub badges: Badges,
  pub senders: Senders,
  pub cosmetics: Cosmetics,
  pub avatars: crate::avatars::Avatars,
  pub arts: NameArts,
  pub blocked: Blocked,
  // who the names in message text mean (their color, paint)
  pub chatters: Chatters,
  pub logo: Logo,
  pub look: Look,
  // link previews under chat lines, per link
  pub previews: crate::model::preview::Previews,
  // inline pictures under chat lines (ext mediaEmbedsEnabled)
  pub media: bool,
  // host-less and defanged links count as links (ext partialLinksEnabled)
  pub partial_links: bool,
  // urls are links at all (ext linksEnabled)
  pub links: bool,
}

impl Res {
  pub fn new(scale: f32) -> Self {
    let text = Text::new(scale);
    Self {
      logo: Logo::new(text.line_px as u32),
      look: Look::default(),
      text,
      images: Images::new(scale),
      emotes: Emotes::default(),
      badges: Badges::default(),
      senders: Senders::default(),
      cosmetics: Cosmetics::default(),
      avatars: Default::default(),
      arts: NameArts::default(),
      blocked: Blocked::load(),
      chatters: Chatters::default(),
      previews: Default::default(),
      media: true,
      partial_links: true,
      links: true,
    }
  }

  // the settings that shape drawing: the look, and which provider wins a shared emote name
  pub fn apply_look(&mut self, s: &Settings) {
    self.look = Look::of(s);
    self.emotes.set_order(s.emote_provider.order());
  }

  // what a word may become an emote from (`emotes::resolve`, the one render chokepoint)
  pub fn sources(&mut self) -> Sources<'_> {
    Sources { emotes: &self.emotes, senders: &mut self.senders, inventory: self.look.inventory_emotes }
  }

  pub fn set_scale(&mut self, scale: f32) {
    self.text.set_scale(scale);
    self.logo = Logo::new(self.text.line_px as u32);
    self.images.set_scale(scale);
  }
}
