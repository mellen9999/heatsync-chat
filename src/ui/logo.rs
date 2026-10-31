// the heatsync mark on the picker button: color on black, black on white when hovered/active.
// embedded, decoded once per size (the 96px file for hidpi)
use crate::images::decode::{Decoded, decode};

const COLOR_48: &[u8] = include_bytes!("../../assets/icon-48.png");
const BLACK_48: &[u8] = include_bytes!("../../assets/icon-48-black.png");
const COLOR_96: &[u8] = include_bytes!("../../assets/icon-96.png");
// no black 96 ships; the 48 black one is scaled up from the same art
const BLACK_96: &[u8] = BLACK_48;

pub struct Logo {
  pub color: Option<Decoded>,
  pub black: Option<Decoded>,
}

impl Logo {
  // both variants scaled to `h` px tall
  pub fn new(h: u32) -> Self {
    let big = h > 48;
    let (c, b) = if big { (COLOR_96, BLACK_96) } else { (COLOR_48, BLACK_48) };
    Self { color: decode(c, h).ok(), black: decode(b, h).ok() }
  }
}

// the window icon: 96px rgba straight from the art
pub fn window_rgba() -> Option<(Vec<u8>, u32)> {
  let d = decode(COLOR_96, 96).ok()?;
  Some((d.frames.first()?.rgba.to_vec(), d.w))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn embedded_art_decodes_at_every_size() {
    for h in [16, 20, 30, 40, 48, 60, 96] {
      let l = Logo::new(h);
      for d in [l.color.as_ref().unwrap(), l.black.as_ref().unwrap()] {
        assert_eq!(d.h, h);
        assert_eq!(d.frames[0].rgba.len() as u32, d.w * d.h * 4);
      }
    }
    let (rgba, w) = window_rgba().unwrap();
    assert_eq!(rgba.len() as u32, w * w * 4);
  }
}
