// shared render resources: fonts/glyphs, decoded images, emote catalogs, sender inventories
use crate::badges::Badges;
use crate::emotes::Emotes;
use crate::heatsync::senders::Senders;
use crate::images::Images;

use super::text::Text;

pub const EMOTE_PX: u16 = 28;
pub const BADGE_PX: u16 = 18;

pub struct Res {
  pub text: Text,
  pub images: Images,
  pub emotes: Emotes,
  pub badges: Badges,
  pub senders: Senders,
}

impl Res {
  pub fn new(scale: f32) -> Self {
    Self {
      text: Text::new(scale),
      images: Images::new(scale),
      emotes: Emotes::default(),
      badges: Badges::default(),
      senders: Senders::default(),
    }
  }

  pub fn set_scale(&mut self, scale: f32) {
    self.text.set_scale(scale);
    self.images.set_scale(scale);
  }
}
