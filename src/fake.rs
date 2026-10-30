// deterministic synthetic chat for the render gate and flood testing
use crate::model::message::Message;
use crate::ui::theme;

const NICKS: &[&str] = &[
  "xqcow",
  "pokimane",
  "forsen",
  "heatsync",
  "a_very_long_username_25",
  "nymn",
  "tyler1",
  "sodapoppin",
];
const COLORS: &[u32] = &[
  theme::RED,
  theme::GREEN,
  theme::YELLOW,
  theme::BLUE,
  theme::MAGENTA,
  theme::CYAN,
  theme::HEATSYNC,
];
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
    Message {
      ts,
      nick: NICKS[self.next() % NICKS.len()].into(),
      color: COLORS[self.next() % COLORS.len()],
      body: BODIES[self.next() % BODIES.len()].into(),
    }
  }
}
