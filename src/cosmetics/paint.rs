// a 7tv name paint, compiled once to what the cpu shader needs: a 256-step color ramp for
// gradients, a still-image url for image paints, drop shadows. mirrors ext src/lib/stv-paint-css.js
// (the css it emits): repeat, stop positions, 100% 100% image stretch, name color underneath
use serde_json::Value;

use super::fx::{Base, BaseKind, Fx};
use crate::images::url::allowed;

const MAX_STOPS: usize = 64;
const MAX_SHADOWS: usize = 10;
const LUT: usize = 256;

// 0xAARRGGBB
pub type Argb = u32;

#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
  pub dx: f32,
  pub dy: f32,
  pub blur: f32,
  pub color: Argb,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
  // only shadows: the name keeps its own color
  None,
  Solid(Argb),
  Linear { deg: f32 },
  Radial { ellipse: bool },
  // first frame of the paint image, stretched over the name
  Image { still: Box<str> },
}

#[derive(Debug, PartialEq)]
pub struct Paint {
  pub id: Box<str>,
  pub fill: Fill,
  // gradient ramp over [d0, d1]; empty for the other fills
  lut: Box<[Argb]>,
  repeat: bool,
  d0: f32,
  d1: f32,
  // heatsync banded paints: one tile's width in px along the gradient (0 = the name box)
  tile: f32,
  pub shadows: Vec<Shadow>,
  // heatsync-native scene + effects (None for 7tv paints and plain heatsync ones)
  pub fx: Option<Box<Fx>>,
}

// 7tv packs rgba into one (signed) int: r<<24 | g<<16 | b<<8 | a
pub fn argb(n: i64) -> Argb {
  (n as u32).rotate_right(8)
}

fn straight(c: Argb) -> [f32; 4] {
  [(c >> 24) as f32, ((c >> 16) & 255) as f32, ((c >> 8) & 255) as f32, (c & 255) as f32]
}

// premultiplied mix: a translucent stop fades toward the next one without darkening
fn mix(a: Argb, b: Argb, f: f32) -> Argb {
  let (a, b) = (straight(a), straight(b));
  let pm = |c: [f32; 4]| [c[0], c[1] * c[0] / 255.0, c[2] * c[0] / 255.0, c[3] * c[0] / 255.0];
  let (pa, pb) = (pm(a), pm(b));
  let m: Vec<f32> = (0..4).map(|i| pa[i] + (pb[i] - pa[i]) * f).collect();
  let al = m[0];
  let un = |v: f32| if al <= 0.0 { 0 } else { (v * 255.0 / al).round().clamp(0.0, 255.0) as u32 };
  (al.round().clamp(0.0, 255.0) as u32) << 24 | un(m[1]) << 16 | un(m[2]) << 8 | un(m[3])
}

fn ramp(stops: &[(f32, Argb)], t: f32) -> Argb {
  let (first, last) = (stops[0], stops[stops.len() - 1]);
  if t <= first.0 {
    return first.1;
  }
  if t >= last.0 {
    return last.1;
  }
  for w in stops.windows(2) {
    if t < w[1].0 {
      let span = w[1].0 - w[0].0;
      return if span <= 0.0 { w[1].1 } else { mix(w[0].1, w[1].1, (t - w[0].0) / span) };
    }
  }
  last.1
}

fn num(v: &Value, k: &str) -> f32 {
  v.get(k).and_then(Value::as_f64).filter(|f| f.is_finite()).unwrap_or(0.0) as f32
}

// only 7tv's own paint images ever reach the network (ext STV_PAINT_IMG)
fn paint_image(url: &str) -> Option<&str> {
  let path = url.strip_prefix("https://cdn.7tv.app/paint/")?;
  let ok = !path.is_empty()
    && path.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'/' | b'.' | b'-'))
    && !path.contains("..");
  (ok && allowed(url)).then_some(url)
}

// "#rrggbb" -> 0xrrggbb
fn hex6(s: &str) -> Option<u32> {
  let h = s.strip_prefix('#').filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
  u32::from_str_radix(h, 16).ok()
}

fn still_of(url: &str) -> Box<str> {
  match url.strip_suffix("/1x.webp") {
    Some(base) => format!("{base}/1x_static.webp").into(),
    None => url.into(),
  }
}

impl Paint {
  // None = nothing to draw (the name stays as it is)
  pub fn parse(v: &Value) -> Option<Paint> {
    let id =
      v.get("id").and_then(Value::as_str).filter(|i| i.len() <= 40 && i.bytes().all(|b| b.is_ascii_alphanumeric()))?;
    let func = v.get("function").and_then(Value::as_str)?.to_ascii_lowercase().replace('_', "-");
    let mut stops: Vec<(f32, Argb)> = Vec::new();
    for s in v.get("stops").and_then(Value::as_array).into_iter().flatten().take(MAX_STOPS) {
      let at = num(s, "at");
      let c = s.get("color").and_then(Value::as_i64)?;
      // css wants stops in order: a stop behind its predecessor is pulled up to it
      stops.push((stops.last().map_or(at, |p| at.max(p.0)), argb(c)));
    }
    let mut repeat = v.get("repeat").and_then(Value::as_bool).unwrap_or(false);
    let (mut d0, mut d1) = (0.0, 1.0);
    if repeat {
      match (stops.first(), stops.last()) {
        (Some(a), Some(b)) if b.0 > a.0 => (d0, d1) = (a.0, b.0),
        _ => repeat = false,
      }
    }
    let ramped =
      || -> Box<[Argb]> { (0..LUT).map(|i| ramp(&stops, d0 + (d1 - d0) * i as f32 / (LUT - 1) as f32)).collect() };
    let (fill, lut) = match func.as_str() {
      "linear-gradient" if !stops.is_empty() => (Fill::Linear { deg: num(v, "angle") }, ramped()),
      "radial-gradient" if !stops.is_empty() => {
        (Fill::Radial { ellipse: v.get("shape").and_then(Value::as_str) == Some("ellipse") }, ramped())
      }
      "url" => match v.get("image_url").and_then(Value::as_str).and_then(paint_image) {
        Some(u) => (Fill::Image { still: still_of(u) }, Box::default()),
        None => (Fill::None, Box::default()),
      },
      "linear-gradient" | "radial-gradient" => (Fill::None, Box::default()),
      // anything else is a flat color
      _ => match v.get("color").and_then(Value::as_i64) {
        Some(c) => {
          return Some(Paint {
            id: id.into(),
            fill: Fill::Solid(argb(c)),
            lut: Box::default(),
            repeat: false,
            d0,
            d1,
            tile: 0.0,
            shadows: Vec::new(),
            fx: None,
          });
        }
        None => (Fill::None, Box::default()),
      },
    };
    let shadows: Vec<Shadow> = v
      .get("shadows")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .take(MAX_SHADOWS)
      .map(|s| Shadow {
        dx: num(s, "x_offset").clamp(-32.0, 32.0),
        dy: num(s, "y_offset").clamp(-32.0, 32.0),
        blur: num(s, "radius").clamp(0.0, 32.0),
        color: argb(s.get("color").and_then(Value::as_i64).unwrap_or(0)),
      })
      .collect();
    if fill == Fill::None && shadows.is_empty() {
      return None;
    }
    Some(Paint { id: id.into(), fill, lut, repeat, d0, d1, tile: 0.0, shadows, fx: None })
  }

  // a heatsync-native paint (`/api/paints` spec v2): the base fill plus the glow at rest (the ext's
  // `static` mode), and in `fx` the scene + effects that animate it. None = nothing to draw. the id
  // names the spec, so an edit re-renders
  pub fn from_hs(v: &Value) -> Option<Paint> {
    let base = v.get("base")?;
    let mut stops: Vec<(f32, Argb)> = base
      .get("stops")?
      .as_array()?
      .iter()
      .take(MAX_STOPS)
      .filter_map(|s| {
        let pos = s.get("pos").and_then(Value::as_f64).filter(|p| p.is_finite())?.clamp(0.0, 100.0) as f32;
        Some((pos / 100.0, 0xff00_0000 | hex6(s.get("color")?.as_str()?)?))
      })
      .collect();
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    let first = *stops.first()?;
    let kind = base.get("type").and_then(Value::as_str).unwrap_or("solid");
    let angle = num(base, "angle").rem_euclid(360.0);
    let (mut d0, mut d1, mut repeat, mut tile) = (0.0, 1.0, false, 0.0);
    let (fill, lut): (Fill, Box<[Argb]>) = match kind {
      "solid" => (Fill::Solid(first.1), Box::default()),
      _ if stops.len() == 1 => (Fill::Solid(first.1), Box::default()),
      "repeating-linear" => {
        // hard bands: each color holds until the next stop; the last runs to the tile edge
        tile = num(base, "tileWidth").clamp(2.0, 40.0);
        (d0, d1, repeat) = (0.0, 1.0, true);
        let band = |t: f32| stops.iter().rev().find(|s| s.0 <= t).map_or(first.1, |s| s.1);
        (Fill::Linear { deg: angle }, (0..LUT).map(|i| band(i as f32 / LUT as f32)).collect())
      }
      _ => {
        (d0, d1) = (0.0, 1.0);
        (Fill::Linear { deg: angle }, (0..LUT).map(|i| ramp(&stops, i as f32 / (LUT - 1) as f32)).collect())
      }
    };
    let mut shadows = Vec::new();
    if let Some(c) = v.get("glow").and_then(|g| g.get("color")).and_then(Value::as_str).and_then(hex6) {
      let [r1, r2] = if v["glow"]["strength"].as_i64() == Some(2) { [10.0, 26.0] } else { [6.0, 14.0] };
      for (blur, a) in [(r1, 0xcc_u32), (r2, 0x66)] {
        shadows.push(Shadow { dx: 0.0, dy: 0.0, blur, color: a << 24 | c });
      }
    }
    // fnv-1a over the canonical json (serde sorts keys): same spec, same id
    let h =
      v.to_string().bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    let base_kind = match kind {
      "solid" => BaseKind::Solid,
      "conic" => BaseKind::Conic,
      "repeating-linear" => BaseKind::Repeating,
      _ => BaseKind::Linear,
    };
    let fx = Fx::parse(
      v,
      Base { kind: base_kind, angle, tile, stops: stops.iter().map(|&(p, c)| (p, c & 0xff_ffff)).collect() },
    )
    .map(Box::new);
    Some(Paint { id: format!("hs{h:016x}").into(), fill, lut, repeat, d0, d1, tile, shadows, fx })
  }

  fn at(&self, t: f32) -> Argb {
    let u = (t - self.d0) / (self.d1 - self.d0);
    let u = if self.repeat { u - u.floor() } else { u.clamp(0.0, 1.0) };
    self.lut[(u * (LUT - 1) as f32).round() as usize]
  }

  // color of the pixel (x, y) in a w×h name box; None when the fill isn't a gradient
  pub fn gradient(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Argb> {
    let (cx, cy) = (x - w / 2.0, y - h / 2.0);
    match self.fill {
      Fill::Solid(c) => Some(c),
      Fill::Linear { deg } => {
        let (s, c) = deg.to_radians().sin_cos();
        let len = (w * s).abs() + (h * c).abs();
        if self.tile > 0.0 {
          let u = ((cx * s - cy * c) / self.tile).rem_euclid(1.0);
          return Some(self.lut[(u * LUT as f32) as usize % LUT]);
        }
        Some(self.at(if len > 0.0 { (cx * s - cy * c) / len + 0.5 } else { 0.5 }))
      }
      Fill::Radial { ellipse } => {
        // css farthest-corner: a circle reaches the far corner, an ellipse keeps the box's aspect
        let t = if ellipse {
          let (rx, ry) = (w / 2.0 * std::f32::consts::SQRT_2, h / 2.0 * std::f32::consts::SQRT_2);
          ((cx / rx).powi(2) + (cy / ry).powi(2)).sqrt()
        } else {
          let r = ((w / 2.0).powi(2) + (h / 2.0).powi(2)).sqrt();
          if r > 0.0 { (cx * cx + cy * cy).sqrt() / r } else { 0.0 }
        };
        Some(self.at(t))
      }
      Fill::Image { .. } | Fill::None => None,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  // red then blue as 7tv ints (signed, rgba)
  const RED: i64 = 0xff0000ffu32 as i32 as i64;
  const BLUE: i64 = 0x0000ffffu32 as i32 as i64;

  fn linear(extra: Value) -> Paint {
    let mut v = json!({"id": "P1", "function": "LINEAR_GRADIENT", "angle": 90, "stops": [{"at": 0, "color": RED}, {"at": 1, "color": BLUE}]});
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    Paint::parse(&v).unwrap()
  }

  #[test]
  fn colors_unpack() {
    assert_eq!(argb(RED), 0xffff0000);
    assert_eq!(argb(0x00ff0080), 0x8000ff00);
    assert_eq!(argb(-1), 0xffffffff);
  }

  #[test]
  fn linear_runs_left_to_right_at_90deg() {
    let p = linear(json!({}));
    assert_eq!(p.gradient(0.0, 5.0, 100.0, 10.0), Some(0xffff0000));
    assert_eq!(p.gradient(99.9, 5.0, 100.0, 10.0).unwrap() & 0xffffff, 0x0000ff);
    let mid = p.gradient(50.0, 5.0, 100.0, 10.0).unwrap();
    assert!((mid >> 16 & 255).abs_diff(128) < 4 && (mid & 255).abs_diff(128) < 4);
  }

  #[test]
  fn repeat_tiles_and_hard_stops_stay_hard() {
    let p = linear(
      json!({"repeat": true, "stops": [{"at": 0, "color": RED}, {"at": 0.5, "color": RED}, {"at": 0.5, "color": BLUE}, {"at": 1, "color": BLUE}]}),
    );
    // repeat over the stop span [0,1] == one tile: sampling stays two flat halves
    assert_eq!(p.gradient(10.0, 5.0, 100.0, 10.0).unwrap() & 0xffffff, 0xff0000);
    assert_eq!(p.gradient(90.0, 5.0, 100.0, 10.0).unwrap() & 0xffffff, 0x0000ff);
    let tiled = linear(json!({"repeat": true, "stops": [{"at": 0, "color": RED}, {"at": 0.25, "color": BLUE}]}));
    // t=0.3 wraps to 0.05 of the 0..0.25 tile: mostly red again
    assert!(tiled.gradient(30.0, 5.0, 100.0, 10.0).unwrap() >> 16 & 255 > 150);
  }

  #[test]
  fn radial_center_and_corner() {
    let p = Paint::parse(&json!({"id": "R", "function": "radial-gradient", "shape": "circle",
      "stops": [{"at": 0, "color": RED}, {"at": 1, "color": BLUE}]}))
    .unwrap();
    assert_eq!(p.gradient(50.0, 5.0, 100.0, 10.0).unwrap() & 0xffffff, 0xff0000);
    assert_eq!(p.gradient(0.0, 0.0, 100.0, 10.0).unwrap() & 0xffffff, 0x0000ff);
  }

  #[test]
  fn translucent_stop_keeps_its_hue() {
    // 50% red -> 50% red must stay red, never darken
    let half = 0xff000080u32 as i32 as i64;
    let p = Paint::parse(&json!({"id": "T", "function": "linear-gradient", "angle": 0,
      "stops": [{"at": 0, "color": half}, {"at": 1, "color": half}]}))
    .unwrap();
    assert_eq!(p.gradient(1.0, 1.0, 10.0, 10.0), Some(0x80ff0000));
  }

  #[test]
  fn image_paints_only_from_the_paint_cdn() {
    let ok = Paint::parse(
      &json!({"id": "I", "function": "url", "image_url": "https://cdn.7tv.app/paint/01ABC/layer/9/1x.webp"}),
    )
    .unwrap();
    assert_eq!(ok.fill, Fill::Image { still: "https://cdn.7tv.app/paint/01ABC/layer/9/1x_static.webp".into() });
    for bad in [
      "https://evil.example/paint/x.webp",
      "https://cdn.7tv.app/emote/x.webp",
      "http://cdn.7tv.app/paint/x",
      "https://cdn.7tv.app/paint/../x",
      "https://cdn.7tv.app/paint/a b",
      "https://cdn.7tv.app.evil.com/paint/x",
    ] {
      assert!(Paint::parse(&json!({"id": "I", "function": "url", "image_url": bad})).is_none(), "{bad}");
    }
  }

  #[test]
  fn shadows_clamped_and_capped_and_empty_paint_is_nothing() {
    let sh: Vec<Value> =
      (0..30).map(|_| json!({"x_offset": 1e9, "y_offset": -1e9, "radius": 1e9, "color": 255})).collect();
    let p = linear(json!({"shadows": sh}));
    assert_eq!(p.shadows.len(), MAX_SHADOWS);
    assert_eq!((p.shadows[0].dx, p.shadows[0].dy, p.shadows[0].blur), (32.0, -32.0, 32.0));
    assert!(Paint::parse(&json!({"id": "E", "function": "linear-gradient", "stops": []})).is_none());
    assert!(Paint::parse(&json!({"id": "bad id", "function": "url"})).is_none());
    assert!(Paint::parse(&json!("junk")).is_none());
  }

  // ennortix's real `/api/paints` row (trimmed): a heatsync-native plus paint, not a 7tv one
  fn ennortix() -> Value {
    json!({"v": 2, "base": {"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]}, "glow": null,
      "scene": {"weather": {"id": "petals", "tint": "#d7408c", "speed": 1, "density": 1}, "backdrop": {"id": "synth", "tint": "#0ca43d", "speed": 1}},
      "effects": [{"id": "twirl", "speed": 1}, {"id": "coin", "speed": 1}]})
  }

  #[test]
  fn heatsync_native_paint_renders_its_resting_base() {
    let p = Paint::from_hs(&ennortix()).unwrap();
    assert_eq!(p.fill, Fill::Solid(0xffffb000));
    assert!(p.shadows.is_empty());
    assert_eq!(p.gradient(3.0, 3.0, 40.0, 10.0), Some(0xffffb000));
    // same spec, same id; an edit gets a new one
    assert_eq!(p.id, Paint::from_hs(&ennortix()).unwrap().id);
    let mut edited = ennortix();
    edited["base"]["stops"][0]["color"] = json!("#00ff00");
    assert_ne!(p.id, Paint::from_hs(&edited).unwrap().id);
  }

  #[test]
  fn heatsync_native_gradients_glow_and_bands() {
    let lin = Paint::from_hs(&json!({"v": 2, "base": {"type": "linear", "angle": 90,
      "stops": [{"pos": 100, "color": "#0000ff"}, {"pos": 0, "color": "#ff0000"}]}, "glow": {"color": "#00ff00", "strength": 2}}))
    .unwrap();
    assert_eq!(lin.gradient(0.0, 5.0, 100.0, 10.0), Some(0xffff0000));
    assert_eq!(lin.gradient(99.9, 5.0, 100.0, 10.0).unwrap() & 0xffffff, 0x0000ff);
    assert_eq!(
      lin.shadows.iter().map(|s| (s.blur, s.color)).collect::<Vec<_>>(),
      vec![(10.0, 0xcc00ff00), (26.0, 0x6600ff00)]
    );
    let conic = Paint::from_hs(&json!({"base": {"type": "conic", "angle": 0,
      "stops": [{"pos": 0, "color": "#ff0000"}, {"pos": 100, "color": "#0000ff"}]}}))
    .unwrap();
    assert!(matches!(conic.fill, Fill::Linear { .. }));
    // 10px tile, red for the first half then blue, repeating along x
    let bands = Paint::from_hs(&json!({"base": {"type": "repeating-linear", "angle": 90, "tileWidth": 10,
      "stops": [{"pos": 0, "color": "#ff0000"}, {"pos": 50, "color": "#0000ff"}]}}))
    .unwrap();
    let at = |x: f32| bands.gradient(x, 5.0, 100.0, 10.0).unwrap() & 0xffffff;
    assert_eq!((at(52.0), at(57.0), at(62.0)), (0xff0000, 0x0000ff, 0xff0000));
  }

  #[test]
  fn hostile_heatsync_paints_are_dropped_or_safe() {
    for bad in [
      json!(null),
      json!("x"),
      json!({}),
      json!({"base": {"stops": []}}),
      json!({"base": {"stops": [{"pos": 0, "color": "red"}]}}),
    ] {
      assert!(Paint::from_hs(&bad).is_none(), "{bad}");
    }
    let wild = Paint::from_hs(&json!({"base": {"type": "repeating-linear", "angle": 1e300, "tileWidth": -5,
      "stops": [{"pos": 1e9, "color": "#112233"}, {"pos": -1e9, "color": "#445566"}]}, "glow": {"color": "nope"}}))
    .unwrap();
    assert!(wild.shadows.is_empty());
    assert!(wild.gradient(1.0, 1.0, 10.0, 10.0).is_some());
  }
}
