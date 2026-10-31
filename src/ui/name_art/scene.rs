// a scene (backdrop + weather) as css-style background layers, resolved once to pixel tiles for one
// name box, then moved per frame. the layer model is the ext's (scene-spec.js): an image, repeat,
// size, a resting position, a position at the end of the loop, optionally a midpoint. positions use
// css semantics: `X%` aligns the image's X% point with the box's X% point
use super::raster::{Buf, Grad, Len, Px, Svg, len, over};
use crate::cosmetics::fx::{Backdrop, Scene, Weather};

// the ext redraws scene planes this often (SCENE_STEPS_PER_SECOND)
pub const STEPS_PER_SEC: f32 = 24.0;

#[derive(Clone)]
pub enum Img {
  // a css gradient string
  G(String),
  // vector art, stretched over its tile
  S(Svg),
  // the same gradient at two alphas, mixed by the band's breathe (furnace underglow, eclipse corona)
  Breathe(String, String),
  // a flat wash that flashes (storm lightning): rgb
  Flash(u32),
}

#[derive(Clone)]
pub struct LayerDef {
  pub img: Img,
  pub repeat: (bool, bool),
  pub size: (Option<Len>, Option<Len>),
  pub from: (Len, Len),
  pub to: Option<(Len, Len)>,
  pub mid: Option<(Len, Len)>,
}

fn pair(s: &str) -> Option<(Len, Len)> {
  let s = s.trim();
  // calc(...) holds spaces: split on the space outside parentheses
  let (mut depth, mut cut) = (0, None);
  for (i, b) in s.bytes().enumerate() {
    match b {
      b'(' => depth += 1,
      b')' => depth -= 1,
      b' ' if depth == 0 && cut.is_none() => cut = Some(i),
      _ => {}
    }
  }
  let c = cut?;
  Some((len(&s[..c])?, len(&s[c + 1..])?))
}

fn dims(s: &str) -> (Option<Len>, Option<Len>) {
  let s = s.trim();
  let (mut depth, mut cut) = (0, None);
  for (i, b) in s.bytes().enumerate() {
    match b {
      b'(' => depth += 1,
      b')' => depth -= 1,
      b' ' if depth == 0 && cut.is_none() => cut = Some(i),
      _ => {}
    }
  }
  match cut {
    Some(c) => (len(&s[..c]), len(&s[c + 1..])),
    None => (len(s), None),
  }
}

// L(img, repeat, size, from) with .to() / .mid() like the ext's constructor
pub fn l(img: Img, repeat: &str, size: &str, from: &str) -> LayerDef {
  let repeat = match repeat {
    "no-repeat" => (false, false),
    "repeat-x" => (true, false),
    _ => (true, true),
  };
  LayerDef { img, repeat, size: dims(size), from: pair(from).expect("from"), to: None, mid: None }
}

impl LayerDef {
  pub fn to(mut self, s: &str) -> Self {
    self.to = Some(pair(s).expect("to"));
    self
  }

  pub fn mid(mut self, s: &str) -> Self {
    self.mid = Some(pair(s).expect("mid"));
    self
  }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dynamic {
  None,
  // 0..1 along the breathe
  Breathe,
  Flash,
}

struct Layer {
  tile: Buf,
  // second tile of a breathing pair
  hi: Option<Buf>,
  flash: Option<u32>,
  repeat: (bool, bool),
  from: (f32, f32),
  to: (f32, f32),
  mid: Option<(f32, f32)>,
  // the breathe value shown while nothing animates
  rest_k: f32,
  dynamic: Dynamic,
}

impl Layer {
  fn moves(&self) -> bool {
    self.from != self.to || self.mid.is_some()
  }
}

pub struct Band {
  // the still layers at the bottom of the stack, flattened once (the sky): copied, not re-blended
  base: Vec<Px>,
  // bottom first, above the flattened base
  layers: Vec<Layer>,
  period: f32,
  alternate: bool,
  luminance: bool,
  // the flash's own loop (storm lightning)
  flash_period: f32,
  w: usize,
  h: usize,
}

fn axis(l: Option<Len>, plate: f32, s: f32, auto: f32) -> f32 {
  l.map_or(auto, |l| l.pct / 100.0 * plate + l.px * s).max(1.0).round()
}

fn at(p: (Len, Len), tw: f32, th: f32, w: f32, h: f32, s: f32) -> (f32, f32) {
  ((w - tw) * p.0.pct / 100.0 + p.0.px * s, (h - th) * p.1.pct / 100.0 + p.1.px * s)
}

impl Band {
  // `defs` are in css order (first paints on top); the plate is w×h with css px scale s
  pub fn new(defs: &[LayerDef], w: usize, h: usize, s: f32, period: f32, alternate: bool, luminance: bool) -> Band {
    let (fw, fh) = (w as f32, h as f32);
    let mut layers: Vec<Layer> = defs
      .iter()
      .rev()
      .map(|d| {
        let (tw, th);
        let (mut tile, mut hi, mut flash) = (None, None, None);
        let mut dynamic = Dynamic::None;
        let mut rest_k = 0.0;
        let (sw, sh) = d.size;
        match &d.img {
          Img::G(g) | Img::Breathe(g, _) => {
            (tw, th) = (axis(sw, fw, s, fw), axis(sh, fh, s, fh));
            let r =
              |g: &str| Grad::parse(g).unwrap_or_else(|| panic!("gradient {g}")).render(tw as usize, th as usize, s);
            tile = Some(r(g));
            if let Img::Breathe(_, b) = &d.img {
              hi = Some(r(b));
              dynamic = Dynamic::Breathe;
            }
          }
          Img::S(svg) => {
            let ratio = svg.vw / svg.vh;
            (tw, th) = match (sw, sh) {
              (None, None) => (fw, fh),
              (None, Some(_)) => {
                let th = axis(sh, fh, s, fh);
                ((th * ratio).round().max(1.0), th)
              }
              (Some(_), None) => {
                let tw = axis(sw, fw, s, fw);
                (tw, (tw / ratio).round().max(1.0))
              }
              (Some(_), Some(_)) => (axis(sw, fw, s, fw), axis(sh, fh, s, fh)),
            };
            tile = Some(svg.render(tw as usize, th as usize));
          }
          Img::Flash(c) => {
            flash = Some(*c);
            dynamic = Dynamic::Flash;
            (tw, th) = (fw, fh);
          }
        }
        let from = at(d.from, tw, th, fw, fh, s);
        let to = d.to.map_or(from, |p| at(p, tw, th, fw, fh, s));
        let mid = d.mid.map(|p| at(p, tw, th, fw, fh, s));
        if dynamic == Dynamic::Breathe {
          rest_k = 0.0;
        }
        Layer {
          tile: tile.unwrap_or_else(|| Buf::new(1, 1)),
          hi,
          flash,
          repeat: d.repeat,
          from,
          to,
          mid,
          rest_k,
          dynamic,
        }
      })
      .collect();
    let still = layers.iter().take_while(|l| !l.moves() && l.dynamic == Dynamic::None && l.flash.is_none()).count();
    let mut base = Vec::new();
    if still > 0 {
      base = vec![[0; 4]; w * h];
      for l in layers.drain(..still) {
        l.paint(l.rest_px(), 0.0, w, h, &mut base);
      }
    }
    Band { base, layers, period, alternate, luminance, flash_period: period, w, h }
  }

  pub fn with_flash_period(mut self, p: f32) -> Self {
    self.flash_period = p;
    self
  }

  pub fn set_rest(&mut self, k: f32) {
    for l in &mut self.layers {
      l.rest_k = k;
    }
  }

  pub fn animated(&self) -> bool {
    self.layers.iter().any(|l| l.moves() || l.dynamic != Dynamic::None)
  }

  // progress through the loop at wall time t (the same for every copy of a name)
  fn phase(&self, t: f32) -> f32 {
    let span = if self.alternate { self.period * 2.0 } else { self.period };
    let u = (t / span).rem_euclid(1.0);
    let mut p = if self.alternate { 1.0 - (2.0 * u - 1.0).abs() } else { u };
    if self.luminance {
      if self.alternate {
        // ease-in-out, the ext's fallback for a smooth alternate
        p = p * p * (3.0 - 2.0 * p);
      }
    } else {
      let n = (self.period * STEPS_PER_SEC).round().max(1.0);
      p = (p * n).floor() / n;
    }
    p
  }

  fn flash(&self, t: f32) -> f32 {
    const KF: [(f32, f32); 6] =
      [(0.0, 0.0), (0.82, 0.0), (0.84, 77.0 / 255.0), (0.86, 16.0 / 255.0), (0.885, 56.0 / 255.0), (0.91, 0.0)];
    let q = (t / self.flash_period).rem_euclid(1.0);
    for w in KF.windows(2) {
      if q < w[1].0 {
        return w[0].1 + (w[1].1 - w[0].1) * (q - w[0].0) / (w[1].0 - w[0].0);
      }
    }
    0.0
  }

  // paints the band over `out` (w×h premultiplied). t = None: the designed resting frame.
  // `empty`: out is still clear, so the flattened base can be copied in
  pub fn paint(&self, t: Option<f32>, out: &mut [Px], empty: bool) {
    debug_assert_eq!(out.len(), self.w * self.h);
    match (empty, self.base.is_empty()) {
      (true, true) => out.iter_mut().for_each(|p| *p = [0; 4]),
      (true, false) => out.copy_from_slice(&self.base),
      (false, true) => {}
      (false, false) => out.iter_mut().zip(&self.base).for_each(|(d, s)| over(d, *s)),
    }
    let p = t.map_or(0.0, |t| self.phase(t));
    for l in &self.layers {
      let (x, y) = match t {
        Some(_) if l.moves() => {
          let (a, b, f) = match l.mid {
            Some(m) if p < 0.5 => (l.from, m, p * 2.0),
            Some(m) => (m, l.to, p * 2.0 - 1.0),
            None => (l.from, l.to, p),
          };
          (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
        }
        _ => l.from,
      };
      let k = match (l.dynamic, t) {
        (Dynamic::Breathe, Some(_)) => p,
        (Dynamic::Breathe, None) => l.rest_k,
        (Dynamic::Flash, Some(t)) => self.flash(t),
        _ => 0.0,
      };
      l.paint((x.round() as i32, y.round() as i32), k, self.w, self.h, out);
    }
  }
}

impl Layer {
  fn rest_px(&self) -> (i32, i32) {
    (self.from.0.round() as i32, self.from.1.round() as i32)
  }

  fn paint(&self, (ox, oy): (i32, i32), k: f32, w: usize, h: usize, out: &mut [Px]) {
    if let Some(c) = self.flash {
      let a = (k * 255.0).round() as u32;
      if a == 0 {
        return;
      }
      let m = |s: u32| super::raster::div255(((c >> s) & 255) * a) as u8;
      let px = [m(16), m(8), m(0), a as u8];
      out.iter_mut().for_each(|d| over(d, px));
      return;
    }
    let (tw, th) = (self.tile.w as i32, self.tile.h as i32);
    let kk = (k.clamp(0.0, 1.0) * 256.0) as u32;
    for y in 0..h as i32 {
      let ty = y - oy;
      let ty = match self.repeat.1 {
        true => ty.rem_euclid(th),
        false if (0..th).contains(&ty) => ty,
        false => continue,
      };
      let row = &mut out[y as usize * w..(y as usize + 1) * w];
      let off = ty as usize * tw as usize;
      let src = &self.tile.px[off..off + tw as usize];
      let hi = self.hi.as_ref().map(|b| &b.px[off..off + tw as usize]);
      // the columns this layer covers, and where in the tile the first one lands
      let (x0, x1, mut tx) = if self.repeat.0 {
        (0, w, (-ox).rem_euclid(tw) as usize)
      } else {
        ((ox.max(0)) as usize, ((ox + tw).clamp(0, w as i32)) as usize, (-ox).max(0) as usize)
      };
      for d in row.iter_mut().take(x1).skip(x0) {
        let i = tx;
        tx = if tx + 1 == tw as usize { 0 } else { tx + 1 };
        let mut s = src[i];
        if s[3] == 0 && hi.is_none() {
          continue;
        }
        if let Some(hi) = hi {
          let b = hi[i];
          for c in 0..4 {
            s[c] = ((s[c] as u32 * (256 - kk) + b[c] as u32 * kk) >> 8) as u8;
          }
        }
        over(d, s);
      }
    }
  }
}

// the bands one scene draws: under the name (plate + far weather), behind it (fog), over it (near)
pub struct SceneArt {
  pub back: Option<Band>,
  pub behind: Option<Band>,
  pub front: Option<Band>,
}

impl SceneArt {
  pub fn new(spec: &Scene, w: usize, h: usize) -> SceneArt {
    super::scene_cat::build(spec, w, h)
  }

  pub fn animated(&self) -> bool {
    [&self.back, &self.behind, &self.front].into_iter().flatten().any(Band::animated)
  }

  // plate + fog: what sits under the name's glyphs
  pub fn paint_under(&self, t: Option<f32>, out: &mut [Px]) {
    let mut empty = true;
    if self.back.is_none() && self.behind.is_none() {
      out.iter_mut().for_each(|p| *p = [0; 4]);
    }
    for b in [&self.back, &self.behind].into_iter().flatten() {
      b.paint(t, out, empty);
      empty = false;
    }
  }

  // the near weather + foreground silhouette, over the glyphs
  pub fn paint_over(&self, t: Option<f32>, out: &mut [Px]) {
    if let Some(b) = &self.front {
      b.paint(t, out, false);
    }
  }
}

pub fn backdrop_period(b: Backdrop) -> (f32, bool) {
  use Backdrop::*;
  match b {
    Dawn => (16.0, false),
    Graveyard => (22.0, false),
    Abyss => (18.0, false),
    Nightfall => (20.0, false),
    Terminal => (9.0, false),
    Furnace => (5.0, true),
    Ocean => (14.0, false),
    Skyline => (26.0, false),
    Orbit => (30.0, false),
    Synth => (1.4, false),
    Glacier => (28.0, false),
    Sakura => (20.0, false),
    Volcano => (12.0, false),
    Alpine => (24.0, false),
    Swamp => (18.0, false),
    Castle => (26.0, false),
    Moon => (30.0, false),
    Circuit => (4.0, false),
    Eclipse => (6.0, true),
    Arcade => (2.0, false),
    Bamboo => (20.0, false),
    Giza => (22.0, false),
  }
}

pub fn weather_period(w: Weather) -> (f32, bool) {
  use Weather::*;
  match w {
    Rain => (0.9, false),
    Snow => (4.5, false),
    Fog => (16.0, false),
    Embers => (3.2, false),
    Glyphs => (2.6, false),
    Storm => (7.0, true),
    Petals => (3.8, false),
    Bubbles => (4.0, false),
    Fireflies => (6.0, false),
    Leaves => (4.2, false),
    Sparks => (1.2, false),
    Dust => (5.0, false),
    Confetti => (4.0, false),
    Meteors => (1.6, false),
    Hearts => (4.4, false),
  }
}
