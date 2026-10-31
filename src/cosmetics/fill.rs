// the `fill`, `outline` and `shadows` blocks of a heatsync paint (spec v1/v2, additive): up to four
// gradient layers with their own motion, a text stroke, and a drop-shadow chain. parsed to bounded
// plain data with the ext's re-clamps (paint-spec.js fillDrawLayers / outlineCss / shadowsOf), so a
// hostile row can only pick values from the allowed ranges
use super::fx::{MAX_SPEED, MIN_SPEED};
use serde_json::Value;

const MAX_LAYERS: usize = 4;
const MAX_STOPS: usize = 28;
const MAX_DROPS: usize = 10;
// the ext's default for a fill that has nothing drawable
const FALLBACK: [u8; 4] = [0xe4, 0xe4, 0xe4, 0xff];

pub type Rgba = [u8; 4];

// `#rrggbb` or `#rrggbbaa`
pub fn rgba(s: &str) -> Option<Rgba> {
  let h = s.strip_prefix('#')?;
  if !matches!(h.len(), 6 | 8) || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
    return None;
  }
  let v = u32::from_str_radix(h, 16).ok()?;
  let [a, b, c, d] = v.to_be_bytes();
  Some(if h.len() == 6 { [b, c, d, 255] } else { [a, b, c, d] })
}

fn num(v: &Value, k: &str) -> Option<f32> {
  v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite()).map(|n| n as f32)
}

fn speed(v: &Value) -> f32 {
  num(v, "speed").unwrap_or(1.0).clamp(MIN_SPEED, MAX_SPEED)
}

// round to `step`, then clamp (the ext's safeFill* / clampStep)
fn snap(n: f32, lo: f32, hi: f32, step: f32) -> f32 {
  ((n / step).round() * step).clamp(lo, hi)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
  // 0..1 along the gradient
  pub pos: f32,
  pub color: Rgba,
  // where the colors meet, 0..1 of the way to the next stop (None = the even middle)
  pub hint: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
  Linear,
  Radial { circle: bool },
  Conic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tile {
  // css px along the gradient line (radial: a dot tile)
  Px(f32),
  // multiples of the gradient line / radius
  Name(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
  pub spin: bool,
  pub speed: f32,
  pub reverse: bool,
  pub bounce: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
  pub shape: Shape,
  // added to the fill's angle
  pub tilt: f32,
  pub stops: Vec<Stop>,
  pub tile: Option<Tile>,
  // radial / conic center, 0..1 of the name box
  pub cx: f32,
  pub cy: f32,
  pub motion: Option<Motion>,
  // a 7tv picture layer: drawn as its fallback stops (the ext's own fallback for a picture it cannot show)
  pub image: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
  pub angle: f32,
  pub layers: Vec<Layer>,
  // hue-rotate speed
  pub hue: Option<f32>,
  // (speed, depth)
  pub breathe: Option<(f32, f32)>,
  // the chatter's own color under every layer
  pub underlay: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline {
  pub width: f32,
  pub color: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drop {
  pub x: f32,
  pub y: f32,
  pub blur: f32,
  pub color: Rgba,
}

fn stops(v: &Value) -> Vec<Stop> {
  let mut out: Vec<Stop> = v
    .as_array()
    .map(|a| a.as_slice())
    .unwrap_or_default()
    .iter()
    .filter_map(|s| {
      let color = rgba(s.get("color")?.as_str()?)?;
      let pos = num(s, "pos").filter(|p| (0.0..=100.0).contains(p))?;
      let hint =
        num(s, "hint").filter(|h| (0.0..=100.0).contains(h) && *h != 50.0).map(|h| snap(h, 0.0, 100.0, 0.1) / 100.0);
      Some(Stop { pos: snap(pos, 0.0, 100.0, 0.1) / 100.0, color, hint })
    })
    .take(MAX_STOPS)
    .collect();
  out.sort_by(|a, b| a.pos.total_cmp(&b.pos));
  if out.is_empty() {
    out = vec![Stop { pos: 0.0, color: FALLBACK, hint: None }, Stop { pos: 1.0, color: FALLBACK, hint: None }];
  }
  out
}

fn layer(v: &Value) -> Option<Layer> {
  if !v.is_object() || v.get("legacy").and_then(Value::as_bool) == Some(true) {
    return None;
  }
  let kind = v.get("kind").and_then(Value::as_str).unwrap_or("linear");
  let image = kind == "image";
  let center = v.get("center").filter(|c| c.is_object());
  let c = |k: &str| {
    center
      .and_then(|c| num(c, k))
      .filter(|n| (0.0..=100.0).contains(n))
      .map_or(0.5, |n| snap(n, 0.0, 100.0, 0.1) / 100.0)
  };
  let shape = match kind {
    "radial" => Shape::Radial { circle: v.get("shape").and_then(Value::as_str) == Some("circle") },
    "conic" => Shape::Conic,
    _ => Shape::Linear,
  };
  let tile = (v.get("repeat").and_then(Value::as_bool) == Some(true)).then(|| v.get("tile")).flatten().and_then(|t| {
    let size = num(t, "size")?;
    match t.get("unit").and_then(Value::as_str)? {
      "px" if (1.0..=200.0).contains(&size) => Some(Tile::Px(size.round().max(1.0))),
      "name" if (0.02..=10.0).contains(&size) => Some(Tile::Name(size)),
      _ => None,
    }
  });
  let motion = if image {
    None
  } else {
    v.get("motion").filter(|m| m.is_object()).and_then(|m| {
      let spin = match m.get("type").and_then(Value::as_str)? {
        "flow" => false,
        "spin" => true,
        _ => return None,
      };
      Some(Motion {
        spin,
        speed: speed(m),
        reverse: m.get("reverse").and_then(Value::as_bool) == Some(true),
        bounce: m.get("loop").and_then(Value::as_str) == Some("bounce"),
      })
    })
  };
  Some(Layer {
    shape,
    tilt: snap(num(v, "tilt").unwrap_or(0.0), -180.0, 180.0, 0.5),
    stops: stops(v.get("stops").unwrap_or(&Value::Null)),
    tile,
    cx: c("x"),
    cy: c("y"),
    motion,
    image,
  })
}

impl Fill {
  pub fn parse(v: &Value, underlay: bool) -> Fill {
    let mut layers: Vec<Layer> = v
      .get("layers")
      .and_then(Value::as_array)
      .map(|a| a.iter().filter_map(layer).take(MAX_LAYERS).collect())
      .unwrap_or_default();
    if layers.is_empty() {
      layers.push(Layer {
        shape: Shape::Linear,
        tilt: 0.0,
        stops: stops(&Value::Null),
        tile: None,
        cx: 0.5,
        cy: 0.5,
        motion: None,
        image: false,
      });
    }
    let modulator = |k: &str| v.get(k).filter(|m| m.is_object());
    Fill {
      angle: snap(num(v, "angle").unwrap_or(0.0), f32::MIN, f32::MAX, 0.5).rem_euclid(360.0),
      layers,
      hue: modulator("hue").map(speed),
      breathe: modulator("breathe")
        .map(|b| (speed(b), num(b, "depth").filter(|d| (0.1..=0.9).contains(d)).unwrap_or(0.5))),
      underlay,
    }
  }

  // anything in the fill moves
  #[cfg(test)]
  pub fn moves(&self) -> bool {
    self.hue.is_some() || self.breathe.is_some() || self.layers.iter().any(|l| l.motion.is_some())
  }
}

impl Outline {
  pub fn parse(v: &Value) -> Option<Outline> {
    if !v.is_object() {
      return None;
    }
    let color = rgba(v.get("color")?.as_str()?)?;
    let width = snap(num(v, "width")?, f32::MIN, f32::MAX, 0.25);
    (0.25..=2.0).contains(&width).then_some(Outline { width, color })
  }
}

// at most ten, each re-clamped; ones with a bad color or position are dropped
pub fn drops(v: &Value) -> Vec<Drop> {
  let one = |s: &Value| {
    let g = |k: &str, lo: f32, hi: f32| num(s, k).map(|n| (snap(n, lo, hi, 0.1) * 10.0).round() / 10.0);
    Some(Drop {
      x: g("x", -60.0, 60.0)?,
      y: g("y", -60.0, 60.0)?,
      blur: g("blur", 0.0, 8.0)?,
      color: rgba(s.get("color")?.as_str()?)?,
    })
  };
  v.as_array().map(|a| a.iter().take(MAX_DROPS).filter_map(one).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn colors_take_six_or_eight_digits() {
    assert_eq!(rgba("#ff8000"), Some([255, 128, 0, 255]));
    assert_eq!(rgba("#ff800040"), Some([255, 128, 0, 64]));
    for bad in ["ff8000", "#ff80", "#gg0000", "#ff80004", "red", ""] {
      assert_eq!(rgba(bad), None, "{bad}");
    }
  }

  #[test]
  fn every_layer_kind_and_motion_parses() {
    let f = Fill::parse(
      &json!({"angle": 450.2, "layers": [
        {"kind": "linear", "tilt": 999, "stops": [{"color": "#ff0000", "pos": 100}, {"color": "#0000ff80", "pos": 0, "hint": 30}],
         "repeat": true, "tile": {"unit": "px", "size": 7.4}, "motion": {"type": "flow", "speed": 99, "reverse": true, "loop": "bounce"}},
        {"kind": "radial", "shape": "circle", "center": {"x": 20, "y": 120}, "stops": [{"color": "#00ff00", "pos": 0}],
         "repeat": true, "tile": {"unit": "name", "size": 0.5}},
        {"kind": "conic", "stops": [{"color": "#ffffff", "pos": 10}], "motion": {"type": "spin", "speed": 2}},
        {"kind": "image", "src": "https://cdn.7tv.app/paint/x/1x.webp", "anim": true, "stops": [{"color": "#123456", "pos": 0}],
         "motion": {"type": "spin"}},
        {"kind": "linear", "stops": []}]}),
      true,
    );
    assert_eq!(f.layers.len(), 4, "capped at four");
    assert!((f.angle - 90.0).abs() < 0.01);
    let l = &f.layers[0];
    assert_eq!((l.tilt, l.tile, l.stops[0].pos, l.stops[0].hint), (180.0, Some(Tile::Px(7.0)), 0.0, Some(0.3)));
    assert_eq!(l.stops[0].color, [0, 0, 255, 128]);
    let m = l.motion.unwrap();
    assert_eq!((m.spin, m.speed, m.reverse, m.bounce), (false, MAX_SPEED, true, true));
    assert_eq!(f.layers[1].shape, Shape::Radial { circle: true });
    assert_eq!((f.layers[1].cx, f.layers[1].tile), (0.2, Some(Tile::Name(0.5))));
    assert_eq!(f.layers[1].cy, 0.5, "out of range center falls back");
    assert!(f.layers[2].motion.unwrap().spin);
    assert!(f.layers[3].image && f.layers[3].motion.is_none(), "a picture never moves");
    assert!(f.underlay && f.moves());
  }

  #[test]
  fn hostile_fills_stay_bounded() {
    let f = Fill::parse(
      &json!({"angle": "x", "layers": [5, null, {"legacy": true, "stops": []}, {"kind": "bogus", "tile": 3,
      "repeat": true, "stops": [{"color": "nope", "pos": 5}, {"color": "#000", "pos": 500}]}], "hue": 7, "breathe": {"depth": 9}}),
      false,
    );
    assert_eq!((f.angle, f.layers.len()), (0.0, 1));
    assert_eq!((f.layers[0].shape, f.layers[0].tile), (Shape::Linear, None));
    assert_eq!(f.layers[0].stops.len(), 2, "no usable stop draws the ext's fallback");
    assert_eq!((f.hue, f.breathe), (None, Some((1.0, 0.5))));
    assert!(!Fill::parse(&json!({}), false).moves());
    let many: Vec<Value> = (0..99).map(|i| json!({"color": "#ffffff", "pos": i as f64 % 100.0})).collect();
    assert_eq!(Fill::parse(&json!({"layers": [{"stops": many}]}), false).layers[0].stops.len(), MAX_STOPS);
  }

  #[test]
  fn outline_and_shadows_reclamp() {
    assert_eq!(
      Outline::parse(&json!({"width": 1.1, "color": "#112233"})),
      Some(Outline { width: 1.0, color: [17, 34, 51, 255] })
    );
    for bad in [
      json!({"width": 5, "color": "#112233"}),
      json!({"width": 1, "color": "x"}),
      json!({"width": 0.01, "color": "#112233"}),
      json!(3),
    ] {
      assert_eq!(Outline::parse(&bad), None, "{bad}");
    }
    let d = drops(
      &json!([{"x": 99, "y": -0.23, "blur": 40, "color": "#ff000080"}, {"x": 1, "y": 1, "blur": 1, "color": "bad"}, 4, {"x": 1}]),
    );
    assert_eq!(d, [Drop { x: 60.0, y: -0.2, blur: 8.0, color: [255, 0, 0, 128] }]);
    assert_eq!(
      drops(&json!((0..40).map(|_| json!({"x": 0, "y": 0, "blur": 0, "color": "#000000"})).collect::<Vec<_>>())).len(),
      MAX_DROPS
    );
  }
}
