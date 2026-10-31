// the animated half of a heatsync-native paint (`/api/paints` spec v2): the scene (backdrop +
// weather) and the effects. parsed to closed enums, so a hostile row can only pick from the
// catalog, never smuggle a value. mirrors the ext's paint-spec.js / scene-spec.js tables
use serde_json::Value;

pub const MIN_SPEED: f32 = 0.25;
pub const MAX_SPEED: f32 = 3.0;
// ext MAX_ANIMATED_LAYERS: the name's own fill, then its motion, share this many slots
const MAX_LAYERS: usize = 3;

macro_rules! catalog {
  ($name:ident { $($v:ident = $id:literal),+ $(,)? }) => {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum $name { $($v),+ }
    impl $name {
      #[cfg(test)]
      pub const ALL: &'static [$name] = &[$($name::$v),+];
      pub fn parse(s: &str) -> Option<Self> {
        match s { $($id => Some(Self::$v),)+ _ => None }
      }
      #[cfg(test)]
      pub fn id(self) -> &'static str {
        match self { $(Self::$v => $id),+ }
      }
    }
  };
}

catalog!(Backdrop {
  Dawn = "dawn", Graveyard = "graveyard", Abyss = "abyss", Nightfall = "nightfall", Terminal = "terminal",
  Furnace = "furnace", Ocean = "ocean", Skyline = "skyline", Orbit = "orbit", Synth = "synth",
  Glacier = "glacier", Sakura = "sakura", Volcano = "volcano", Alpine = "alpine", Swamp = "swamp",
  Castle = "castle", Moon = "moon", Circuit = "circuit", Eclipse = "eclipse", Arcade = "arcade",
  Bamboo = "bamboo", Giza = "giza",
});

catalog!(Weather {
  Rain = "rain", Snow = "snow", Fog = "fog", Embers = "embers", Glyphs = "glyphs", Storm = "storm",
  Petals = "petals", Bubbles = "bubbles", Fireflies = "fireflies", Leaves = "leaves", Sparks = "sparks",
  Dust = "dust", Confetti = "confetti", Meteors = "meteors", Hearts = "hearts",
});

catalog!(Kind {
  Pan = "pan", Conic = "conic", Hue = "hue", Glint = "glint", Reveal = "reveal", Stripes = "stripes",
  Stardust = "stardust", Pulse = "pulse",
  Wave = "wave", Ripple = "ripple", Coin = "coin", Heli = "heli", Float = "float", Heart = "heart",
  Wobble = "wobble", Swing = "swing", Tumble = "tumble", Neon = "neon", Glitch = "glitch",
  Jitter = "jitter", Hop = "hop", Twirl = "twirl", Type = "type", Flicker = "flicker",
});

impl Kind {
  // paint-slot effects own the fill (at most one); the rest move or glow
  pub fn paints(self) -> bool {
    use Kind::*;
    matches!(self, Pan | Conic | Hue | Glint | Reveal | Stripes | Stardust | Pulse)
  }

  // per-letter motions split the name into glyphs
  pub fn per_letter(self) -> bool {
    use Kind::*;
    matches!(self, Wave | Ripple | Tumble | Hop | Twirl | Type)
  }

  // a brightness change: the ext floors its period at 1s (flashing guard)
  pub fn luminance(self) -> bool {
    use Kind::*;
    matches!(self, Hue | Pulse | Ripple | Neon | Type | Flicker)
  }

  // seconds per cycle at speed 1
  pub fn period(self) -> f32 {
    use Kind::*;
    match self {
      Pan => 5.0,
      Conic => 6.0,
      Hue => 8.0,
      Glint => 3.4,
      Reveal => 3.0,
      Stripes => 2.4,
      Stardust => 3.0,
      Pulse => 2.4,
      Wave => 1.6,
      Ripple => 2.4,
      Coin => 5.0,
      Heli => 2.2,
      Float => 5.5,
      Heart => 1.3,
      Wobble => 2.8,
      Swing => 2.6,
      Tumble => 3.4,
      Neon => 2.6,
      Glitch => 2.8,
      Jitter => 3.0,
      Hop => 2.2,
      Twirl => 3.6,
      Type => 4.0,
      Flicker => 3.0,
    }
  }

  // two motions on the same (target, property) override each other in css: the ext refuses the pair
  fn sig(self) -> Option<&'static str> {
    use Kind::*;
    match self {
      Hue => Some("self:filter"),
      Pulse => Some("self:opacity"),
      Wave | Tumble | Hop | Twirl => Some("letter:transform"),
      Ripple => Some("letter:filter"),
      Type => Some("letter:opacity"),
      Coin | Heli | Float | Heart | Wobble | Swing | Jitter => Some("self:transform"),
      Neon | Glitch => Some("self:shadow"),
      Flicker => Some("self:opacity"),
      _ => None,
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effect {
  pub kind: Kind,
  pub speed: f32,
  // pan only
  pub scale: f32,
  pub bounce: bool,
  pub skew: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackdropSpec {
  pub id: Backdrop,
  pub tint: u32,
  pub speed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherSpec {
  pub id: Weather,
  pub tint: u32,
  pub density: u8,
  pub speed: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Scene {
  pub backdrop: Option<BackdropSpec>,
  pub weather: Option<WeatherSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseKind {
  Solid,
  Linear,
  Conic,
  Repeating,
}

// the resting fill the effects start from (stops sorted, positions 0..1, colors 0xrrggbb)
#[derive(Clone, Debug, PartialEq)]
pub struct Base {
  pub kind: BaseKind,
  pub angle: f32,
  pub tile: f32,
  pub stops: Vec<(f32, u32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fx {
  pub scene: Option<Scene>,
  pub effects: Vec<Effect>,
  pub base: Base,
  // 0xrrggbb + strength 1|2
  pub glow: Option<(u32, u8)>,
  // a motion effect that is split per glyph is present (read from the raw list, like the ext)
  pub split: bool,
}

pub fn hex6(s: &str) -> Option<u32> {
  let h = s.strip_prefix('#').filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
  u32::from_str_radix(h, 16).ok()
}

fn speed(v: &Value) -> f32 {
  v.get("speed").and_then(Value::as_f64).filter(|s| s.is_finite()).map_or(1.0, |s| s as f32).clamp(MIN_SPEED, MAX_SPEED)
}

// a layer's tint: its `tint`, else its retired `variant` read through the catalog's list, else the default
fn tint(v: &Value, legacy: [u32; 3]) -> u32 {
  if let Some(c) = v.get("tint").and_then(Value::as_str).and_then(hex6) {
    return c;
  }
  let i = v.get("variant").and_then(Value::as_u64).map_or(0, |i| i as usize);
  legacy[if i < 3 { i } else { 0 }]
}

fn scene(v: &Value) -> Option<Scene> {
  let backdrop = v.get("backdrop").filter(|b| b.is_object()).and_then(|b| {
    let id = Backdrop::parse(b.get("id")?.as_str()?)?;
    Some(BackdropSpec { id, tint: tint(b, super::fx_tints::backdrop(id)), speed: speed(b) })
  });
  let weather = v.get("weather").filter(|w| w.is_object()).and_then(|w| {
    let id = Weather::parse(w.get("id")?.as_str()?)?;
    let density = w.get("density").and_then(Value::as_u64).filter(|d| (1..=3).contains(d)).map_or(2, |d| d as u8);
    Some(WeatherSpec { id, tint: tint(w, super::fx_tints::weather(id)), density, speed: speed(w) })
  });
  (backdrop.is_some() || weather.is_some()).then_some(Scene { backdrop, weather })
}

fn effects(v: &Value) -> (Vec<Effect>, bool) {
  let mut out: Vec<Effect> = Vec::new();
  let mut split = false;
  let mut budget = MAX_LAYERS;
  let mut has_paint = false;
  let mut motion = 0;
  let mut sigs: Vec<&str> = Vec::new();
  let Some(list) = v.as_array() else { return (out, false) };
  let parsed: Vec<Effect> = list
    .iter()
    .take(8)
    .filter_map(|e| {
      let kind = Kind::parse(e.get("id")?.as_str()?)?;
      let scale = e.get("scale").and_then(Value::as_f64).filter(|s| s.is_finite()).map_or(220.0, |s| s as f32);
      Some(Effect {
        kind,
        speed: speed(e),
        scale: scale.clamp(150.0, 340.0),
        bounce: e.get("loop").and_then(Value::as_str) == Some("bounce"),
        skew: e.get("skew").and_then(Value::as_bool) == Some(true),
      })
    })
    .collect();
  // the ext spends its animation slots on the paint effect first, then the motions, in order
  for e in &parsed {
    if e.kind.paints() && !has_paint && budget > 0 && !out.iter().any(|o| o.kind == e.kind) {
      has_paint = true;
      budget -= 1;
      out.push(*e);
    }
  }
  for e in &parsed {
    if e.kind.paints() || budget == 0 || motion == 2 || out.iter().any(|o| o.kind == e.kind) {
      continue;
    }
    if let Some(s) = e.kind.sig() {
      if sigs.contains(&s) {
        continue;
      }
      sigs.push(s);
    }
    motion += 1;
    budget -= 1;
    out.push(*e);
  }
  for e in &parsed {
    split |= e.kind.per_letter();
  }
  (out, split)
}

impl Fx {
  // None = nothing animated (a plain static paint)
  pub fn parse(v: &Value, base: Base) -> Option<Fx> {
    let scene = if v.get("v").and_then(Value::as_i64) == Some(2) { v.get("scene").and_then(scene) } else { None };
    let (effects, split) = v.get("effects").map_or((Vec::new(), false), effects);
    if scene.is_none() && effects.is_empty() {
      return None;
    }
    let glow = v.get("glow").filter(|g| g.is_object()).and_then(|g| {
      let c = hex6(g.get("color")?.as_str()?)?;
      Some((c, if g.get("strength").and_then(Value::as_i64) == Some(2) { 2 } else { 1 }))
    });
    Some(Fx { scene, effects, base, glow, split })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn base() -> Base {
    Base { kind: BaseKind::Solid, angle: 0.0, tile: 0.0, stops: vec![(0.0, 0xffb000)] }
  }

  #[test]
  fn ennortix_scene_and_effects_parse() {
    let v = json!({"v": 2, "base": {}, "glow": null,
      "scene": {"weather": {"id": "petals", "tint": "#d7408c", "speed": 1, "density": 1},
                "backdrop": {"id": "synth", "tint": "#0ca43d", "speed": 1}},
      "effects": [{"id": "twirl", "speed": 1}, {"id": "coin", "speed": 1}]});
    let fx = Fx::parse(&v, base()).unwrap();
    let s = fx.scene.unwrap();
    assert_eq!(s.backdrop.unwrap().id, Backdrop::Synth);
    assert_eq!(s.backdrop.unwrap().tint, 0x0ca43d);
    assert_eq!((s.weather.unwrap().id, s.weather.unwrap().density), (Weather::Petals, 1));
    assert_eq!(fx.effects.iter().map(|e| e.kind).collect::<Vec<_>>(), [Kind::Twirl, Kind::Coin]);
    assert!(fx.split);
  }

  #[test]
  fn every_catalog_id_round_trips() {
    for b in Backdrop::ALL {
      assert_eq!(Backdrop::parse(b.id()), Some(*b));
    }
    for w in Weather::ALL {
      assert_eq!(Weather::parse(w.id()), Some(*w));
    }
    for k in Kind::ALL {
      assert_eq!(Kind::parse(k.id()), Some(*k));
    }
    assert_eq!((Backdrop::ALL.len(), Weather::ALL.len(), Kind::ALL.len()), (22, 15, 24));
  }

  #[test]
  fn every_kind_parses_alone() {
    for b in Backdrop::ALL {
      let v = json!({"v": 2, "scene": {"backdrop": {"id": b.id(), "tint": "#112233"}}, "effects": []});
      assert_eq!(Fx::parse(&v, base()).unwrap().scene.unwrap().backdrop.unwrap().id, *b);
    }
    for w in Weather::ALL {
      let v = json!({"v": 2, "scene": {"weather": {"id": w.id(), "density": 3}}, "effects": []});
      assert_eq!(Fx::parse(&v, base()).unwrap().scene.unwrap().weather.unwrap().id, *w);
    }
    for k in Kind::ALL {
      let v = json!({"v": 2, "scene": null, "effects": [{"id": k.id(), "speed": 2}]});
      let fx = Fx::parse(&v, base()).unwrap();
      assert_eq!((fx.effects[0].kind, fx.effects[0].speed), (*k, 2.0));
    }
  }

  #[test]
  fn hostile_specs_clamp_or_drop() {
    let v = json!({"v": 2, "scene": {"backdrop": {"id": "../etc", "tint": "red"}, "weather": {"id": 7}},
      "effects": [{"id": "nope"}, 5, {"id": "pan", "speed": 1e9, "scale": -4, "loop": "x"}]});
    let fx = Fx::parse(&v, base()).unwrap();
    assert!(fx.scene.is_none());
    assert_eq!((fx.effects[0].speed, fx.effects[0].scale, fx.effects[0].bounce), (MAX_SPEED, 150.0, false));
    // v1 specs carry no scene; nothing animated at all is not an Fx
    assert!(Fx::parse(&json!({"v": 1, "scene": {"backdrop": {"id": "dawn"}}, "effects": []}), base()).is_none());
    assert!(Fx::parse(&json!({"v": 2, "effects": "x"}), base()).is_none());
  }

  #[test]
  fn legacy_variant_and_default_tint() {
    let v =
      json!({"v": 2, "scene": {"backdrop": {"id": "dawn", "variant": 1}, "weather": {"id": "rain", "variant": 99}}});
    let s = Fx::parse(&v, base()).unwrap().scene.unwrap();
    assert_eq!(s.backdrop.unwrap().tint, 0xff5f87);
    assert_eq!(s.weather.unwrap().tint, 0x9db4c9);
  }

  #[test]
  fn slots_and_conflicts_follow_the_ext() {
    // one paint effect, two motions, distinct targets; a third animated layer is the cap
    let v =
      json!({"v": 2, "effects": [{"id": "pan"}, {"id": "glint"}, {"id": "coin"}, {"id": "heli"}, {"id": "neon"}]});
    let kinds: Vec<Kind> = Fx::parse(&v, base()).unwrap().effects.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [Kind::Pan, Kind::Coin, Kind::Neon]);
  }
}
