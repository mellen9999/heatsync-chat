// name colors: twitch defaults for colorless users + the ext's readability lift
// (ext src/lib/utils.js boostReadability: hsl lightness only, until 3:1 on the #444444 zebra)
use crate::ui::theme;

// twitch's default palette, picked by login hash like the web client does
const DEFAULTS: [u32; 15] = [
  0xff0000, 0x0000ff, 0x008000, 0xb22222, 0xff7f50, 0x9acd32, 0xff4500, 0x2e8b57, 0xdaa520, 0xd2691e, 0x5f9ea0,
  0x1e90ff, 0xff69b4, 0x8a2be2, 0x00ff7f,
];

pub fn default_for(login: &str) -> u32 {
  let h = login.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
  DEFAULTS[(h % DEFAULTS.len() as u32) as usize]
}

// heatsync's own name colors, for people with none of their own (youtube has no native color). picked by
// djb2 over the lowercased name, the ext's `hsUsernameColor` and the site's `usernameColor` byte for byte, so
// a chatter is the same color in all three
const HEATSYNC: [u32; 15] = [
  0xff7a7a, 0xff9d4d, 0xffd24d, 0xb3e833, 0x5fd75f, 0x33d9b2, 0x5fbfd7, 0x69a8ff, 0xa675ff, 0xd76bcb, 0xff6e9c,
  0xff8fc0, 0xe57373, 0xf0a23a, 0x7bc46c,
];

// `login` as the line holds it (lowercased; a youtube name keeps its `@`)
pub fn heatsync_for(login: &str) -> u32 {
  // js: h = ((h << 5) + h + charCodeAt(i)) | 0 over utf-16 units, then Math.abs(h) % 15
  let h = login.encode_utf16().fold(5381i32, |h, c| h.wrapping_shl(5).wrapping_add(h).wrapping_add(c as i32));
  HEATSYNC[(h.unsigned_abs() as usize) % HEATSYNC.len()]
}

pub fn parse_hex(s: &str) -> Option<u32> {
  let h = s.strip_prefix('#').filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))?;
  match h.len() {
    6 => u32::from_str_radix(h, 16).ok(),
    3 => {
      let v = u32::from_str_radix(h, 16).ok()?;
      let (r, g, b) = (v >> 8 & 0xf, v >> 4 & 0xf, v & 0xf);
      Some(((r * 0x11) << 16) | ((g * 0x11) << 8) | (b * 0x11))
    }
    _ => None,
  }
}

fn luminance(rgb: u32) -> f32 {
  let lin = |c: u32| {
    let v = c as f32 / 255.0;
    if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
  };
  0.2126 * lin(rgb >> 16 & 0xff) + 0.7152 * lin(rgb >> 8 & 0xff) + 0.0722 * lin(rgb & 0xff)
}

fn min_luminance() -> f32 {
  // 3:1 against the zebra stripe, expressed as a floor against black
  let ratio = 3.0 * (luminance(theme::ZEBRA) + 0.05) / 0.05;
  ratio * 0.05 - 0.05
}

pub fn readable(rgb: u32) -> u32 {
  let min_l = min_luminance();
  if luminance(rgb) >= min_l {
    return rgb;
  }
  let (r, g, b) = ((rgb >> 16 & 0xff) as f32 / 255.0, (rgb >> 8 & 0xff) as f32 / 255.0, (rgb & 0xff) as f32 / 255.0);
  let max = r.max(g).max(b);
  let min = r.min(g).min(b);
  let l = (max + min) / 2.0;
  let (mut h, mut s) = (0.0, 0.0);
  if max != min {
    let d = max - min;
    s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    h = if max == r {
      (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
      (b - r) / d + 2.0
    } else {
      (r - g) / d + 4.0
    };
    h /= 6.0;
  }
  let hue2rgb = |p: f32, q: f32, mut t: f32| {
    if t < 0.0 {
      t += 1.0
    }
    if t > 1.0 {
      t -= 1.0
    }
    if t < 1.0 / 6.0 {
      p + (q - p) * 6.0 * t
    } else if t < 0.5 {
      q
    } else if t < 2.0 / 3.0 {
      p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
      p
    }
  };
  let byte = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u32;
  let at = |ll: f32| {
    let q = if ll < 0.5 { ll * (1.0 + s) } else { ll + s - ll * s };
    let p = 2.0 * ll - q;
    byte(hue2rgb(p, q, h + 1.0 / 3.0)) << 16 | byte(hue2rgb(p, q, h)) << 8 | byte(hue2rgb(p, q, h - 1.0 / 3.0))
  };
  let mut lt = l + 0.02;
  while lt < 1.0 {
    let out = at(lt);
    if luminance(out) >= min_l {
      return out;
    }
    lt += 0.02;
  }
  theme::WHITE
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hex() {
    assert_eq!(parse_hex("#FF8700"), Some(0xff8700));
    assert_eq!(parse_hex("#f80"), Some(0xff8800));
    assert_eq!(parse_hex("ff8700"), None);
    assert_eq!(parse_hex("#ggg"), None);
  }

  // vectors from the ext's `hsUsernameColor` (src/multichat/paints.js): a negative hash, a non-ascii name, utf-16
  // pairs (an emoji is two units), and a name long enough to wrap 32 bits
  #[test]
  fn heatsync_colors_match_the_ext_byte_for_byte() {
    for (login, want) in [
      ("@alpha_one", 0x33d9b2),
      ("@beta-9", 0xff7a7a),
      ("@gamma.three", 0x33d9b2),
      ("@delta", 0x5fd75f),
      ("@😀smile", 0xf0a23a),
      ("@محمد", 0xff6e9c),
      ("@zeta1234567890abcdefghijklmnop", 0x5fd75f),
      ("@éclair", 0xa675ff),
      ("", 0xff8fc0),
    ] {
      assert_eq!(heatsync_for(login), want, "{login}");
    }
  }

  #[test]
  fn readable_keeps_bright_lifts_dark() {
    assert_eq!(readable(0xffff00), 0xffff00);
    assert_eq!(readable(0xffffff), 0xffffff);
    let blue = readable(0x0000ff);
    assert_ne!(blue, 0x0000ff);
    // still blue-dominant after the lift
    assert!((blue & 0xff) > (blue >> 16 & 0xff));
    assert!(luminance(blue) >= min_luminance());
    assert!(luminance(readable(0x000000)) >= min_luminance());
  }
}
