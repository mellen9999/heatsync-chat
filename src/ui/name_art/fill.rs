// the `fill` block drawn: up to four gradient layers (linear / radial / conic, tiled, flowing or
// spinning) over the glyphs, then the hue / breathe modulators, over the chatter's own color when
// there is an underlay. mirrors the ext's fillLayerCss / fillBoxCss (paint-spec.js): the same tile
// lengths, travel and periods, one gradient across the whole name when it moves
use super::raster::{Px, over};
use crate::cosmetics::fill::{Fill, Layer, Motion, Rgba, Shape, Stop, Tile};
use crate::cosmetics::fx::{MAX_SPEED, MIN_SPEED};

const LUT: usize = 512;
const TAU: f32 = std::f32::consts::TAU;
// paint-spec.js FILL_FLOW_NAME_PERIOD / FILL_FLOW_PX_PERIOD / FILL_SPIN_PERIOD / FILL_HUE_PERIOD / FILL_BREATHE_PERIOD
pub const FLOW_NAME_S: f32 = 2.5;
pub const FLOW_PX_S: f32 = 2.4;
pub const SPIN_S: f32 = 6.0;
pub const HUE_S: f32 = 8.0;
pub const BREATHE_S: f32 = 2.4;
// paint-core.js MIN_LUMINANCE_PERIOD_S: hue and breathe change brightness
const MIN_LUMINANCE_S: f32 = 1.0;

// premultiplied float rgba
type Pm = [f32; 4];

fn premul(c: Rgba) -> Pm {
  let a = c[3] as f32 / 255.0;
  [c[0] as f32 * a, c[1] as f32 * a, c[2] as f32 * a, c[3] as f32]
}

fn mix(a: Pm, b: Pm, f: f32) -> Pm {
  std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f)
}

// css gradient color at t: hold the ends, blend premultiplied, bend by the color hint
fn ramp(stops: &[Stop], t: f32) -> Pm {
  let first = &stops[0];
  if t <= first.pos {
    return premul(first.color);
  }
  for w in stops.windows(2) {
    if t < w[1].pos {
      let span = w[1].pos - w[0].pos;
      if span <= 0.0 {
        return premul(w[1].color);
      }
      let mut f = (t - w[0].pos) / span;
      if let Some(h) = w[0].hint.filter(|h| *h > 0.0 && *h < 1.0) {
        f = f.powf(0.5f32.ln() / h.ln());
      }
      return mix(premul(w[0].color), premul(w[1].color), f);
    }
  }
  premul(stops[stops.len() - 1].color)
}

fn lut_of(stops: &[Stop]) -> Box<[Px]> {
  (0..LUT)
    .map(|i| {
      let c = ramp(stops, i as f32 / (LUT - 1) as f32);
      [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, c[3].round() as u8]
    })
    .collect()
}

struct Mv {
  m: Motion,
  // seconds per loop at speed 1, over the speed
  period: f32,
}

struct LayerArt {
  shape: Shape,
  // css degrees: the fill's angle plus the layer's tilt
  deg: f32,
  sin: f32,
  cos: f32,
  lut: Box<[Px]>,
  tile: Option<Tile>,
  cx: f32,
  cy: f32,
  mv: Option<Mv>,
  // a radial dot tile in px
  dot: bool,
}

impl LayerArt {
  fn new(l: &Layer, angle: f32, motion: bool) -> LayerArt {
    let deg = (angle + l.tilt).rem_euclid(360.0);
    let (sin, cos) = deg.to_radians().sin_cos();
    let dot = matches!((l.shape, l.tile), (Shape::Radial { .. }, Some(Tile::Px(_))));
    // seconds per loop, the ext's fillBoxCss: a spin turns in 6s; a flow takes 2.4s per px tile or 2.5s per name
    // (times the tile's length in names, for a linear layer)
    let mv = l.motion.filter(|_| motion).map(|m| {
      let name_n = if let (Shape::Linear, Some(Tile::Name(n))) = (l.shape, l.tile) { n } else { 1.0 };
      let base = if m.spin {
        SPIN_S
      } else if matches!(l.tile, Some(Tile::Px(_))) && (matches!(l.shape, Shape::Linear) || dot) {
        FLOW_PX_S
      } else if matches!(l.shape, Shape::Linear) {
        FLOW_NAME_S * if matches!(l.tile, Some(Tile::Name(_))) { name_n } else { 1.0 }
      } else {
        FLOW_NAME_S
      };
      Mv { m, period: base / m.speed.clamp(MIN_SPEED, MAX_SPEED) }
    });
    LayerArt { shape: l.shape, deg, sin, cos, lut: lut_of(&l.stops), tile: l.tile, cx: l.cx, cy: l.cy, mv, dot }
  }

  fn at(&self, u: f32) -> Px {
    self.lut[(u.clamp(0.0, 1.0) * (LUT - 1) as f32).round() as usize]
  }

  // the layer at (x, y) of a bw×bh name box; `s` css px -> device px; `mv` 0..1 of the layer's travel
  fn eval(&self, x: f32, y: f32, bw: f32, bh: f32, s: f32, mv: f32) -> Px {
    let (mut x, mut y) = (x, y);
    let mut from = self.deg;
    let (cx, cy) = (self.cx * bw, self.cy * bh);
    let flowing_linear = self.mv.as_ref().is_some_and(|m| !m.m.spin) && matches!(self.shape, Shape::Linear);
    let mut travel = 0.0;
    if let Some(m) = &self.mv {
      if m.m.spin {
        if self.shape == Shape::Conic {
          from += 360.0 * mv;
        } else {
          // the layer turns about its center: sample where this pixel came from
          let (sn, cs) = (-TAU * mv).sin_cos();
          let (c0, c1) = if self.shape == Shape::Linear { (bw / 2.0, bh / 2.0) } else { (cx, cy) };
          let (dx, dy) = (x - c0, y - c1);
          (x, y) = (c0 + dx * cs - dy * sn, c1 + dx * sn + dy * cs);
          (x, y) = self.wrap(x, y, bw, bh, s);
        }
      } else if !flowing_linear {
        // radial / conic flow along the box axis the angle mostly runs, by one tile of their own pattern
        let axis_x = self.sin.abs() >= self.cos.abs();
        let dir = if axis_x { -self.sin.signum() } else { self.cos.signum() };
        let dir = if dir == 0.0 { -1.0 } else { dir };
        let t = if self.dot {
          self.dot_px(s)
        } else if axis_x {
          bw
        } else {
          bh
        };
        if axis_x {
          x -= dir * t * mv;
        } else {
          y -= dir * t * mv;
        }
        (x, y) = self.wrap(x, y, bw, bh, s);
      } else {
        travel = mv;
      }
    }
    match self.shape {
      Shape::Linear => {
        let (sn, cs) = if from == self.deg { (self.sin, self.cos) } else { from.to_radians().sin_cos() };
        let g = (bw * sn).abs() + (bh * cs).abs();
        if g <= 0.0 {
          return self.at(0.5);
        }
        let line = (x - bw / 2.0) * sn - (y - bh / 2.0) * cs + g / 2.0;
        match (self.tile, flowing_linear) {
          (Some(Tile::Px(p)), _) => {
            let len = (p * s).max(1e-3);
            let l = line + if flowing_linear { len * travel } else { 0.0 };
            self.at((l / len).rem_euclid(1.0))
          }
          (Some(Tile::Name(n)), _) => {
            let len = (g * n).max(1e-3);
            let l = line + if flowing_linear { len * travel } else { 0.0 };
            self.at((l / len).rem_euclid(1.0))
          }
          // a moving layer that does not repeat tiles by one gradient line
          (None, true) => self.at(((line + g * travel) / g).rem_euclid(1.0)),
          (None, false) => self.at(line / g),
        }
      }
      Shape::Radial { circle } => {
        if self.dot {
          // a small dot tile repeated across the box, anchored at the center's share of the free space
          let p = self.dot_px(s);
          let (ox, oy) = ((bw - p) * self.cx, (bh - p) * self.cy);
          let (u, v) = ((x - ox).rem_euclid(p) - p / 2.0, (y - oy).rem_euclid(p) - p / 2.0);
          return self.at((u * u + v * v).sqrt() / (p * std::f32::consts::FRAC_1_SQRT_2));
        }
        let (fx, fy) = (cx.max(bw - cx), cy.max(bh - cy));
        let (dx, dy) = (x - cx, y - cy);
        let t = if circle {
          let r = fx.hypot(fy);
          if r > 0.0 { dx.hypot(dy) / r } else { 0.0 }
        } else {
          let (rx, ry) = (fx * std::f32::consts::SQRT_2, fy * std::f32::consts::SQRT_2);
          if rx > 0.0 && ry > 0.0 { (dx / rx).hypot(dy / ry) } else { 0.0 }
        };
        match self.tile {
          Some(Tile::Name(n)) => self.at((t / n.max(1e-3)).rem_euclid(1.0)),
          _ => self.at(t),
        }
      }
      Shape::Conic => {
        let a = ((x - cx).atan2(-(y - cy)).to_degrees() - from).rem_euclid(360.0) / 360.0;
        self.at(a)
      }
    }
  }

  fn dot_px(&self, s: f32) -> f32 {
    match self.tile {
      Some(Tile::Px(p)) => (p * s).max(1.0),
      _ => 1.0,
    }
  }

  // a moved layer's image repeats by the name box (a dot lattice by its own period)
  fn wrap(&self, x: f32, y: f32, bw: f32, bh: f32, _s: f32) -> (f32, f32) {
    if self.dot { (x, y) } else { (x.rem_euclid(bw.max(1.0)), y.rem_euclid(bh.max(1.0))) }
  }
}

pub struct FillArt {
  layers: Vec<LayerArt>,
  // layers below every mover are flattened once
  first_moving: usize,
  hue: Option<f32>,
  breathe: Option<(f32, f32)>,
  underlay: Option<[u8; 3]>,
  s: f32,
}

// what one frame needs of the modulators
pub struct Frame {
  mv: [f32; 4],
  hue: Option<[f32; 9]>,
  fade: Option<f32>,
}

impl FillArt {
  // `motion` off draws the resting frame; `ink` is the chatter's color
  pub fn new(f: &Fill, s: f32, motion: bool, ink: u32) -> FillArt {
    let layers: Vec<LayerArt> = f.layers.iter().map(|l| LayerArt::new(l, f.angle, motion)).collect();
    let first_moving = layers.iter().position(|l| l.mv.is_some()).unwrap_or(layers.len());
    FillArt {
      layers,
      first_moving,
      hue: f.hue.filter(|_| motion),
      breathe: f.breathe.filter(|_| motion),
      underlay: f.underlay.then_some([(ink >> 16) as u8, (ink >> 8) as u8, ink as u8]),
      s,
    }
  }

  // anything here changes between frames: the name is then one gradient, not one per glyph
  pub fn moves(&self) -> bool {
    self.first_moving < self.layers.len() || self.hue.is_some() || self.breathe.is_some()
  }

  pub fn rest_frame() -> Frame {
    Frame { mv: [0.0; 4], hue: None, fade: None }
  }

  // layer travel and modulators at wall time t
  pub fn frame(&self, t: f32) -> Frame {
    let mut mv = [0.0; 4];
    for (k, l) in self.layers.iter().enumerate() {
      if let Some(m) = &l.mv {
        let p = super::motion::phase(t, (m.period * 1000.0).round() / 1000.0);
        let e = if m.m.bounce { 0.5 - 0.5 * (p * TAU).cos() } else { p };
        mv[k] = if m.m.reverse { 1.0 - e } else { e };
      }
    }
    // hue and breathe change luminance: the ext floors their period at 1s
    let per = |base: f32, sp: f32| ((base / sp).max(MIN_LUMINANCE_S) * 1000.0).round() / 1000.0;
    let hue = self.hue.map(|sp| super::motion::hue_matrix(360.0 * super::motion::phase(t, per(HUE_S, sp))));
    let fade = self.breathe.map(|(sp, depth)| {
      let p = super::motion::phase(t, per(BREATHE_S, sp));
      1.0 - depth * (0.5 - 0.5 * (p * TAU).cos())
    });
    Frame { mv, hue, fade }
  }

  // the layers below every mover, premultiplied, for the pixel at (x, y) of its bw×bh box
  pub fn base_px(&self, x: f32, y: f32, bw: f32, bh: f32) -> Px {
    let mut p = [0u8; 4];
    for l in &self.layers[..self.first_moving] {
      over(&mut p, l.eval(x, y, bw, bh, self.s, 0.0));
    }
    p
  }

  // the finished fill color at a pixel: `base` is base_px of it
  pub fn px(&self, base: Px, x: f32, y: f32, bw: f32, bh: f32, f: &Frame) -> Px {
    let mut p = base;
    for k in self.first_moving..self.layers.len() {
      over(&mut p, self.layers[k].eval(x, y, bw, bh, self.s, f.mv[k]));
    }
    if let Some(m) = &f.hue {
      let (r, g, b, a) = (p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32);
      p[0] = (m[0] * r + m[1] * g + m[2] * b).clamp(0.0, a) as u8;
      p[1] = (m[3] * r + m[4] * g + m[5] * b).clamp(0.0, a) as u8;
      p[2] = (m[6] * r + m[7] * g + m[8] * b).clamp(0.0, a) as u8;
    }
    if let Some(k) = f.fade.filter(|k| *k < 0.999) {
      for c in &mut p {
        *c = (*c as f32 * k) as u8;
      }
    }
    if let Some(u) = self.underlay {
      let mut under = [u[0], u[1], u[2], 255];
      over(&mut under, p);
      p = under;
    }
    p
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::cosmetics::fill::Fill;
  use serde_json::json;

  fn art(v: serde_json::Value, motion: bool) -> FillArt {
    FillArt::new(&Fill::parse(&v, false), 1.0, motion, 0xffffff)
  }

  fn at(a: &FillArt, x: f32, y: f32, f: &Frame) -> Px {
    a.px(a.base_px(x, y, 100.0, 20.0), x, y, 100.0, 20.0, f)
  }

  fn stops2() -> serde_json::Value {
    json!([{"color": "#ff0000", "pos": 0}, {"color": "#0000ff", "pos": 100}])
  }

  #[test]
  fn linear_ramps_across_the_name_and_holds_ends() {
    let a = art(json!({"angle": 90, "layers": [{"kind": "linear", "stops": stops2()}]}), true);
    let f = FillArt::rest_frame();
    let (l, m, r) = (at(&a, 0.5, 10.0, &f), at(&a, 50.0, 10.0, &f), at(&a, 99.5, 10.0, &f));
    assert!(l[0] > 240 && l[2] < 10 && r[2] > 240 && r[0] < 10, "{l:?} {r:?}");
    assert!((m[0] as i32 - 128).abs() < 6 && (m[2] as i32 - 128).abs() < 6, "{m:?}");
  }

  #[test]
  fn translucent_stops_blend_premultiplied() {
    let a = art(
      json!({"layers": [{"stops": [{"color": "#ff000000", "pos": 0}, {"color": "#ff0000ff", "pos": 100}]}]}),
      false,
    );
    let m = at(&a, 50.0, 10.0, &FillArt::rest_frame());
    // halfway between clear and red is half-covered red, not dark red
    assert!((m[3] as i32 - 128).abs() < 8 && (m[0] as i32 - m[3] as i32).abs() < 3, "{m:?}");
  }

  #[test]
  fn hint_bends_the_midpoint() {
    let plain = art(json!({"angle": 90, "layers": [{"stops": stops2()}]}), false);
    let hinted = art(
      json!({"angle": 90, "layers": [{"stops": [{"color": "#ff0000", "pos": 0, "hint": 20}, {"color": "#0000ff", "pos": 100}]}]}),
      false,
    );
    let f = FillArt::rest_frame();
    assert!(at(&hinted, 20.0, 10.0, &f)[2] > at(&plain, 20.0, 10.0, &f)[2] + 40);
    let mid = at(&hinted, 20.0, 10.0, &f);
    assert!((mid[0] as i32 - mid[2] as i32).abs() < 12, "the hint is where the colors meet {mid:?}");
  }

  #[test]
  fn flow_moves_one_tile_per_loop_and_returns() {
    let v = json!({"angle": 90, "layers": [{"stops": stops2(), "repeat": true, "tile": {"unit": "px", "size": 20},
      "motion": {"type": "flow", "speed": 1}}]});
    let a = art(v.clone(), true);
    assert!(a.moves());
    let sample = |t: f32| at(&a, 7.0, 10.0, &a.frame(t));
    assert_ne!(sample(0.0), sample(0.6));
    // 2.4s per px tile at speed 1: a whole loop lands on itself
    assert_eq!(sample(0.3), sample(0.3 + FLOW_PX_S));
    // motion off draws the resting frame and never moves
    let still = art(v, false);
    assert!(!still.moves());
  }

  #[test]
  fn spin_turns_conic_and_reverse_runs_backwards() {
    let mk = |rev: bool| {
      art(json!({"layers": [{"kind": "conic", "stops": stops2(), "motion": {"type": "spin", "reverse": rev}}]}), true)
    };
    let (fw, bw) = (mk(false), mk(true));
    let p = |a: &FillArt, t: f32| at(a, 80.0, 10.0, &a.frame(t));
    assert_eq!(p(&fw, 1.0), p(&fw, 1.0 + SPIN_S));
    assert_eq!(p(&fw, 1.5), p(&bw, SPIN_S - 1.5), "reverse is the mirrored clock");
  }

  #[test]
  fn modulators_floor_at_one_second_and_fade() {
    let a = art(
      json!({"layers": [{"stops": [{"color": "#ff0000", "pos": 0}]}], "breathe": {"speed": 3, "depth": 0.5}}),
      true,
    );
    let lo = at(&a, 10.0, 10.0, &a.frame(0.0));
    let hi = at(&a, 10.0, 10.0, &a.frame(0.4));
    assert_eq!(lo[3], 255, "breathe starts opaque");
    assert!(hi[3] < 200, "{hi:?}");
    let h = art(json!({"layers": [{"stops": [{"color": "#ff0000", "pos": 0}]}], "hue": {"speed": 3}}), true);
    assert_ne!(at(&h, 5.0, 5.0, &h.frame(0.0)), at(&h, 5.0, 5.0, &h.frame(0.5)));
  }

  #[test]
  fn underlay_shows_through_clear_stops() {
    let f = Fill::parse(&json!({"layers": [{"stops": [{"color": "#ff000000", "pos": 0}]}]}), true);
    let a = FillArt::new(&f, 1.0, false, 0x00ff00);
    assert_eq!(a.px(a.base_px(3.0, 3.0, 50.0, 20.0), 3.0, 3.0, 50.0, 20.0, &FillArt::rest_frame()), [0, 255, 0, 255]);
  }

  #[test]
  fn radial_and_tiles_stay_inside_the_ramp() {
    for v in [
      json!({"kind": "radial", "shape": "circle", "stops": stops2()}),
      json!({"kind": "radial", "stops": stops2(), "repeat": true, "tile": {"unit": "px", "size": 6}}),
      json!({"kind": "radial", "stops": stops2(), "repeat": true, "tile": {"unit": "name", "size": 0.3}}),
      json!({"kind": "conic", "center": {"x": 10, "y": 90}, "stops": stops2()}),
    ] {
      let a = art(json!({"layers": [v]}), true);
      for (x, y) in [(0.0, 0.0), (50.0, 10.0), (99.9, 19.9), (13.3, 4.4)] {
        let p = at(&a, x, y, &FillArt::rest_frame());
        assert_eq!(p[3], 255);
      }
    }
  }
}
