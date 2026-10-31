// painted usernames. the name's glyph coverage is shaded per pixel with the paint's color function
// (cpu only), drop shadows composited behind, and the result cached as a small rgba bitmap per
// (chatter, name, paint): rows draw it over the spot where the plain name would be.
// 7tv image paints use the first frame (their `1x_static.webp`): nothing here ever animates
use std::collections::HashMap;

pub mod anim;
mod fill;
mod motion;
pub mod raster;
pub mod scene;
mod scene_cat;
mod stroke;

use self::anim::{AnimArt, Scratch};
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;
use crate::cosmetics::paint::{Argb, Fill, Paint, Shadow};

// bitmaps are ~8KB; 256 of them stay well under 2MB
const MAX_ARTS: usize = 256;
// image paints decode to this logical height; the picture is stretched over the name anyway
pub const PAINT_IMG_PX: u16 = 20;

pub struct Art {
  pub w: i32,
  pub h: i32,
  // baseline inside the bitmap, to line it up with the row's own baseline
  pub base: f32,
  // straight-alpha rgba8
  pub rgba: Box<[u8]>,
}

#[derive(Hash, PartialEq, Eq, Clone)]
pub struct ArtKey {
  pub uid: crate::cosmetics::id::PaintId,
  pub color: u32,
  pub font_px: u32,
  // the paint is shared by everyone wearing it: its address names it
  pub paint: usize,
}

struct Entry {
  nick: Box<str>,
  art: Art,
  // built with the paint's picture (or none was needed)
  complete: bool,
  used: u64,
}

// animated names alive at once (their scene tiles are the heavy part)
const MAX_ANIMS: usize = 64;

struct AnimEntry {
  nick: Box<str>,
  art: AnimArt,
  // the frame when nothing moves (paint motion off, or a scene that holds still)
  still: Option<Art>,
  used: u64,
}

// seconds on the wall clock every copy of a name shares: all frames of one paint line up
pub fn clock() -> f32 {
  use std::sync::OnceLock;
  static T0: OnceLock<std::time::Instant> = OnceLock::new();
  T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f32()
}

pub struct NameArts {
  map: HashMap<ArtKey, Entry>,
  anims: HashMap<ArtKey, AnimEntry>,
  scratch: Scratch,
  tick: u64,
  // paint motion (the ext's animatePaints): off draws every paint's first frame
  motion: bool,
}

impl Default for NameArts {
  fn default() -> Self {
    NameArts { map: HashMap::new(), anims: HashMap::new(), scratch: Scratch::new(), tick: 0, motion: true }
  }
}

impl NameArts {
  // `ready`: the picture an image paint needs is decoded. a bitmap built without it is redone once it is.
  // the nick is checked on every hit, so a changed display name can never wear a stale bitmap
  pub fn get(&mut self, key: &ArtKey, nick: &str, ready: bool, build: impl FnOnce() -> Art) -> &Art {
    self.tick += 1;
    let tick = self.tick;
    let hit = self.map.get(key).is_some_and(|e| *e.nick == *nick && (e.complete || !ready));
    if !hit {
      if self.map.len() >= MAX_ARTS && !self.map.contains_key(key) {
        let oldest = self.map.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone());
        if let Some(k) = oldest {
          self.map.remove(&k);
        }
      }
      self.map.insert(key.clone(), Entry { nick: nick.into(), art: build(), complete: ready, used: tick });
    }
    let e = self.map.get_mut(key).expect("present");
    e.used = tick;
    &e.art
  }

  #[cfg(test)]
  pub fn len(&self) -> usize {
    self.map.len()
  }

  // flipping paint motion rebuilds every animated name
  pub fn set_motion(&mut self, on: bool) {
    if self.motion != on {
      self.motion = on;
      self.anims.clear();
    }
  }

  // an animated name's art, built once per (chatter, name, paint); a changed display name rebuilds it
  pub fn anim(&mut self, key: &ArtKey, nick: &str, build: impl FnOnce(bool) -> AnimArt) -> &AnimArt {
    self.tick += 1;
    let hit = self.anims.get(key).is_some_and(|e| *e.nick == *nick);
    if !hit {
      if self.anims.len() >= MAX_ANIMS && !self.anims.contains_key(key) {
        let oldest = self.anims.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone());
        if let Some(k) = oldest {
          self.anims.remove(&k);
        }
      }
      let art = build(self.motion);
      let still = (!art.animated()).then(|| {
        let px = art.render(None, &mut self.scratch);
        Art { w: art.w, h: art.h, base: art.base, rgba: unpremultiply(px) }
      });
      self.anims.insert(key.clone(), AnimEntry { nick: nick.into(), art, still, used: self.tick });
    }
    let e = self.anims.get_mut(key).expect("present");
    e.used = self.tick;
    &e.art
  }

  // the still frame of a name that does not move
  pub fn still(&self, key: &ArtKey) -> Option<&Art> {
    self.anims.get(key)?.still.as_ref()
  }

  // frame of an animated name at wall time `t`, premultiplied
  pub fn frame(&mut self, key: &ArtKey, t: f32) -> Option<(&AnimArt, &[[u8; 4]])> {
    let e = self.anims.get(key)?;
    let px = e.art.render(Some(t), &mut self.scratch);
    Some((&e.art, px))
  }
}

fn unpremultiply(px: &[[u8; 4]]) -> Box<[u8]> {
  let mut out = Vec::with_capacity(px.len() * 4);
  for p in px {
    let a = p[3] as u32;
    for &c in &p[..3] {
      out.push((c as u32 * 255).checked_div(a).map_or(0, |v| v.min(255) as u8));
    }
    out.push(p[3]);
  }
  out.into_boxed_slice()
}

// the picture an image paint is stretched from: rgba8, width, height
pub type Picture<'a> = (&'a [u8], u32, u32);

fn blend(base: u32, top: Argb) -> [f32; 3] {
  let a = (top >> 24) as f32 / 255.0;
  let ch = |s: u32| {
    let (b, t) = (((base >> s) & 255) as f32, ((top >> s) & 255) as f32);
    b + (t - b) * a
  };
  [ch(16), ch(8), ch(0)]
}

fn box_blur(buf: &mut [f32], w: usize, h: usize, r: usize) {
  if r == 0 {
    return;
  }
  let n = (2 * r + 1) as f32;
  let mut tmp = vec![0f32; buf.len()];
  for _ in 0..3 {
    for y in 0..h {
      for x in 0..w {
        let (lo, hi) = (x.saturating_sub(r), (x + r).min(w - 1));
        tmp[y * w + x] = buf[y * w + lo..=y * w + hi].iter().sum::<f32>() / n;
      }
    }
    for y in 0..h {
      for x in 0..w {
        let (lo, hi) = (y.saturating_sub(r), (y + r).min(h - 1));
        buf[y * w + x] = (lo..=hi).map(|yy| tmp[yy * w + x]).sum::<f32>() / n;
      }
    }
  }
}

// one drop shadow: the glyph alpha shifted, blurred, tinted. css blur radius = 2σ
fn shadow_layer(alpha: &[f32], w: usize, h: usize, s: &Shadow) -> Vec<f32> {
  // the glyph alpha shifted by (dx, dy), fractional offsets split over the four neighbours
  let (fx, fy) = (s.dx.floor(), s.dy.floor());
  let (kx, ky) = (s.dx - fx, s.dy - fy);
  let (ix, iy) = (fx as i32, fy as i32);
  let at = |x: i32, y: i32| -> f32 {
    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h { alpha[y as usize * w + x as usize] } else { 0.0 }
  };
  let mut a = vec![0f32; w * h];
  for y in 0..h as i32 {
    for x in 0..w as i32 {
      let (sx, sy) = (x - ix, y - iy);
      let v = if kx == 0.0 && ky == 0.0 {
        at(sx, sy)
      } else {
        let top = at(sx, sy) * (1.0 - kx) + at(sx - 1, sy) * kx;
        let bot = at(sx, sy - 1) * (1.0 - kx) + at(sx - 1, sy - 1) * kx;
        top * (1.0 - ky) + bot * ky
      };
      a[y as usize * w + x as usize] = v;
    }
  }
  let sigma = s.blur / 2.0;
  box_blur(&mut a, w, h, (sigma - 0.5).round().max(0.0) as usize);
  let k = (s.color >> 24) as f32 / 255.0;
  a.iter_mut().for_each(|v| *v *= k);
  a
}

// the name's glyph coverage (0..1) in a w×h box, and the baseline inside it
pub struct Cov {
  pub w: i32,
  pub h: i32,
  pub base: f32,
  pub alpha: Vec<f32>,
}

pub fn coverage(text: &mut Text, nick: &str, w: i32) -> Cov {
  let layout = text.layout(nick, &[Span { range: 0..nick.len(), ink: Ink(theme::WHITE), bold: true }], &[], 1e5);
  let (w, h) = (w.max(1), (text.line_px as i32).max(1));
  let mut cov = Canvas::new(w, h);
  let area = Rect::new(0, 0, w, h);
  text.draw(&layout, &mut cov, 0.0, 0.0, area, |_, _| {});
  let base = layout.lines().next().map_or(0.0, |l| l.metrics().baseline);
  Cov { w, h, base, alpha: cov.px.iter().map(|p| ((p >> 16) & 255) as f32 / 255.0).collect() }
}

// `w` is the name's width in the row, `base` its color (the paint's underlay)
pub fn build(text: &mut Text, nick: &str, w: i32, base: u32, paint: &Paint, pic: Option<Picture>) -> Art {
  let Cov { w, h, base: baseline, alpha } = coverage(text, nick, w);
  let (wu, hu) = (w as usize, h as usize);

  // the painted glyphs, premultiplied: [r, g, b, a] in 0..1
  let mut layers: Vec<Vec<[f32; 4]>> = Vec::with_capacity(1 + paint.shadows.len());
  let mut glyph = vec![[0f32; 4]; wu * hu];
  for y in 0..hu {
    for x in 0..wu {
      let a = alpha[y * wu + x];
      if a <= 0.0 {
        continue;
      }
      let rgb = match (&paint.fill, pic) {
        (Fill::Image { .. }, Some((px, iw, ih))) if iw > 0 && ih > 0 => {
          let (sx, sy) = ((x as u32 * iw / w as u32).min(iw - 1), (y as u32 * ih / h as u32).min(ih - 1));
          let i = ((sy * iw + sx) * 4) as usize;
          match px.get(i..i + 4) {
            Some(p) => blend(base, (p[3] as u32) << 24 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32),
            None => blend(base, 0),
          }
        }
        (f, _) => match paint.gradient(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32) {
          Some(c) if !matches!(f, Fill::None) => blend(base, c),
          _ => blend(base, 0),
        },
      };
      glyph[y * wu + x] = [rgb[0] / 255.0 * a, rgb[1] / 255.0 * a, rgb[2] / 255.0 * a, a];
    }
  }
  layers.push(glyph);
  for s in &paint.shadows {
    let a = shadow_layer(&alpha, wu, hu, s);
    let c = [(s.color >> 16 & 255) as f32 / 255.0, (s.color >> 8 & 255) as f32 / 255.0, (s.color & 255) as f32 / 255.0];
    layers.push(a.iter().map(|&a| [c[0] * a, c[1] * a, c[2] * a, a]).collect());
  }
  // css drop-shadow list: the first sits closest to the glyphs, each later one further behind
  let mut out = vec![0u8; wu * hu * 4];
  for i in 0..wu * hu {
    let mut acc = [0f32; 4];
    for layer in layers.iter().rev() {
      let l = layer[i];
      let k = 1.0 - l[3];
      acc = [l[0] + acc[0] * k, l[1] + acc[1] * k, l[2] + acc[2] * k, l[3] + acc[3] * k];
    }
    if acc[3] > 0.0 {
      for c in 0..3 {
        out[i * 4 + c] = (acc[c] / acc[3] * 255.0).round().clamp(0.0, 255.0) as u8;
      }
      out[i * 4 + 3] = (acc[3] * 255.0).round().clamp(0.0, 255.0) as u8;
    }
  }
  Art { w, h, base: baseline, rgba: out.into_boxed_slice() }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  const RED: i64 = 0xff0000ffu32 as i32 as i64;
  const BLUE: i64 = 0x0000ffffu32 as i32 as i64;

  fn paint(v: serde_json::Value) -> Paint {
    Paint::parse(&v).unwrap()
  }

  fn grad() -> Paint {
    paint(
      json!({"id": "G", "function": "linear-gradient", "angle": 90, "stops": [{"at": 0, "color": RED}, {"at": 1, "color": BLUE}]}),
    )
  }

  fn width(text: &mut Text, nick: &str) -> i32 {
    text
      .layout(nick, &[Span { range: 0..nick.len(), ink: Ink(theme::WHITE), bold: true }], &[], 1e5)
      .full_width()
      .ceil() as i32
  }

  // mean color of the opaque pixels in columns x0..x1
  fn mean(a: &Art, x0: i32, x1: i32) -> [u32; 3] {
    let (mut n, mut s) = (0u32, [0u32; 3]);
    for y in 0..a.h {
      for x in x0..x1 {
        let p = &a.rgba[((y * a.w + x) * 4) as usize..][..4];
        if p[3] > 200 {
          n += 1;
          (0..3).for_each(|c| s[c] += p[c] as u32);
        }
      }
    }
    assert!(n > 0, "no ink in {x0}..{x1}");
    [s[0] / n, s[1] / n, s[2] / n]
  }

  #[test]
  fn gradient_shades_the_glyphs_left_red_right_blue() {
    let mut t = Text::new(1.0);
    let w = width(&mut t, "wwwwwwww");
    let a = build(&mut t, "wwwwwwww", w, theme::WHITE, &grad(), None);
    assert_eq!((a.w, a.rgba.len() as i32), (w, w * a.h * 4));
    let (l, r) = (mean(&a, 0, w / 4), mean(&a, 3 * w / 4, w));
    assert!(l[0] > 180 && l[2] < 80, "left {l:?}");
    assert!(r[2] > 180 && r[0] < 80, "right {r:?}");
  }

  #[test]
  fn shadow_only_paint_keeps_name_color_and_adds_halo() {
    let mut t = Text::new(1.0);
    let w = width(&mut t, "iii");
    let p = paint(json!({"id": "S", "function": "linear-gradient", "stops": [],
      "shadows": [{"x_offset": 0, "y_offset": 0, "radius": 4, "color": 0xff0000ffu32 as i32}]}));
    let a = build(&mut t, "iii", w, theme::GREEN, &p, None);
    // some pixel is faint red halo (not on a glyph), and the opaque ones are the green name
    let halo = a.rgba.chunks(4).any(|p| p[3] > 8 && p[3] < 200 && p[0] > p[1]);
    assert!(halo);
    let m = mean(&a, 0, w);
    assert!(m[1] > 200 && m[0] < 60, "{m:?}");
  }

  #[test]
  fn image_paint_stretches_the_picture_and_falls_back_to_name_color() {
    let mut t = Text::new(1.0);
    let w = width(&mut t, "mmmm");
    let p = paint(json!({"id": "I", "function": "url", "image_url": "https://cdn.7tv.app/paint/X/1x.webp"}));
    // 2x1 picture: left red, right blue
    let px = [255, 0, 0, 255, 0, 0, 255, 255];
    let a = build(&mut t, "mmmm", w, theme::WHITE, &p, Some((&px, 2, 1)));
    assert!(mean(&a, 0, w / 3)[0] > 200);
    assert!(mean(&a, 2 * w / 3, w)[2] > 200);
    let plain = build(&mut t, "mmmm", w, theme::GREEN, &p, None);
    assert!(mean(&plain, 0, w)[1] > 200);
  }

  #[test]
  fn cache_is_bounded_lru_and_rebuilds_incomplete_art() {
    let mut arts = NameArts::default();
    let key = |uid: u64| ArtKey { uid: crate::cosmetics::id::PaintId::Twitch(uid), color: 0, font_px: 15, paint: 1 };
    let art = || Art { w: 1, h: 1, base: 0.0, rgba: Box::new([0; 4]) };
    let mut built = 0;
    for uid in 0..(MAX_ARTS as u64 * 2) {
      arts.get(&key(uid), "n", true, || {
        built += 1;
        art()
      });
    }
    assert_eq!((arts.len(), built), (MAX_ARTS, MAX_ARTS * 2));
    // a hit does not rebuild
    arts.get(&key(MAX_ARTS as u64 * 2 - 1), "n", true, || panic!("rebuilt"));
    // built before the picture arrived: redone exactly once when it is ready
    arts.get(&key(9999), "n", false, art);
    arts.get(&key(9999), "n", false, || panic!("rebuilt while still waiting"));
    let mut redone = 0;
    arts.get(&key(9999), "n", true, || {
      redone += 1;
      art()
    });
    arts.get(&key(9999), "n", true, || panic!("rebuilt twice"));
    assert_eq!(redone, 1);
    // a renamed chatter gets a fresh bitmap under the same key
    let mut renamed = 0;
    arts.get(&key(9999), "other", true, || {
      renamed += 1;
      art()
    });
    assert_eq!(renamed, 1);
  }
}
