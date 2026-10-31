// an animated painted name: everything static is built once (glyph coverage, per-glyph boxes, the
// scene's tiles, shadow stacks); a frame is then a few cheap passes over small premultiplied
// buffers with no allocation: plate -> glyph fill -> shadows -> per-glyph motion -> whole-name
// motion -> near weather. mirrors the ext's compilePaintCss (paint-spec.js)
use super::fill::FillArt;
use super::motion::{self, Aff};
use super::raster::{Px, over};
use super::scene::SceneArt;
use super::stroke::{self, Stroke};
use super::{Cov, coverage, shadow_layer};
use crate::cosmetics::fx::{Base, BaseKind, Effect, Fx, Kind};
use crate::cosmetics::paint::Shadow;
use crate::ui::text::{Ink, Span, Text};
use crate::ui::theme;

const LUT: usize = 256;

type Lut = [u32; LUT];

fn mix(a: u32, b: u32, f: f32) -> u32 {
  let ch = |s: u32| {
    let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
    (x + (y - x) * f).round().clamp(0.0, 255.0) as u32
  };
  ch(16) << 16 | ch(8) << 8 | ch(0)
}

fn ramp(stops: &[(f32, u32)], t: f32) -> u32 {
  let (first, last) = (stops[0], stops[stops.len() - 1]);
  if t <= first.0 {
    return first.1;
  }
  for w in stops.windows(2) {
    if t < w[1].0 {
      let span = w[1].0 - w[0].0;
      return if span <= 0.0 { w[1].1 } else { mix(w[0].1, w[1].1, (t - w[0].0) / span) };
    }
  }
  last.1
}

fn lut_of(stops: &[(f32, u32)]) -> Lut {
  std::array::from_fn(|i| ramp(stops, i as f32 / (LUT - 1) as f32))
}

// the ext's seamless loop stops: positions pulled in 1%, the first color closing the ring
fn wrapped(stops: &[(f32, u32)]) -> Vec<(f32, u32)> {
  let mut v: Vec<(f32, u32)> = stops.iter().map(|&(p, c)| ((p * 100.0 * 0.99).round() / 100.0, c)).collect();
  v.push((1.0, stops[0].1));
  v
}

// hard bands: each color holds until the next stop, the last runs to the tile edge
fn bands_lut(stops: &[(f32, u32)]) -> Lut {
  std::array::from_fn(|i| {
    let t = i as f32 / LUT as f32;
    stops.iter().rev().find(|s| s.0 <= t).map_or(stops[0].1, |s| s.1)
  })
}

struct Filler {
  base: Base,
  lut: Lut,
  wrap: Lut,
  // an effect's stripes use the stops' own colors (a solid gets white as its second band)
  stripe: Vec<u32>,
}

impl Filler {
  fn new(base: &Base) -> Filler {
    let stops = &base.stops;
    let lut = match base.kind {
      BaseKind::Repeating if stops.len() > 1 => bands_lut(stops),
      _ => lut_of(stops),
    };
    let wrap = match base.kind {
      BaseKind::Repeating => bands_lut(stops),
      _ => lut_of(&wrapped(stops)),
    };
    let stripe = if stops.len() > 1 { stops.iter().map(|s| s.1).collect() } else { vec![stops[0].1, 0xffffff] };
    Filler { base: base.clone(), lut, wrap, stripe }
  }

  // a linear/banded gradient over a w×h tile at `deg`
  fn linear(&self, lut: &Lut, deg: f32, x: f32, y: f32, w: f32, h: f32) -> u32 {
    let (s, c) = deg.to_radians().sin_cos();
    let (cx, cy) = (x - w / 2.0, y - h / 2.0);
    let proj = cx * s - cy * c;
    if self.base.kind == BaseKind::Repeating && self.base.tile > 0.0 {
      let u = (proj / self.base.tile).rem_euclid(1.0);
      return lut[(u * LUT as f32) as usize % LUT];
    }
    let len = (w * s).abs() + (h * c).abs();
    let t = if len > 0.0 { proj / len + 0.5 } else { 0.5 };
    lut[(t.clamp(0.0, 1.0) * (LUT - 1) as f32).round() as usize]
  }

  fn conic(&self, lut: &Lut, from: f32, x: f32, y: f32, w: f32, h: f32) -> u32 {
    let a = ((x - w / 2.0).atan2(-(y - h / 2.0)).to_degrees() - from).rem_euclid(360.0) / 360.0;
    lut[((a * LUT as f32) as usize).min(LUT - 1)]
  }

  // the resting fill at (x, y) of a w×h box
  fn rest(&self, x: f32, y: f32, w: f32, h: f32) -> u32 {
    match self.base.kind {
      BaseKind::Solid => self.base.stops[0].1,
      BaseKind::Conic => self.conic(&self.lut, self.base.angle, x, y, w, h),
      _ => self.linear(&self.lut, self.base.angle, x, y, w, h),
    }
  }

  // the fill with a paint effect at phase p, in a w×h box; `s` = css px scale
  #[allow(clippy::too_many_arguments)]
  fn effect(&self, e: &Effect, p: f32, x: f32, y: f32, w: f32, h: f32, s: f32) -> u32 {
    let tau = std::f32::consts::TAU;
    match e.kind {
      Kind::Pan => {
        let (sn, cs) = self.base.angle.to_radians().sin_cos();
        let end_of = |v: f32| if v > 0.0 { -e.scale } else { e.scale };
        let mv = |end: f32| if e.bounce { end / 2.0 * (1.0 - (p * tau).cos()) } else { end * p };
        let k = e.scale / 100.0;
        let (tw, th, ox, oy) = if sn.abs() >= cs.abs() {
          let tw = w * k;
          (tw, h, (w - tw) * mv(end_of(sn)) / 100.0, 0.0)
        } else {
          let th = h * k;
          (w, th, 0.0, (h - th) * mv(-end_of(cs)) / 100.0)
        };
        let lut = if self.base.kind == BaseKind::Repeating { &self.lut } else { &self.wrap };
        self.linear(lut, self.base.angle, (x - ox).rem_euclid(tw.max(1.0)), (y - oy).rem_euclid(th.max(1.0)), tw, th)
      }
      Kind::Conic => self.conic(&self.wrap, self.base.angle + 360.0 * p, x, y, w, h),
      Kind::Glint => {
        let base = self.rest(x, y, w, h);
        let pct = 50.0 + 160.0 * (p * std::f32::consts::PI).cos();
        let bw = w * 2.5;
        let ox = (w - bw) * pct / 100.0;
        let (gx, gy) = (x - ox, y);
        if gx < 0.0 || gx >= bw {
          return base;
        }
        let (sn, cs) = 115f32.to_radians().sin_cos();
        let len = (bw * sn).abs() + (h * cs).abs();
        let t = ((gx - bw / 2.0) * sn - (gy - h / 2.0) * cs) / len + 0.5;
        let a = if t < 0.38 || t > 0.62 {
          0.0
        } else if t < 0.5 {
          0.8 * (t - 0.38) / 0.12
        } else {
          0.8 * (0.62 - t) / 0.12
        };
        mix(base, 0xffffff, a)
      }
      Kind::Stripes => {
        // 5px bands at 45 degrees, the box-sized tile sliding along x by one period per loop
        let n = self.stripe.len() as f32;
        let (u, v, bw, bh) = (x / s, y / s, w / s, h / s);
        let shift = n * 5.0 * std::f32::consts::SQRT_2 * p;
        let (tu, tv) = ((u - shift).rem_euclid(bw.max(1.0)), v);
        let (sn, cs) = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
        let len = (bw + bh) * sn;
        let d = (tu - bw / 2.0) * sn - (tv - bh / 2.0) * cs + len / 2.0;
        self.stripe[((d.rem_euclid(n * 5.0) / 5.0) as usize).min(self.stripe.len() - 1)]
      }
      Kind::Stardust => {
        let base = self.rest(x, y, w, h);
        let (u, v) = (x / s, y / s);
        // two dot fields drifting at different rates; the first listed paints on top
        let dot = |tw: f32, th: f32, ox: f32, oy: f32, solid: f32, edge: f32, a: f32| {
          let (tu, tv) = ((u - ox).rem_euclid(tw), (v - oy).rem_euclid(th));
          let d = ((tu - tw / 2.0).powi(2) + (tv - th / 2.0).powi(2)).sqrt();
          a * ((edge - d) / (edge - solid)).clamp(0.0, 1.0)
        };
        let under = dot(13.0, 11.0, 4.0 - 26.0 * p, 3.0 + 22.0 * p, 0.5, 0.9, 0.667);
        let over = dot(9.0, 7.0, -27.0 * p, 21.0 * p, 0.7, 1.1, 1.0);
        mix(mix(base, 0xffffff, under), 0xffffff, over)
      }
      _ => self.rest(x, y, w, h),
    }
  }
}

fn stack(alpha: &[f32], w: usize, h: usize, shadows: &[Shadow]) -> Vec<Px> {
  let mut out = vec![[0u8; 4]; w * h];
  // css text-shadow list: the first sits on top
  for s in shadows.iter().rev() {
    let a = shadow_layer(alpha, w, h, s);
    let c = [s.color >> 16 & 255, s.color >> 8 & 255, s.color & 255];
    for (d, &a) in out.iter_mut().zip(&a) {
      let q = (a * 255.0 + 0.5) as u32;
      if q > 0 {
        over(d, [(c[0] * q / 255) as u8, (c[1] * q / 255) as u8, (c[2] * q / 255) as u8, q as u8]);
      }
    }
  }
  out
}

fn sh(dx: f32, dy: f32, blur: f32, color: u32) -> Shadow {
  Shadow { dx, dy, blur, color }
}

fn glow(c: u32, strength: u8) -> Vec<Shadow> {
  let [r1, r2] = if strength == 2 { [10.0, 26.0] } else { [6.0, 14.0] };
  vec![sh(0.0, 0.0, r1, 0xcc << 24 | c), sh(0.0, 0.0, r2, 0x66 << 24 | c)]
}

fn neon_set(c: u32, strength: u8, full: bool) -> Vec<Shadow> {
  let k = if strength == 2 { 1.6 } else { 1.0 };
  let r = |v: f32| (v * k).round();
  if full {
    vec![
      sh(0.0, 0.0, r(6.0), 0xcc << 24 | c),
      sh(0.0, 0.0, r(22.0), 0x88 << 24 | c),
      sh(0.0, 0.0, r(40.0), 0x44 << 24 | c),
    ]
  } else {
    vec![sh(0.0, 0.0, r(4.0), 0x80 << 24 | c), sh(0.0, 0.0, r(11.0), 0x40 << 24 | c)]
  }
}

pub struct Scratch {
  plate: Vec<Px>,
  glyph: Vec<Px>,
  sprite: Vec<Px>,
  name: Vec<Px>,
  shadow: Vec<Px>,
}

impl Scratch {
  pub fn new() -> Scratch {
    Scratch { plate: Vec::new(), glyph: Vec::new(), sprite: Vec::new(), name: Vec::new(), shadow: Vec::new() }
  }

  fn fit(&mut self, n: usize) {
    for v in [&mut self.plate, &mut self.glyph, &mut self.sprite, &mut self.name, &mut self.shadow] {
      v.resize(n, [0; 4]);
    }
  }
}

pub struct AnimArt {
  pub w: i32,
  pub h: i32,
  // baseline inside the bitmap, to line it up with the row's own baseline
  pub base: f32,
  fx: Fx,
  filler: Filler,
  cov: Vec<u8>,
  // pixels with ink, so a frame only shades those
  ink: Vec<u32>,
  // the fill boxes: one per glyph for a letter-split name, else the whole name
  boxes: Vec<(usize, usize)>,
  // which box each column belongs to
  box_of: Vec<u8>,
  glyph_rest: Vec<Px>,
  shadow_rest: Vec<Px>,
  neon: Option<(Vec<Px>, Vec<Px>)>,
  ghosts: Option<[Vec<Px>; 3]>,
  scene: Option<SceneArt>,
  fill: Option<FillArt>,
  // the fill moves: one gradient across the name, not one per glyph
  fill_whole: bool,
  // the fill's layers below every mover, flattened once
  fill_base: Vec<Px>,
  stroke: Option<Stroke>,
  // css px -> device px
  s: f32,
  motion: bool,
}

fn glyph_boxes(text: &mut Text, nick: &str, w: usize) -> Vec<(usize, usize)> {
  let spans = |n: usize| [Span { range: 0..n, ink: Ink(theme::WHITE), bold: true }];
  let mut cuts = vec![0usize];
  for (i, _) in nick.char_indices().skip(1).take(63) {
    let x = text.layout(&nick[..i], &spans(i), &[], 1e5).full_width().round() as usize;
    cuts.push(x.clamp(*cuts.last().unwrap_or(&0), w));
  }
  cuts.push(w);
  cuts.windows(2).filter(|c| c[1] > c[0]).map(|c| (c[0], c[1])).collect()
}

impl AnimArt {
  pub fn build(text: &mut Text, nick: &str, w: i32, fx: &Fx, motion: bool, ink_color: u32) -> AnimArt {
    let Cov { w, h, base, alpha } = coverage(text, nick, w);
    let (wu, hu) = (w as usize, h as usize);
    let cov: Vec<u8> = alpha.iter().map(|&a| (a * 255.0 + 0.5) as u8).collect();
    let ink = (0..wu * hu).filter(|&i| cov[i] > 0).map(|i| i as u32).collect();
    let boxes = if fx.split { glyph_boxes(text, nick, wu) } else { vec![(0, wu)] };
    let effects: &[Effect] = if motion { &fx.effects } else { &[] };
    let filler = Filler::new(&fx.base);
    let has = |k: Kind| effects.iter().any(|e| e.kind == k);
    let s = (h as f32 / 20.0).clamp(0.5, 6.0);
    let fill = fx.fill.as_ref().map(|f| FillArt::new(f, s, motion, ink_color));
    let fill_moves = fill.as_ref().is_some_and(FillArt::moves);
    // shadows are a filter on the name: ripple and tumble cannot live under one, so they win
    let drops_on = !fx.shadows.is_empty() && !has(Kind::Ripple) && !has(Kind::Tumble);

    // the shadows the name wears at rest: its glow, else the plate's dark rim
    let backdrop = fx.scene.as_ref().is_some_and(|s| s.backdrop.is_some());
    let clip_text = fx.base.kind != BaseKind::Solid || fx.fill.is_some() || effects.iter().any(|e| e.kind.paints());
    let rim_hostile = has(Kind::Ripple) || has(Kind::Tumble);
    let animated_fill = fill_moves || effects.iter().any(|e| e.kind.paints());
    let rim = backdrop
      && fx.glow.is_none()
      && !has(Kind::Neon)
      && !drops_on
      && (!clip_text || (!rim_hostile && !animated_fill));
    let stroke = fx.outline.map(|o| stroke::stroke(&alpha, wu, hu, o.width * s / 2.0, o.color, clip_text));
    // the shadows are cast from the glyph and its stroke, and replace the glow and the rim
    let drop_px =
      drops_on.then(|| stroke::chain(stroke.as_ref().map_or(&alpha, |st| &st.outer), wu, hu, s, &fx.shadows));
    let glow_on = fx.glow.filter(|_| !drops_on);
    let rest_shadows = match (glow_on, rim) {
      (Some((c, st)), _) => glow(c, st),
      (None, true) => vec![sh(0.0, 1.0, 1.0, 0xdd << 24), sh(0.0, 0.0, 2.0, 0xaa << 24)],
      (None, false) => Vec::new(),
    };
    let beneath = |top: Vec<Px>| match &drop_px {
      Some(d) => top
        .into_iter()
        .zip(d)
        .map(|(t, &b)| {
          let mut p = b;
          over(&mut p, t);
          p
        })
        .collect(),
      None => top,
    };
    let shadow_rest = beneath(stack(&alpha, wu, hu, &if has(Kind::Neon) { Vec::new() } else { rest_shadows }));
    let neon = has(Kind::Neon).then(|| {
      let (c, st) = fx.glow.unwrap_or((0xff40af, 1));
      (beneath(stack(&alpha, wu, hu, &neon_set(c, st, false))), beneath(stack(&alpha, wu, hu, &neon_set(c, st, true))))
    });
    let ghosts = has(Kind::Glitch).then(|| {
      let g = |a: f32, b: f32| {
        stack(&alpha, wu, hu, &[sh(a, 0.0, 0.0, 0xff << 24 | 0x00ffff), sh(b, 0.0, 0.0, 0xff << 24 | 0xff00ff)])
      };
      [g(-2.0, 2.0), g(2.0, -2.0), g(-1.0, 1.0)].map(beneath)
    });
    // a glitch's resting frame is the user's own glow (or nothing), never the rim
    let shadow_rest = if has(Kind::Glitch) {
      beneath(stack(&alpha, wu, hu, &glow_on.map(|(c, st)| glow(c, st)).unwrap_or_default()))
    } else {
      shadow_rest
    };

    let mut art = AnimArt {
      w,
      h,
      base,
      fx: fx.clone(),
      filler,
      cov,
      ink,
      box_of: (0..wu).map(|x| boxes.iter().position(|b| x >= b.0 && x < b.1).unwrap_or(0) as u8).collect(),
      boxes,
      glyph_rest: Vec::new(),
      shadow_rest,
      neon,
      ghosts,
      scene: fx.scene.as_ref().map(|sc| SceneArt::new(sc, wu, hu)),
      fill,
      fill_whole: fill_moves,
      fill_base: Vec::new(),
      stroke,
      s,
      motion,
    };
    art.bake_fill();
    let mut rest = vec![[0u8; 4]; wu * hu];
    art.shade(None, -1.0, &mut rest);
    art.glyph_rest = rest;
    art
  }

  // every frame of this name is the same: nothing to tick
  pub fn animated(&self) -> bool {
    self.motion
      && (!self.fx.effects.is_empty() || self.fill_whole || self.scene.as_ref().is_some_and(SceneArt::animated))
  }

  // a tick needs the display's rate: motion effects run smooth, a scene alone at the ext's 24 steps/s
  pub fn smooth(&self) -> bool {
    self.motion && (!self.fx.effects.is_empty() || self.fill_whole)
  }

  // fills the glyph pixels at phase p of the paint effect (or at rest)
  // the fill's box around column x: the whole name when it moves, else the glyph's own
  fn fill_box(&self, x: usize) -> (usize, usize) {
    if self.fill_whole { (0, self.w as usize) } else { self.boxes[self.box_of[x] as usize] }
  }

  fn bake_fill(&mut self) {
    let Some(f) = &self.fill else { return };
    let w = self.w as usize;
    let mut base = vec![[0u8; 4]; w * self.h as usize];
    for &i in &self.ink {
      let i = i as usize;
      let (x, y) = (i % w, i / w);
      let (x0, x1) = self.fill_box(x);
      base[i] = f.base_px((x - x0) as f32 + 0.5, y as f32 + 0.5, (x1 - x0) as f32, self.h as f32);
    }
    self.fill_base = base;
  }

  fn shade(&self, e: Option<&Effect>, p: f32, out: &mut [Px]) {
    if let Some(f) = &self.fill {
      // the fill replaces the paint-slot effect; `p` is wall time here, -1 = the resting frame
      let frame = if p < 0.0 { FillArt::rest_frame() } else { f.frame(p) };
      let w = self.w as usize;
      for &i in &self.ink {
        let i = i as usize;
        let (x, y) = (i % w, i / w);
        let (x0, x1) = self.fill_box(x);
        let c = f.px(self.fill_base[i], (x - x0) as f32 + 0.5, y as f32 + 0.5, (x1 - x0) as f32, self.h as f32, &frame);
        let a = self.cov[i] as u32;
        out[i] = [
          (c[0] as u32 * a / 255) as u8,
          (c[1] as u32 * a / 255) as u8,
          (c[2] as u32 * a / 255) as u8,
          (c[3] as u32 * a / 255) as u8,
        ];
      }
      return;
    }
    let w = self.w as usize;
    for &i in &self.ink {
      let i = i as usize;
      let (x, y) = (i % w, i / w);
      let (x0, x1) = self.boxes[self.box_of[x] as usize];
      let (bx, by, bw, bh) = ((x - x0) as f32 + 0.5, y as f32 + 0.5, (x1 - x0) as f32, self.h as f32);
      let rgb = match e {
        Some(e) => self.filler.effect(e, p, bx, by, bw, bh, self.s),
        None => self.filler.rest(bx, by, bw, bh),
      };
      let a = self.cov[i] as u32;
      out[i] = [
        ((rgb >> 16 & 255) * a / 255) as u8,
        ((rgb >> 8 & 255) * a / 255) as u8,
        ((rgb & 255) * a / 255) as u8,
        a as u8,
      ];
    }
  }

  fn phase_of(&self, e: &Effect, t: f32) -> f32 {
    motion::phase(t, motion::period(e.kind, e.speed))
  }

  // paints the frame at wall time `t` (None = the designed resting frame) into `sc`; returns the
  // premultiplied w×h bitmap
  pub fn render<'a>(&self, t: Option<f32>, sc: &'a mut Scratch) -> &'a [Px] {
    let (w, h) = (self.w as usize, self.h as usize);
    sc.fit(w * h);
    let live = t.filter(|_| self.motion);
    match &self.scene {
      Some(scene) => scene.paint_under(live, &mut sc.plate),
      None => sc.plate.iter_mut().for_each(|p| *p = [0; 4]),
    }
    let effects: &[Effect] = if live.is_some() { &self.fx.effects } else { &[] };
    let t = live.unwrap_or(0.0);

    // glyph fill
    match effects.iter().find(|e| e.kind.paints()) {
      None if self.fill_whole && live.is_some() => {
        sc.glyph.iter_mut().for_each(|p| *p = [0; 4]);
        self.shade(None, t, &mut sc.glyph);
      }
      Some(e) => {
        sc.glyph.iter_mut().for_each(|p| *p = [0; 4]);
        self.shade(Some(e), self.phase_of(e, t), &mut sc.glyph);
      }
      None => sc.glyph.copy_from_slice(&self.glyph_rest),
    }
    // shadows: neon breathes between two precomputed stacks, a glitch holds one of three ghosts
    let shadow: &[Px] =
      match (&self.neon, &self.ghosts, effects.iter().find(|e| matches!(e.kind, Kind::Neon | Kind::Glitch))) {
        (Some((a, b)), _, Some(e)) if e.kind == Kind::Neon => {
          let k = (motion::neon(self.phase_of(e, t)) * 256.0) as u32;
          for ((d, a), b) in sc.shadow.iter_mut().zip(a).zip(b) {
            *d = std::array::from_fn(|c| ((a[c] as u32 * (256 - k) + b[c] as u32 * k) >> 8) as u8);
          }
          &sc.shadow
        }
        (None, Some(g), Some(e)) if e.kind == Kind::Glitch => match motion::glitch(self.phase_of(e, t)) {
          0 => &self.shadow_rest,
          i => &g[i - 1],
        },
        (Some((a, _)), _, None) => a,
        _ => &self.shadow_rest,
      };
    for (i, ((d, g), s)) in sc.sprite.iter_mut().zip(&sc.glyph).zip(shadow).enumerate() {
      *d = *s;
      match &self.stroke {
        // an opaque fill hides the stroke's inner half; a see-through one shows it
        Some(st) if st.top => {
          over(d, *g);
          over(d, st.px[i]);
        }
        Some(st) => {
          over(d, st.px[i]);
          over(d, *g);
        }
        None => over(d, *g),
      }
    }

    // per-glyph motion
    let letter = effects.iter().find(|e| e.kind.per_letter());
    let name: &mut Vec<Px> = match letter {
      Some(e) => {
        sc.name.iter_mut().for_each(|p| *p = [0; 4]);
        for (i, &(x0, x1)) in self.boxes.iter().enumerate() {
          let lf = motion::letter_fx(e, i, t, self.s, (x1 - x0) as f32, h as f32);
          let hue = lf.hue.map(motion::hue_matrix);
          // letter_fx works in the glyph's own cell; the sprite is the whole name, so move the cell home
          let xf = Aff::translate(x0 as f32, 0.0).then(lf.xf).then(Aff::translate(-(x0 as f32), 0.0));
          draw(&mut sc.name, (w, h), &sc.sprite, (x0, x1), &xf, lf.opacity, hue.as_ref());
        }
        &mut sc.name
      }
      None => &mut sc.sprite,
    };

    // whole-name look: hue, breathe, mask, then its transform
    for e in effects {
      let p = self.phase_of(e, t);
      match e.kind {
        Kind::Hue => hue_rotate(name, &motion::hue_matrix(p * 360.0)),
        Kind::Pulse | Kind::Flicker => fade(name, motion::opacity(e, p)),
        Kind::Reveal => {
          for (i, px) in name.iter_mut().enumerate() {
            let a = motion::reveal_alpha(p, (i % w) as f32 + 0.5, w as f32);
            *px = std::array::from_fn(|c| (px[c] as f32 * a) as u8);
          }
        }
        _ => {}
      }
    }
    let mut xf = Aff::ID;
    // one whole-name transform: a motion effect, else pan's skew (a css animation wins over it)
    let moving = effects.iter().find(|e| {
      matches!(e.kind, Kind::Coin | Kind::Heli | Kind::Float | Kind::Heart | Kind::Wobble | Kind::Swing | Kind::Jitter)
    });
    if let Some(e) = moving.or_else(|| effects.iter().find(|e| e.kind == Kind::Pan && e.skew)) {
      xf = motion::self_xf(e, self.phase_of(e, t), self.s, w as f32, h as f32);
    }
    draw(&mut sc.plate, (w, h), name, (0, w), &xf, 1.0, None);
    if let Some(scene) = &self.scene {
      scene.paint_over(live, &mut sc.plate);
    }
    &sc.plate
  }
}

fn fade(buf: &mut [Px], k: f32) {
  if k >= 0.999 {
    return;
  }
  let q = (k.clamp(0.0, 1.0) * 256.0) as u32;
  for p in buf {
    *p = [
      ((p[0] as u32 * q) >> 8) as u8,
      ((p[1] as u32 * q) >> 8) as u8,
      ((p[2] as u32 * q) >> 8) as u8,
      ((p[3] as u32 * q) >> 8) as u8,
    ];
  }
}

fn hue_rotate(buf: &mut [Px], m: &[f32; 9]) {
  for p in buf {
    if p[3] == 0 {
      continue;
    }
    let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
    let a = p[3] as f32;
    p[0] = (m[0] * r + m[1] * g + m[2] * b).clamp(0.0, a) as u8;
    p[1] = (m[3] * r + m[4] * g + m[5] * b).clamp(0.0, a) as u8;
    p[2] = (m[6] * r + m[7] * g + m[8] * b).clamp(0.0, a) as u8;
  }
}

// draws columns [c0, c1) of `src` through `m` onto `dst`, with an opacity and optional hue rotation
fn draw(
  dst: &mut [Px],
  (w, h): (usize, usize),
  src: &[Px],
  (c0, c1): (usize, usize),
  m: &Aff,
  alpha: f32,
  hue: Option<&[f32; 9]>,
) {
  if alpha <= 0.0 || c1 <= c0 {
    return;
  }
  let q = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
  let fin = |mut p: Px| -> Px {
    if let Some(m) = hue {
      let mut one = [p];
      hue_rotate(&mut one, m);
      p = one[0];
    }
    if q < 256 {
      p = [
        ((p[0] as u32 * q) >> 8) as u8,
        ((p[1] as u32 * q) >> 8) as u8,
        ((p[2] as u32 * q) >> 8) as u8,
        ((p[3] as u32 * q) >> 8) as u8,
      ];
    }
    p
  };
  if let Some((dx, dy)) = m.int_shift() {
    for y in 0..h as i32 {
      let ty = y + dy;
      if ty < 0 || ty >= h as i32 {
        continue;
      }
      for x in c0..c1 {
        let tx = x as i32 + dx;
        let s = src[y as usize * w + x];
        if tx >= 0 && (tx as usize) < w && s[3] > 0 {
          over(&mut dst[ty as usize * w + tx as usize], fin(s));
        }
      }
    }
    return;
  }
  let Some(inv) = m.inverse() else { return };
  let corners =
    [(c0 as f32, 0.0), (c1 as f32, 0.0), (c0 as f32, h as f32), (c1 as f32, h as f32)].map(|(x, y)| m.apply(x, y));
  let lo = |f: fn((f32, f32)) -> f32| corners.iter().map(|&c| f(c)).fold(f32::MAX, f32::min);
  let hi = |f: fn((f32, f32)) -> f32| corners.iter().map(|&c| f(c)).fold(f32::MIN, f32::max);
  let (x0, x1) = ((lo(|c| c.0).floor().max(0.0)) as usize, (hi(|c| c.0).ceil().max(0.0) as usize).min(w));
  let (y0, y1) = ((lo(|c| c.1).floor().max(0.0)) as usize, (hi(|c| c.1).ceil().max(0.0) as usize).min(h));
  let at = |x: i32, y: i32| -> [f32; 4] {
    if x < c0 as i32 || x >= c1 as i32 || y < 0 || y >= h as i32 {
      return [0.0; 4];
    }
    let p = src[y as usize * w + x as usize];
    [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]
  };
  for y in y0..y1 {
    for x in x0..x1 {
      let (u, v) = inv.apply(x as f32 + 0.5, y as f32 + 0.5);
      let (u, v) = (u - 0.5, v - 0.5);
      let (iu, iv) = (u.floor(), v.floor());
      let (fu, fv) = (u - iu, v - iv);
      let (iu, iv) = (iu as i32, iv as i32);
      let (a, b, c, d) = (at(iu, iv), at(iu + 1, iv), at(iu, iv + 1), at(iu + 1, iv + 1));
      let mut p = [0u8; 4];
      for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fu;
        let bot = c[k] + (d[k] - c[k]) * fu;
        p[k] = (top + (bot - top) * fv + 0.5).clamp(0.0, 255.0) as u8;
      }
      if p[3] > 0 {
        over(&mut dst[y * w + x], fin([p[0].min(p[3]), p[1].min(p[3]), p[2].min(p[3]), p[3]]));
      }
    }
  }
}

#[cfg(test)]
mod tests_support {
  use super::*;
  use serde_json::json;

  pub fn fx(effects: serde_json::Value, scene: serde_json::Value) -> Fx {
    let base = Base { kind: BaseKind::Linear, angle: 90.0, tile: 0.0, stops: vec![(0.0, 0xff4040), (1.0, 0x4040ff)] };
    Fx::parse(&json!({"v": 2, "scene": scene, "effects": effects, "glow": null}), base).unwrap()
  }
}

#[cfg(test)]
mod tests {
  use super::tests_support::fx;
  use super::*;
  use crate::cosmetics::fx::{Backdrop, Weather};
  use serde_json::json;

  fn hash(px: &[Px]) -> u64 {
    px.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
      p.iter().fold(h, |h, &c| (h ^ u64::from(c)).wrapping_mul(0x100_0000_01b3))
    })
  }

  fn build(f: &Fx, motion: bool) -> (AnimArt, Scratch) {
    let mut text = Text::new(1.0);
    let w =
      text.layout("mellen", &[Span { range: 0..6, ink: Ink(theme::WHITE), bold: true }], &[], 1e5).full_width().ceil()
        as i32;
    (AnimArt::build(&mut text, "mellen", w, f, motion, 0xffffff), Scratch::new())
  }

  // every effect, alone: a deterministic frame at a fixed t, and it moves over the loop
  #[test]
  fn every_effect_renders_a_deterministic_frame_and_moves() {
    for &k in Kind::ALL {
      let f = fx(json!([{"id": k.id(), "speed": 1}]), json!(null));
      let (art, mut sc) = build(&f, true);
      assert!(art.animated(), "{k:?}");
      let rest = hash(art.render(None, &mut sc));
      let seen: Vec<u64> =
        [0.15f32, 1.1, 1.7, 2.2, 2.5, 2.6, 2.9, 3.4].iter().map(|&t| hash(art.render(Some(t), &mut sc))).collect();
      let again: Vec<u64> =
        [0.15f32, 1.1, 1.7, 2.2, 2.5, 2.6, 2.9, 3.4].iter().map(|&t| hash(art.render(Some(t), &mut sc))).collect();
      assert_eq!(seen, again, "{k:?} is deterministic");
      assert!(seen.iter().any(|&h| h != rest), "{k:?} leaves the resting frame");
    }
  }

  #[test]
  fn every_scene_kind_animates_inside_the_name_box() {
    for &b in Backdrop::ALL {
      let f = fx(json!([]), json!({"backdrop": {"id": b.id(), "tint": "#3080c0"}}));
      let (art, mut sc) = build(&f, true);
      let (w, h) = (art.w as usize, art.h as usize);
      let px = art.render(Some(0.7), &mut sc);
      assert_eq!(px.len(), w * h);
      assert!(px.iter().any(|p| p[3] == 255), "{b:?}");
    }
    for &wd in Weather::ALL {
      let f = fx(json!([]), json!({"backdrop": {"id": "terminal"}, "weather": {"id": wd.id(), "density": 2}}));
      let (art, mut sc) = build(&f, true);
      assert!(art.animated(), "{wd:?}");
      assert_ne!(hash(art.render(Some(0.3), &mut sc)), hash(art.render(Some(0.9), &mut sc)), "{wd:?}");
    }
  }

  #[test]
  fn motion_off_is_the_first_frame_and_never_ticks() {
    let f =
      fx(json!([{"id": "twirl"}, {"id": "coin"}]), json!({"backdrop": {"id": "synth"}, "weather": {"id": "petals"}}));
    let (art, mut sc) = build(&f, false);
    assert!(!art.animated() && !art.smooth());
    let a = hash(art.render(Some(0.4), &mut sc));
    assert_eq!(a, hash(art.render(Some(9.9), &mut sc)));
    assert_eq!(a, hash(art.render(None, &mut sc)));
  }

  #[test]
  fn plain_paint_with_a_backdrop_only_ticks_at_scene_rate() {
    let f = fx(json!([]), json!({"backdrop": {"id": "orbit"}}));
    let (art, _) = build(&f, true);
    assert!(art.animated() && !art.smooth());
    // a still backdrop (nothing moves) has nothing to tick
    let still = fx(json!([]), json!({"backdrop": {"id": "dawn"}}));
    let (a2, _) = build(&still, true);
    assert!(a2.animated(), "dawn's haze drifts");
  }

  #[test]
  fn coin_flip_mirrors_the_name_and_rest_matches_the_still() {
    let f = fx(json!([{"id": "coin"}]), json!(null));
    let (art, mut sc) = build(&f, true);
    let rest = art.render(None, &mut sc).to_vec();
    // coin rests for the first 55% of its 5s cycle: identical to the static frame
    assert_eq!(hash(&rest), hash(art.render(Some(1.0), &mut sc)));
    // after the half turn it is mirrored: red (left) <-> blue (right) swap sides
    let flipped = art.render(Some(3.8), &mut sc).to_vec();
    let side = |px: &[Px], x0: usize, x1: usize| {
      let w = art.w as usize;
      let (mut r, mut b) = (0u32, 0u32);
      for y in 0..art.h as usize {
        for x in x0..x1 {
          let p = px[y * w + x];
          r += p[0] as u32;
          b += p[2] as u32;
        }
      }
      (r, b)
    };
    let w = art.w as usize;
    let (rl, bl) = side(&rest, 0, w / 3);
    let (rf, bf) = side(&flipped, 0, w / 3);
    assert!(rl > bl, "rest: left is red");
    assert!(bf > rf, "flipped: left is blue");
  }

  #[test]
  fn letters_split_per_glyph_and_hold_the_name_width() {
    let f = fx(json!([{"id": "wave"}]), json!(null));
    let (art, _) = build(&f, true);
    assert_eq!(art.boxes.len(), 6);
    assert_eq!(art.boxes[0].0, 0);
    assert_eq!(art.boxes[5].1, art.w as usize);
  }

  #[test]
  fn painted_pixels_are_the_name_ink_only() {
    let f = fx(json!([{"id": "pan"}]), json!(null));
    let (art, mut sc) = build(&f, true);
    let px = art.render(Some(1.0), &mut sc);
    let inked = px.iter().filter(|p| p[3] > 0).count();
    assert!(inked > 20 && inked < px.len() / 2, "{inked}");
  }

  fn spec(extra: serde_json::Value) -> Fx {
    let mut v = json!({"v": 2, "scene": null, "effects": [], "glow": null});
    for (k, x) in extra.as_object().unwrap() {
      v[k] = x.clone();
    }
    let base = Base { kind: BaseKind::Solid, angle: 0.0, tile: 0.0, stops: vec![(0.0, 0xffffff)] };
    Fx::parse(&v, base).unwrap()
  }

  fn red_blue() -> serde_json::Value {
    json!({"angle": 90, "layers": [{"stops": [{"color": "#ff0000", "pos": 0}, {"color": "#0000ff", "pos": 100}]}]})
  }

  fn ink_count(px: &[Px]) -> usize {
    px.iter().filter(|p| p[3] > 0).count()
  }

  #[test]
  fn a_static_fill_paints_the_glyphs_and_is_a_still() {
    let (art, mut sc) = build(&spec(json!({"fill": red_blue()})), true);
    assert!(!art.animated());
    let px = art.render(None, &mut sc).to_vec();
    let w = art.w as usize;
    let (l, r) = (
      px.iter().enumerate().filter(|(i, p)| p[3] > 0 && i % w < w / 3),
      px.iter().enumerate().filter(|(i, p)| p[3] > 0 && i % w > 2 * w / 3),
    );
    assert!(l.map(|(_, p)| p[0] as u32).sum::<u32>() > 0 && r.map(|(_, p)| p[2] as u32).sum::<u32>() > 0);
  }

  #[test]
  fn a_fill_replaces_the_paint_slot_effect() {
    let f = spec(json!({"fill": red_blue(), "effects": [{"id": "pan"}, {"id": "coin"}]}));
    assert_eq!(f.effects.iter().map(|e| e.kind).collect::<Vec<_>>(), [Kind::Coin]);
  }

  #[test]
  fn a_moving_fill_ticks_and_motion_off_holds_the_first_frame() {
    let mv = json!({"angle": 90, "layers": [{"stops": [{"color": "#ff0000", "pos": 0}, {"color": "#0000ff", "pos": 100}],
      "repeat": true, "tile": {"unit": "name", "size": 1}, "motion": {"type": "flow"}}]});
    let f = spec(json!({"fill": mv}));
    let (art, mut sc) = build(&f, true);
    assert!(art.animated());
    let a = hash(art.render(Some(0.5), &mut sc));
    assert_ne!(a, hash(art.render(Some(1.3), &mut sc)));
    assert_eq!(a, hash(art.render(Some(0.5), &mut sc)));
    let (off, mut sc2) = build(&f, false);
    assert!(!off.animated());
    assert_eq!(hash(off.render(Some(0.5), &mut sc2)), hash(off.render(Some(1.3), &mut sc2)));
  }

  #[test]
  fn outline_and_shadows_draw_outside_the_glyphs() {
    let (plain, mut sc) = build(&spec(json!({"fill": red_blue()})), true);
    let base = ink_count(plain.render(None, &mut sc));
    let (o, mut sc) = build(&spec(json!({"fill": red_blue(), "outline": {"width": 2, "color": "#00ff00"}})), true);
    assert!(ink_count(o.render(None, &mut sc)) > base + 20);
    let sh = json!([{"x": 2, "y": 2, "blur": 0, "color": "#000000"}]);
    let (d, mut sc) = build(&spec(json!({"fill": red_blue(), "shadows": sh})), true);
    assert!(ink_count(d.render(None, &mut sc)) > base + 20);
    // shadows need a fill, and ripple wins over them
    let (nf, mut sc) = build(&spec(json!({"outline": {"width": 1, "color": "#ffffff"}, "shadows": sh})), true);
    assert!(nf.fill.is_none() && nf.stroke.is_some());
    let _ = nf.render(None, &mut sc);
    let (rp, mut sc) = build(&spec(json!({"fill": red_blue(), "shadows": sh, "effects": [{"id": "ripple"}]})), true);
    assert_eq!(ink_count(rp.render(None, &mut sc)), base);
    let (st, mut sc) = build(&spec(json!({"fill": red_blue(), "shadows": sh, "effects": [{"id": "ripple"}]})), false);
    assert!(ink_count(st.render(None, &mut sc)) > base, "static mode keeps the shadows");
  }

  // ennortix: each letter twirls on its own, staggered by 0.08s per glyph, about its own center
  #[test]
  fn twirl_turns_each_letter_alone_with_its_own_delay() {
    let f = fx(json!([{"id": "twirl", "speed": 1}]), json!(null));
    let mut text = Text::new(1.0);
    let spans = [Span { range: 0..8, ink: Ink(theme::WHITE), bold: true }];
    let w = text.layout("ennortix", &spans, &[], 1e5).full_width().ceil() as i32;
    let art = AnimArt::build(&mut text, "ennortix", w, &f, true, 0xffffff);
    assert_eq!(art.boxes.len(), 8, "one cell per glyph");
    let mut sc = Scratch::new();
    let rest = art.render(None, &mut sc).to_vec();
    // glyph 0 is at phase 0.60 (still holding), glyph 7 at 0.60 + 7*0.08/3.6 = 0.756 (mid spin)
    let px = art.render(Some(0.60 * 3.6), &mut sc).to_vec();
    let (wu, h) = (art.w as usize, art.h as usize);
    let cols = |px: &[Px], c: (usize, usize)| -> Vec<Px> {
      (0..h).flat_map(|y| (c.0..c.1).map(move |x| (y, x))).map(|(y, x)| px[y * wu + x]).collect()
    };
    let (c0, c7) = (art.boxes[0], art.boxes[7]);
    assert_eq!(cols(&px, c0), cols(&rest, c0), "glyph 0 has not started turning");
    assert_ne!(cols(&px, c7), cols(&rest, c7), "glyph 7 is mid spin");
    // each cell stays inside its own neighbourhood (a rotation about the cell's center keeps the
    // glyph within ~h/2 of it), so the last letter never lands on the first
    let right: u32 = (0..h)
      .flat_map(|y| (c7.0.saturating_sub(h)..wu).map(move |x| (y, x)))
      .map(|(y, x)| px[y * wu + x][3] as u32)
      .sum();
    let total: u32 = px.iter().map(|q| q[3] as u32).sum();
    assert!(right * 10 > total, "the name is still spread across its width: {right}/{total}");
    let left0: u32 = (0..h).flat_map(|y| (0..c0.1).map(move |x| (y, x))).map(|(y, x)| px[y * wu + x][3] as u32).sum();
    assert!(left0 > 0);
    // two glyphs at the same t differ by exactly their delay
    let ang = |i| {
      let m = motion::letter_fx(&f.effects[0], i, 0.60 * 3.6, 1.0, 8.0, 20.0).xf.0;
      m[1].atan2(m[0]).to_degrees().rem_euclid(360.0)
    };
    assert_eq!(ang(0), 0.0);
    let want = |i: f32| 360.0 * ((0.60 + i * 0.08 / 3.6 - 0.64) / 0.36).clamp(0.0, 1.0);
    for i in [2usize, 5, 7] {
      assert!((ang(i) - want(i as f32)).abs() < 0.1, "glyph {i}: {} vs {}", ang(i), want(i as f32));
    }
  }

  // the real ennortix paint (twirl + coin) through the same parse the app uses
  #[test]
  fn ennortix_as_served_splits_into_glyph_cells() {
    let v = json!({"v":2,"base":{"type":"solid","angle":0,"stops":[{"pos":0,"color":"#ffb000"}]},"glow":null,
      "scene":{"weather":{"id":"petals","tint":"#d7408c","speed":1,"density":1},"backdrop":{"id":"synth","tint":"#0ca43d","speed":1}},
      "effects":[{"id":"twirl","speed":1},{"id":"coin","speed":1}]});
    let paint = crate::cosmetics::paint::Paint::from_hs(&v).unwrap();
    let fx = paint.fx.unwrap();
    assert!(fx.split);
    let (art, mut sc) = build(&fx, true);
    assert_eq!(art.boxes.len(), 6, "mellen: one cell per letter");
    let rest = art.render(None, &mut sc).to_vec();
    let px = art.render(Some(0.60 * 3.6), &mut sc).to_vec();
    let w = art.w as usize;
    let first = art.boxes[0];
    let cell = |p: &[Px]| -> Vec<Px> {
      (0..art.h as usize).flat_map(|y| (first.0..first.1).map(move |x| (y, x))).map(|(y, x)| p[y * w + x]).collect()
    };
    // glyph 0 still holds at this instant while the last letter is already turning
    let ink = |p: &[Px]| p.iter().filter(|q| q[3] > 0 && q[0] > 200).count();
    assert!(ink(&cell(&px)) > 0 && ink(&cell(&rest)) > 0);
    assert_eq!(
      cell(&px).iter().map(|q| q[3] as u32).sum::<u32>(),
      cell(&rest).iter().map(|q| q[3] as u32).sum::<u32>()
    );
  }

  // pixel probe: a letter at the half turn is its resting self rotated 180 degrees about its own cell center
  #[test]
  fn a_letter_turns_about_its_own_center() {
    let f = fx(json!([{"id": "twirl", "speed": 1}]), json!(null));
    let mut text = Text::new(1.0);
    let spans = [Span { range: 0..2, ink: Ink(theme::WHITE), bold: true }];
    // two glyphs: the second sits well right of the origin, so a wrong pivot would throw it off the sprite
    let w = text.layout("ee", &spans, &[], 1e5).full_width().ceil() as i32;
    let art = AnimArt::build(&mut text, "ee", w, &f, true, 0xffffff);
    let mut sc = Scratch::new();
    let rest = art.render(None, &mut sc).to_vec();
    let (wu, h) = (art.w as usize, art.h as usize);
    // glyph 1 is at the half turn when 0.64 + 0.18 = its phase, i.e. t = (0.82 - 0.08/3.6) * 3.6
    let px = art.render(Some(3.6 * 0.82 - 0.08), &mut sc).to_vec();
    let (x0, x1) = art.boxes[1];
    let mut diff = 0;
    for y in 0..h {
      for x in x0..x1 {
        let mirror = rest[(h - 1 - y) * wu + (x0 + x1 - 1 - x)];
        let got = px[y * wu + x];
        diff += (0..4).map(|c| (got[c] as i32 - mirror[c] as i32).abs()).sum::<i32>();
      }
    }
    let ink: i32 = (0..h).flat_map(|y| (x0..x1).map(move |x| (y, x))).map(|(y, x)| rest[y * wu + x][3] as i32).sum();
    assert!(ink > 0 && diff * 10 < ink * 4, "diff {diff} vs ink {ink}");
  }
}
