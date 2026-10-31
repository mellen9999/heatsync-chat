// (the shader lands with the name renderer: until then only parse + tests use it)
#![allow(dead_code)]
// a 7tv name paint, compiled once to what the cpu shader needs: a 256-step color ramp for
// gradients, a still-image url for image paints, drop shadows. mirrors ext src/lib/stv-paint-css.js
// (the css it emits): repeat, stop positions, 100% 100% image stretch, name color underneath
use serde_json::Value;

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
  pub shadows: Vec<Shadow>,
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
            shadows: Vec::new(),
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
    Some(Paint { id: id.into(), fill, lut, repeat, d0, d1, shadows })
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
}
