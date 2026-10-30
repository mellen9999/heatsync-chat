// shared render resources: fonts/glyphs, decoded images, emote catalogs
use crate::emotes::Emotes;
use crate::images::Images;

use super::text::Text;

pub const EMOTE_PX: f32 = 28.0;

pub struct Res {
  pub text: Text,
  pub images: Images,
  pub emotes: Emotes,
}

impl Res {
  pub fn new(scale: f32) -> Self {
    Self {
      text: Text::new(scale),
      images: Images::new((EMOTE_PX * scale).round() as u32),
      emotes: Emotes::default(),
    }
  }

  pub fn set_scale(&mut self, scale: f32) {
    self.text.set_scale(scale);
    self.images.set_target_h((EMOTE_PX * scale).round() as u32);
  }
}
