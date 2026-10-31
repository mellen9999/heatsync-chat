// shared render resources: fonts/glyphs, decoded images, emote catalogs, sender inventories
use crate::badges::Badges;
use crate::cosmetics::Cosmetics;
use crate::emotes::Emotes;
use crate::emotes::block::Blocked;
use crate::heatsync::senders::Senders;
use crate::images::Images;

use super::logo::Logo;
use super::name_art::NameArts;
use super::text::Text;

pub const EMOTE_PX: u16 = 28;
pub const BADGE_PX: u16 = 18;
// tallest inline picture in a chat line (the ext's max-height)
pub const EMBED_PX: u16 = 220;

pub struct Res {
  pub text: Text,
  pub images: Images,
  pub emotes: Emotes,
  pub badges: Badges,
  pub senders: Senders,
  pub cosmetics: Cosmetics,
  pub arts: NameArts,
  pub blocked: Blocked,
  pub logo: Logo,
  // inline pictures under chat lines (ext mediaEmbedsEnabled)
  pub media: bool,
  // host-less and defanged links count as links (ext partialLinksEnabled)
  pub partial_links: bool,
}

impl Res {
  pub fn new(scale: f32) -> Self {
    let text = Text::new(scale);
    Self {
      logo: Logo::new(text.line_px as u32),
      text,
      images: Images::new(scale),
      emotes: Emotes::default(),
      badges: Badges::default(),
      senders: Senders::default(),
      cosmetics: Cosmetics::default(),
      arts: NameArts::default(),
      blocked: Blocked::load(),
      media: true,
      partial_links: true,
    }
  }

  pub fn set_scale(&mut self, scale: f32) {
    self.text.set_scale(scale);
    self.logo = Logo::new(self.text.line_px as u32);
    self.images.set_scale(scale);
  }
}
