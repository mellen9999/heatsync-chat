// deterministic synthetic chat for the render gate and flood testing
use crate::model::message::{Kind, Message, Platform};
use crate::ui::theme;

const NICKS: &[&str] =
  &["xqcow", "pokimane", "forsen", "heatsync", "a_very_long_username_25", "nymn", "tyler1", "sodapoppin"];
const COLORS: &[u32] =
  &[theme::RED, theme::GREEN, theme::YELLOW, theme::BLUE, theme::MAGENTA, theme::CYAN, theme::HEATSYNC];
const BODIES: &[&str] = &[
  "LULW",
  "that was actually insane",
  "OMEGALUL OMEGALUL OMEGALUL clip it",
  "first time here, what game is this",
  "日本語のチャットも大丈夫かな 🔥",
  "مرحبا بالجميع في البث",
  "emoji test 😂😂😂 👍🏽 🇯🇵",
  "a much longer message that should wrap across multiple lines when the chat pane is narrow enough, which it usually is in a sidebar layout like this one",
  "@heatsync gg",
  "https://heatsync.org/thread/abc123",
];

// HSC_PAINTS=n: the first n fake chatters wear animated heatsync paints (render gate + flood only)
pub fn painted_count() -> u64 {
  std::env::var("HSC_PAINTS").ok().and_then(|v| v.parse().ok()).unwrap_or(0).min(64)
}

// a spread of specs: every base kind, scenes with and without weather, paint/motion/letter effects
pub fn paint_spec(i: u64) -> serde_json::Value {
  use serde_json::json;
  const BACKDROPS: &[&str] =
    &["synth", "dawn", "graveyard", "ocean", "skyline", "orbit", "furnace", "circuit", "arcade", "nightfall"];
  const WEATHERS: &[&str] =
    &["petals", "rain", "snow", "embers", "glyphs", "storm", "fireflies", "confetti", "meteors", "fog"];
  const PAINT_FX: &[&str] = &["pan", "conic", "hue", "glint", "stripes", "stardust", "pulse", "reveal"];
  const MOTION: &[&str] = &["twirl", "coin", "wave", "hop", "float", "heart", "tumble", "heli", "wobble", "swing"];
  let k = i as usize;
  let base = match k % 3 {
    0 => {
      json!({"type": "linear", "angle": 90, "stops": [{"pos": 0, "color": "#ff4040"}, {"pos": 100, "color": "#4040ff"}]})
    }
    1 => {
      json!({"type": "conic", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}, {"pos": 50, "color": "#00ffd7"}, {"pos": 100, "color": "#ff40af"}]})
    }
    _ => json!({"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]}),
  };
  json!({"v": 2, "base": base, "glow": null,
    "scene": {"backdrop": {"id": BACKDROPS[k % BACKDROPS.len()], "tint": "#3080c0"},
              "weather": {"id": WEATHERS[(k * 3) % WEATHERS.len()], "tint": "#ffffff", "density": 2}},
    "effects": [{"id": PAINT_FX[k % PAINT_FX.len()]}, {"id": MOTION[(k * 7) % MOTION.len()]}]})
}

pub struct Fake(u64);

impl Fake {
  pub fn new(seed: u64) -> Self {
    Fake(seed | 1)
  }

  fn next(&mut self) -> usize {
    // xorshift64
    self.0 ^= self.0 << 13;
    self.0 ^= self.0 >> 7;
    self.0 ^= self.0 << 17;
    self.0 as usize
  }

  pub fn message(&mut self, ts: i64) -> Message {
    let n = painted_count();
    let (nick, uid): (String, String) = if n > 0 {
      let u = self.next() as u64 % n;
      (format!("painted{u}"), (u + 1).to_string())
    } else {
      (NICKS[self.next() % NICKS.len()].into(), String::new())
    };
    let nick = nick.as_str();
    Message {
      id: format!("fake-{ts}").into(),
      ts,
      platform: Platform::Twitch,
      kind: Kind::Chat,
      flags: 0,
      login: nick.into(),
      user_id: uid.into(),
      nick: nick.into(),
      color: COLORS[self.next() % COLORS.len()],
      body: BODIES[self.next() % BODIES.len()].into(),
      emotes: Box::new([]),
      hs: Box::new([]),
      badges: Box::new([]),
      reply_to: None,
      reply_text: None,
      system: None,
      x: None,
    }
  }
}

// gives the fake chatters their paints, known up front so nothing is fetched
pub fn seed_paints(cosmetics: &mut crate::cosmetics::Cosmetics) {
  for u in 0..painted_count() {
    if let Some(p) = crate::cosmetics::paint::Paint::from_hs(&paint_spec(u)) {
      cosmetics.seed(
        crate::cosmetics::id::PaintId::Twitch(u + 1),
        crate::cosmetics::Cosmetic { paint: Some(std::sync::Arc::new(p)), badge: None },
      );
    }
  }
}
