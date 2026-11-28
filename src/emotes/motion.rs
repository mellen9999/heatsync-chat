//! the modifiers that move (`p!`, `s!`, `ffzSpin`, …), frame by frame. every number is the extension's
//! src/multichat/styles/10-emotes.css (the site keeps the same keyframes): the same durations, the same
//! keyframes, the same timing. in css each effect is a class that sets `animation`, so an emote with several
//! runs only the one whose rule comes last in that file; `Motion`'s order is that file's order

use std::f32::consts::PI;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Motion {
  Rainbow,
  Party,
  Spin,
  Shake,
  Hyper,
  Bounce,
  Jam,
  Slide,
  Arrive,
  Leave,
}

// where a moving picture is at one instant, in its own frame and before its modifiers' turn and stretch
// (the ext puts those on the emote's wrapper and the animation on the picture inside it)
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pose {
  // shift, in css px
  pub dx: f32,
  pub dy: f32,
  // shift, in widths of the picture (`translate: -50%`)
  pub dw: f32,
  // turn, degrees clockwise
  pub rot: f32,
  pub sx: f32,
  pub sy: f32,
  // what `--hs-fx-hue` is: the turn of the colours the party and rainbow looks take, degrees
  pub hue: f32,
}

impl Pose {
  pub const REST: Pose = Pose { dx: 0.0, dy: 0.0, dw: 0.0, rot: 0.0, sx: 1.0, sy: 1.0, hue: 0.0 };
}

// css `animation-timing-function`: each step between two keyframes is straight, or (`step-start`) jumps
// to the next keyframe's value the moment it starts
#[derive(Clone, Copy, PartialEq)]
enum Timing {
  Linear,
  StepStart,
}

const SHAKE: [(f32, f32); 11] = [
  (0., 1.),
  (2., 0.),
  (1., -2.),
  (-2., 1.),
  (0., -1.),
  (2., 2.),
  (-1., -1.),
  (-2., 2.),
  (2., 1.),
  (-1., -2.),
  (1., 0.),
];
const HYPER: [(f32, f32); 11] = [
  (1., 1.),
  (-1., -2.),
  (-3., 0.),
  (3., 2.),
  (1., -1.),
  (-1., 2.),
  (-3., 1.),
  (3., 1.),
  (-1., -1.),
  (1., 2.),
  (1., -2.),
];
const BOUNCE: [(f32, f32, f32); 13] = [
  (0., 0.8, 1.),
  (10., 0.9, 0.8),
  (20., 1., 0.4),
  (25., 1.2, 0.3),
  (30., 1., 0.4),
  (40., 0.9, 0.8),
  (50., 0.8, 1.),
  (60., 0.9, 0.8),
  (70., 1., 0.4),
  (75., 1.2, 0.3),
  (80., 1., 0.4),
  (90., 0.9, 0.8),
  (100., 0.8, 1.),
];
// translate x, y and rotate
const JAM: [(f32, f32, f32); 11] = [
  (-2., -2., -6.),
  (-1.5, -2., -8.),
  (1., -1.5, -8.),
  (3., 2.5, -6.),
  (3., 4., -2.),
  (2., 4., 3.),
  (1., 4., 3.),
  (-0.5, 3., 2.),
  (-1.25, 1., 0.),
  (-1.75, -0.5, -2.),
  (-2., -2., -5.),
];
// at percent: translate x and scale
const ARRIVE: [(f32, f32, f32); 4] = [(0., -18., 0.1), (20., -18., 0.1), (45., 0., 1.), (100., 0., 1.)];
const LEAVE: [(f32, f32, f32); 4] = [(0., 0., 1.), (55., 0., 1.), (80., 18., 0.1), (100., 18., 0.1)];

impl Motion {
  // every one, in the css's order (= by `bit`)
  pub const ALL: [Motion; 10] = [
    Motion::Rainbow,
    Motion::Party,
    Motion::Spin,
    Motion::Shake,
    Motion::Hyper,
    Motion::Bounce,
    Motion::Jam,
    Motion::Slide,
    Motion::Arrive,
    Motion::Leave,
  ];

  pub fn bit(self) -> u16 {
    1 << self as u16
  }

  // one round, ms
  pub fn period_ms(self) -> u32 {
    match self {
      Motion::Rainbow => 2000,
      Motion::Party | Motion::Spin | Motion::Slide => 1500,
      Motion::Shake | Motion::Bounce => 500,
      Motion::Hyper => 100,
      Motion::Jam => 600,
      Motion::Arrive | Motion::Leave => 3000,
    }
  }

  // the pose `ms` into the animation (the clock wraps by the period)
  pub fn pose(self, ms: u64) -> Pose {
    let p = (ms % self.period_ms() as u64) as f32 / self.period_ms() as f32;
    let mut out = Pose::REST;
    match self {
      Motion::Rainbow | Motion::Party => out.hue = 360.0 * p,
      Motion::Spin => out.rot = 360.0 * p,
      Motion::Shake => (out.dx, out.dy) = even(&SHAKE, p, Timing::StepStart),
      Motion::Hyper => (out.dx, out.dy) = even(&HYPER, p, Timing::Linear),
      Motion::Bounce => (out.sx, out.sy) = at(&BOUNCE, p),
      Motion::Jam => {
        let (x, y) = even(&JAM.map(|k| (k.0, k.1)), p, Timing::Linear);
        let (r, _) = even(&JAM.map(|k| (k.2, 0.0)), p, Timing::Linear);
        (out.dx, out.dy, out.rot) = (x, y, r);
      }
      Motion::Slide => out.dw = p - 0.5,
      Motion::Arrive | Motion::Leave => {
        let (x, s) = at(if self == Motion::Arrive { &ARRIVE } else { &LEAVE }, p);
        (out.dx, out.sx, out.sy) = (x, s, s);
      }
    }
    out
  }
}

fn lerp(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
  (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

// keyframes every 10%, `p` in 0..1
fn even(k: &[(f32, f32); 11], p: f32, timing: Timing) -> (f32, f32) {
  let f = p * 10.0;
  let i = (f.floor() as usize).min(9);
  match timing {
    Timing::StepStart => k[i + 1],
    Timing::Linear => lerp(k[i], k[i + 1], f - i as f32),
  }
}

// keyframes at their own percents (first 0, last 100), linear between
fn at<const N: usize>(k: &[(f32, f32, f32); N], p: f32) -> (f32, f32) {
  let pc = p * 100.0;
  let j = k.iter().rposition(|f| f.0 <= pc).unwrap_or(0).min(N - 2);
  let (a, b) = (k[j], k[j + 1]);
  let t = if b.0 > a.0 { ((pc - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) } else { 0.0 };
  lerp((a.1, a.2), (b.1, b.2), t)
}

// an affine map: x' = a x + b y + e, y' = c x + d y + f
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Affine([f32; 6]);

impl Affine {
  pub fn scale(sx: f32, sy: f32) -> Affine {
    Affine([sx, 0.0, 0.0, sy, 0.0, 0.0])
  }

  // clockwise on screen (y down), as css `rotate`
  pub fn rotate(deg: f32) -> Affine {
    let (s, c) = (deg * PI / 180.0).sin_cos();
    Affine([c, -s, s, c, 0.0, 0.0])
  }

  pub fn shift(x: f32, y: f32) -> Affine {
    Affine([1.0, 0.0, 0.0, 1.0, x, y])
  }

  // `self` after `o`: (self ∘ o)(p) = self(o(p))
  pub fn then_from(self, o: Affine) -> Affine {
    let [a, b, c, d, e, f] = self.0;
    let [p, q, r, s, t, u] = o.0;
    Affine([a * p + b * r, a * q + b * s, c * p + d * r, c * q + d * s, a * t + b * u + e, c * t + d * u + f])
  }

  pub fn apply(&self, (x, y): (f32, f32)) -> (f32, f32) {
    let [a, b, c, d, e, f] = self.0;
    (a * x + b * y + e, c * x + d * y + f)
  }

  // None when it squashes the picture flat
  pub fn inverse(&self) -> Option<Affine> {
    let [a, b, c, d, e, f] = self.0;
    let det = a * d - b * c;
    if det.abs() < 1e-6 {
      return None;
    }
    let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
    Some(Affine([ia, ib, ic, id, -(ia * e + ib * f), -(ic * e + id * f)]))
  }
}

impl Pose {
  // the pose as a map of the picture's own points (its middle at 0, 0), `w`×`h` px drawn with `k` px to a css
  // px. css composes `translate`, `rotate`, `scale` in that order about the transform origin: the middle,
  // or the bottom middle for a bounce (whose class sets `transform-origin: bottom center`)
  pub fn map(&self, (w, h): (f32, f32), k: f32, bottom: bool) -> Affine {
    let oy = if bottom { h / 2.0 } else { 0.0 };
    Affine::shift(self.dx * k + self.dw * w, self.dy * k + oy)
      .then_from(Affine::rotate(self.rot))
      .then_from(Affine::scale(self.sx, self.sy))
      .then_from(Affine::shift(0.0, -oy))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
  }

  // each number below is read off 10-emotes.css by hand
  #[test]
  fn the_keyframes_are_the_extensions() {
    // shake jumps to the next keyframe the moment a step starts: 0-50ms is the 10% keyframe
    let s = |ms| Motion::Shake.pose(ms);
    assert_eq!((s(0).dx, s(0).dy), (2.0, 0.0));
    assert_eq!((s(49).dx, s(49).dy), (2.0, 0.0));
    assert_eq!((s(50).dx, s(50).dy), (1.0, -2.0));
    assert_eq!((s(475).dx, s(475).dy), (1.0, 0.0));
    assert_eq!(s(500), s(0), "500ms a round");
    // hyper is straight between its keyframes, every 10ms
    let h = Motion::Hyper.pose(5);
    assert!(near(h.dx, 0.0) && near(h.dy, -0.5), "{h:?}");
    // bounce squashes at a quarter: scale 1.2 .3
    let b = Motion::Bounce.pose(125);
    assert!(near(b.sx, 1.2) && near(b.sy, 0.3), "{b:?}");
    let b = Motion::Bounce.pose(0);
    assert!(near(b.sx, 0.8) && near(b.sy, 1.0));
    // jam: halfway through, 2px 4px and 3deg
    let j = Motion::Jam.pose(300);
    assert!(near(j.dx, 2.0) && near(j.dy, 4.0) && near(j.rot, 3.0), "{j:?}");
    // spin: a quarter round is 90deg; party and rainbow turn the colours at their own pace
    assert!(near(Motion::Spin.pose(375).rot, 90.0));
    assert!(near(Motion::Party.pose(750).hue, 180.0));
    assert!(near(Motion::Rainbow.pose(500).hue, 90.0));
    // slide: half a width back to half a width on
    assert!(near(Motion::Slide.pose(0).dw, -0.5) && near(Motion::Slide.pose(750).dw, 0.0));
    // arrive: small and left, held, then grown in place by 45%
    let a = Motion::Arrive.pose(300);
    assert!(near(a.dx, -18.0) && near(a.sx, 0.1));
    let a = Motion::Arrive.pose(1350);
    assert!(near(a.dx, 0.0) && near(a.sx, 1.0));
    let l = Motion::Leave.pose(2400);
    assert!(near(l.dx, 18.0) && near(l.sy, 0.1));
    // everything else stands still
    let p = Motion::Party.pose(750);
    assert!(p.dx == 0.0 && p.rot == 0.0 && p.sx == 1.0);
  }

  #[test]
  fn the_css_order_is_the_order() {
    assert!(Motion::ALL.windows(2).all(|w| w[0] < w[1]));
    assert!(Motion::ALL.iter().enumerate().all(|(i, m)| m.bit() == 1 << i));
  }

  #[test]
  fn a_pose_maps_the_pictures_points() {
    // a quarter turn takes the right edge's middle to the bottom's
    let m = Pose { rot: 90.0, ..Pose::REST }.map((28.0, 28.0), 1.0, false);
    let (x, y) = m.apply((14.0, 0.0));
    assert!(near(x, 0.0) && near(y, 14.0));
    // a bounce squashes toward the bottom: the bottom middle stays put
    let m = Pose { sx: 1.2, sy: 0.3, ..Pose::REST }.map((28.0, 28.0), 1.0, true);
    let (x, y) = m.apply((0.0, 14.0));
    assert!(near(x, 0.0) && near(y, 14.0));
    let (_, top) = m.apply((0.0, -14.0));
    assert!(near(top, 14.0 - 28.0 * 0.3));
    // a shift in css px is scaled to the screen; a slide is in widths
    let m = Pose { dx: 2.0, dw: 0.5, ..Pose::REST }.map((28.0, 28.0), 2.0, false);
    assert_eq!(m.apply((0.0, 0.0)), (18.0, 0.0));
    let inv = Pose { rot: 30.0, sx: 0.5, ..Pose::REST }.map((28.0, 28.0), 1.0, false).inverse().unwrap();
    let back = inv.apply(Pose { rot: 30.0, sx: 0.5, ..Pose::REST }.map((28.0, 28.0), 1.0, false).apply((3.0, 4.0)));
    assert!(near(back.0, 3.0) && near(back.1, 4.0));
  }
}
