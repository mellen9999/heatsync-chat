// the motion half of a heatsync paint: each effect's keyframes from the ext's paint-spec.js, as
// pure functions of the cycle phase. a transform is 2x3 affine; css px are scaled by `s`
use crate::cosmetics::fx::{Effect, Kind};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Aff(pub [f32; 6]);

impl Aff {
  pub const ID: Aff = Aff([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

  // self ∘ other (other applies first)
  pub fn then(self, o: Aff) -> Aff {
    let (a, b) = (self.0, o.0);
    Aff([
      a[0] * b[0] + a[2] * b[1],
      a[1] * b[0] + a[3] * b[1],
      a[0] * b[2] + a[2] * b[3],
      a[1] * b[2] + a[3] * b[3],
      a[0] * b[4] + a[2] * b[5] + a[4],
      a[1] * b[4] + a[3] * b[5] + a[5],
    ])
  }

  pub fn translate(x: f32, y: f32) -> Aff {
    Aff([1.0, 0.0, 0.0, 1.0, x, y])
  }

  pub fn scale(x: f32, y: f32) -> Aff {
    Aff([x, 0.0, 0.0, y, 0.0, 0.0])
  }

  pub fn rotate(deg: f32) -> Aff {
    let (s, c) = deg.to_radians().sin_cos();
    Aff([c, s, -s, c, 0.0, 0.0])
  }

  pub fn skew_x(deg: f32) -> Aff {
    Aff([1.0, 0.0, deg.to_radians().tan(), 1.0, 0.0, 0.0])
  }

  // the same transform about a point of the box
  pub fn about(self, ox: f32, oy: f32) -> Aff {
    Aff::translate(ox, oy).then(self).then(Aff::translate(-ox, -oy))
  }

  pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
    let m = self.0;
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
  }

  pub fn inverse(&self) -> Option<Aff> {
    let m = self.0;
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-5 {
      return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some(Aff([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])]))
  }

  // a pure shift by whole pixels
  pub fn int_shift(&self) -> Option<(i32, i32)> {
    let m = self.0;
    let near = |v: f32| (v - v.round()).abs() < 0.01;
    ((m[0] - 1.0).abs() < 1e-4
      && m[1].abs() < 1e-4
      && m[2].abs() < 1e-4
      && (m[3] - 1.0).abs() < 1e-4
      && near(m[4])
      && near(m[5]))
    .then(|| (m[4].round() as i32, m[5].round() as i32))
  }
}

#[derive(Clone, Copy)]
enum Ease {
  Bezier(f32, f32, f32, f32),
  // steps(1, end): hold the interval's start value
  Hold,
}

const IN_OUT: Ease = Ease::Bezier(0.42, 0.0, 0.58, 1.0);
const OUT: Ease = Ease::Bezier(0.0, 0.0, 0.58, 1.0);

fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
  let at = |a: f32, b: f32, t: f32| 3.0 * a * (1.0 - t).powi(2) * t + 3.0 * b * (1.0 - t) * t * t + t.powi(3);
  let (mut lo, mut hi) = (0.0f32, 1.0f32);
  for _ in 0..24 {
    let mid = (lo + hi) / 2.0;
    if at(x1, x2, mid) < x {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  at(y1, y2, (lo + hi) / 2.0)
}

fn ease(e: Ease, u: f32) -> f32 {
  match e {
    Ease::Bezier(a, b, c, d) => bezier(a, b, c, d, u),
    Ease::Hold => 0.0,
  }
}

// css keyframes: the timing function applies inside each interval
fn kf<const N: usize>(stops: &[(f32, [f32; N])], e: Ease, p: f32) -> [f32; N] {
  let p = p * 100.0;
  for w in stops.windows(2) {
    if p < w[1].0 {
      let span = w[1].0 - w[0].0;
      let u = ease(e, if span > 0.0 { (p - w[0].0) / span } else { 1.0 });
      return std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * u);
    }
  }
  stops[stops.len() - 1].1
}

// seconds per cycle: the ext floors a brightness change at 1s (flashing guard)
pub fn period(k: Kind, speed: f32) -> f32 {
  let s = k.period() / speed;
  ((if k.luminance() { s.max(1.0) } else { s }) * 1000.0).round() / 1000.0
}

pub fn phase(t: f32, period: f32) -> f32 {
  (t / period).rem_euclid(1.0)
}

// what a whole-name motion does at phase p: the transform (about its origin, in the name box w×h),
// and for the shadow effects the state to show
pub fn self_xf(e: &Effect, p: f32, s: f32, w: f32, h: f32) -> Aff {
  let (cx, cy) = (w / 2.0, h / 2.0);
  match e.kind {
    Kind::Coin => {
      let [deg] = kf(
        &[(0.0, [0.0]), (55.0, [0.0]), (75.0, [180.0]), (95.0, [360.0]), (100.0, [360.0])],
        Ease::Bezier(0.6, 0.0, 0.4, 1.0),
        p,
      );
      Aff::scale(deg.to_radians().cos(), 1.0).about(cx, cy)
    }
    Kind::Heli => {
      let [deg] = kf(&[(0.0, [0.0]), (72.0, [0.0]), (100.0, [360.0])], Ease::Bezier(0.5, 0.0, 0.5, 1.0), p);
      Aff::rotate(deg).about(cx, cy)
    }
    Kind::Float => {
      let [ty, rot] = kf(&[(0.0, [1.5, -1.6]), (50.0, [-2.5, 1.6]), (100.0, [1.5, -1.6])], IN_OUT, p);
      Aff::translate(0.0, ty * s).then(Aff::rotate(rot).about(cx, cy))
    }
    Kind::Heart => {
      let [k] = kf(&[(0.0, [1.0]), (10.0, [1.11]), (20.0, [1.04]), (28.0, [1.0]), (100.0, [1.0])], OUT, p);
      Aff::scale(k, k).about(cx, cy)
    }
    Kind::Wobble => {
      let [k] = kf(&[(0.0, [1.0]), (50.0, [1.09]), (100.0, [1.0])], IN_OUT, p);
      Aff::scale(k, 1.0).about(cx, cy)
    }
    Kind::Swing => {
      let [deg] = kf(&[(0.0, [4.5]), (50.0, [-4.5]), (100.0, [4.5])], IN_OUT, p);
      Aff::rotate(deg).about(cx, -0.6 * h)
    }
    Kind::Jitter => {
      let [x, y] = kf(
        &[
          (0.0, [0.0, 0.0]),
          (80.0, [0.0, 0.0]),
          (82.0, [-1.0, 1.0]),
          (84.0, [1.0, -1.0]),
          (86.0, [-1.0, -1.0]),
          (88.0, [1.0, 1.0]),
          (90.0, [-1.0, 0.0]),
          (92.0, [1.0, 0.0]),
          (100.0, [0.0, 0.0]),
        ],
        Ease::Hold,
        p,
      );
      Aff::translate(x * s, y * s)
    }
    // pan's wobble rides its own sweep
    Kind::Pan if e.skew => {
      let a = if e.bounce { -0.75 * (1.0 - (p * std::f32::consts::TAU).cos()) } else { -1.5 * p };
      Aff::skew_x(a).about(cx, cy)
    }
    _ => Aff::ID,
  }
}

// whole-name opacity: pulse breathes, flicker dips
pub fn opacity(e: &Effect, p: f32) -> f32 {
  match e.kind {
    Kind::Pulse => 0.56 + 0.44 * (p * std::f32::consts::TAU).cos(),
    Kind::Flicker => kf(
      &[(0.0, [1.0]), (70.0, [1.0]), (72.0, [0.4]), (74.0, [1.0]), (84.0, [0.55]), (86.0, [1.0]), (100.0, [1.0])],
      Ease::Hold,
      p,
    )[0],
    _ => 1.0,
  }
}

// neon's breathe, 0 (rest glow) .. 1 (full glow)
pub fn neon(p: f32) -> f32 {
  kf(&[(0.0, [0.0]), (50.0, [1.0]), (100.0, [0.0])], IN_OUT, p)[0]
}

// glitch: which ghost state shows (0 = rest, 1..=3 = the three splits)
pub fn glitch(p: f32) -> usize {
  let p = p * 100.0;
  match p {
    p if p < 88.0 => 0,
    p if p < 90.0 => 1,
    p if p < 93.0 => 2,
    p if p < 95.0 => 3,
    _ => 0,
  }
}

// reveal's mask over the name: opacity at name-x fraction u in the moving 300%-wide mask
pub fn reveal_alpha(p: f32, x: f32, w: f32) -> f32 {
  let pct = 130.0 - 260.0 * p;
  let off = -2.0 * w * pct / 100.0;
  let u = ((x - off) / (3.0 * w)).rem_euclid(1.0);
  let k = [(0.0, 1.0), (0.3, 1.0), (0.5, 0.2), (0.7, 1.0), (1.0, 1.0)];
  for q in k.windows(2) {
    if u < q[1].0 {
      return q[0].1 + (q[1].1 - q[0].1) * (u - q[0].0) / (q[1].0 - q[0].0);
    }
  }
  1.0
}

// css hue-rotate as a 3x3 over rgb
pub fn hue_matrix(deg: f32) -> [f32; 9] {
  let (s, c) = deg.to_radians().sin_cos();
  [
    0.213 + 0.787 * c - 0.213 * s,
    0.715 - 0.715 * c - 0.715 * s,
    0.072 - 0.072 * c + 0.928 * s,
    0.213 - 0.213 * c + 0.143 * s,
    0.715 + 0.285 * c + 0.140 * s,
    0.072 - 0.072 * c - 0.283 * s,
    0.213 - 0.213 * c - 0.787 * s,
    0.715 - 0.715 * c + 0.715 * s,
    0.072 + 0.928 * c + 0.072 * s,
  ]
}

// one glyph's motion
pub struct LetterFx {
  pub xf: Aff,
  pub opacity: f32,
  pub hue: Option<f32>,
}

// per-glyph stagger in seconds at speed 1 (ripple runs backwards through the name)
fn step(k: Kind) -> f32 {
  match k {
    Kind::Wave => -0.09,
    Kind::Ripple => 0.18,
    Kind::Hop => -0.07,
    Kind::Twirl => -0.08,
    Kind::Type => -0.12,
    Kind::Tumble => -0.12,
    _ => 0.0,
  }
}

// glyph i of a name at wall time t. the glyph box is (lw, h)
pub fn letter_fx(e: &Effect, i: usize, t: f32, s: f32, lw: f32, h: f32) -> LetterFx {
  let per = period(e.kind, e.speed);
  // the stagger is a time offset: glyph i runs `i * step` of a (speed-scaled) second ahead
  let p = phase(t - i as f32 * step(e.kind) / e.speed, per);
  let (cx, cy) = (lw / 2.0, h / 2.0);
  let tau = std::f32::consts::TAU;
  let mut out = LetterFx { xf: Aff::ID, opacity: 1.0, hue: None };
  match e.kind {
    Kind::Wave => out.xf = Aff::translate(0.0, -4.0 * s * (1.0 - (p * tau).cos()) / 2.0),
    Kind::Ripple => out.hue = Some(p * 360.0),
    Kind::Hop => {
      let pulse = (1.0 - (p * 6.5 - 1.0).abs()).clamp(0.0, 1.0);
      out.xf = Aff::translate(0.0, -5.0 * s * pulse).then(Aff::scale(1.0, 1.0 + 0.06 * pulse).about(cx, h));
    }
    Kind::Twirl => out.xf = Aff::rotate(360.0 * ((p - 0.64) / 0.36).clamp(0.0, 1.0)).about(cx, cy),
    Kind::Type => out.opacity = ((p - 0.03) * 100.0).clamp(0.0, 1.0),
    Kind::Tumble => {
      let r1 = ((p - 0.60) / 0.15).clamp(0.0, 1.0);
      let r2 = ((p - 0.75) / 0.15).clamp(0.0, 1.0);
      // rotateX: the glyph squashes through edge-on and comes back mirrored
      out.xf = Aff::scale(1.0, (180.0 * (r1 + r2)).to_radians().cos()).about(cx, cy);
    }
    _ => {}
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fx(kind: Kind) -> Effect {
    Effect { kind, speed: 1.0, scale: 220.0, bounce: false, skew: false }
  }

  #[test]
  fn affine_round_trips() {
    let m = Aff::rotate(30.0).about(10.0, 5.0).then(Aff::translate(3.0, -2.0));
    let (x, y) = m.apply(7.0, 9.0);
    let (bx, by) = m.inverse().unwrap().apply(x, y);
    assert!((bx - 7.0).abs() < 1e-3 && (by - 9.0).abs() < 1e-3);
    assert_eq!(Aff::translate(2.0, -3.0).int_shift(), Some((2, -3)));
    assert_eq!(Aff::translate(0.5, 0.0).int_shift(), None);
  }

  #[test]
  fn coin_rests_then_flips_through_edge_on() {
    let e = fx(Kind::Coin);
    let sx = |p| self_xf(&e, p, 1.0, 40.0, 20.0).0[0];
    assert!((sx(0.3) - 1.0).abs() < 1e-5);
    assert!(sx(0.66).abs() < 1.0);
    assert!(sx(0.76) < -0.5, "mirrored past 90 degrees");
    assert!((sx(0.97) - 1.0).abs() < 1e-3);
  }

  #[test]
  fn twirl_holds_then_spins_once() {
    let e = fx(Kind::Twirl);
    let at = |p: f32| letter_fx(&e, 0, p * period(Kind::Twirl, 1.0), 1.0, 8.0, 20.0).xf;
    assert_eq!(at(0.5), Aff::ID);
    assert_ne!(at(0.8), Aff::ID);
    assert!((at(0.999).0[0] - 1.0).abs() < 0.01);
  }

  #[test]
  fn jitter_and_flicker_hold_their_frames() {
    let j = fx(Kind::Jitter);
    assert_eq!(self_xf(&j, 0.5, 1.0, 10.0, 10.0), Aff::ID);
    assert_eq!(self_xf(&j, 0.83, 1.0, 10.0, 10.0).int_shift(), Some((-1, 1)));
    let f = fx(Kind::Flicker);
    assert_eq!((opacity(&f, 0.5), opacity(&f, 0.73), opacity(&f, 0.85)), (1.0, 0.4, 0.55));
    assert_eq!((glitch(0.5), glitch(0.89), glitch(0.91), glitch(0.94), glitch(0.97)), (0, 1, 2, 3, 0));
  }

  #[test]
  fn letters_are_staggered_and_ripple_runs_backwards() {
    let w = fx(Kind::Wave);
    let y = |i, t| letter_fx(&w, i, t, 1.0, 8.0, 20.0).xf.0[5];
    assert!((y(0, 0.0)).abs() < 1e-5);
    assert!(y(1, 0.0) < -0.01, "glyph 1 is already rising");
    assert!((hue_matrix(0.0)[0] - 1.0).abs() < 1e-3);
    let r = fx(Kind::Ripple);
    let h = |i| letter_fx(&r, i, 1.0, 1.0, 8.0, 20.0).hue.unwrap();
    assert!(h(1) < h(0) || h(0) < 1.0);
  }

  #[test]
  fn luminance_period_is_floored() {
    assert_eq!(period(Kind::Flicker, 3.0), 1.0);
    assert_eq!(period(Kind::Twirl, 3.0), 1.2);
  }

  // ground truth: the ext's compiled css (compilePaintCss) played in chrome with every animation
  // paused at p = 0, 0.05 .. 1 of its cycle, the computed transform / opacity / hue-rotate read back
  // and conjugated by the element's transform-origin, stored as thousandths. names 36x20 (self) and 12x20 (per glyph) boxes.
  // motion_ref.json is regenerated from the ext, never edited by hand
  #[test]
  fn every_motion_matches_the_exts_computed_style() {
    let data: serde_json::Value = serde_json::from_str(include_str!("motion_ref.json")).unwrap();
    let mut seen = 0;
    for (id, d) in data.as_object().unwrap() {
      let kind = Kind::parse(id).unwrap();
      let e = fx(kind);
      assert_eq!(d["period"].as_f64().unwrap() as f32, kind.period(), "{id} period");
      let per = period(kind, 1.0);
      for (i, rows) in d["rows"].as_array().unwrap().iter().enumerate() {
        for r in rows.as_array().unwrap() {
          let p = r["p"].as_f64().unwrap() as f32;
          // keyframe edges are ambiguous to a float: sample a hair inside the interval
          let t = (p + 1e-5) * per;
          let (xf, op, hue) = if d["letter"].as_bool().unwrap() {
            let l = letter_fx(&e, i, t, 1.0, 12.0, 20.0);
            (l.xf, l.opacity, l.hue)
          } else {
            (self_xf(&e, phase(t, per), 1.0, 36.0, 20.0), opacity(&e, phase(t, per)), None)
          };
          let want: Vec<f32> = r["m"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32 / 1000.0).collect();
          for (k, (a, b)) in xf.0.iter().zip(&want).enumerate() {
            assert!((a - b).abs() < 0.06, "{id} glyph {i} p={p} m[{k}]: ours {a} ext {b}");
          }
          assert!((op - r["op"].as_f64().unwrap() as f32 / 1000.0).abs() < 0.01, "{id} p={p} opacity {op}");
          if let Some(h) = r["hue"].as_f64() {
            let ours = hue.unwrap_or(f32::NAN);
            assert!((ours - h as f32 / 1000.0).abs() < 0.5, "{id} glyph {i} p={p} hue {ours} vs {h} (thousandths)");
          }
        }
        seen += 1;
      }
    }
    assert!(seen >= 13, "{seen}");
  }
}
