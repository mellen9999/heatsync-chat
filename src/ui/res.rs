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
    }
  }

  pub fn set_scale(&mut self, scale: f32) {
    self.text.set_scale(scale);
    self.logo = Logo::new(self.text.line_px as u32);
    self.images.set_scale(scale);
  }
}
