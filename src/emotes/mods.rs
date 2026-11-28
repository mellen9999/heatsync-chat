//! the words that change an emote: `w! Kappa` is a wide Kappa, `Kappa ffzX` a mirrored one, `w!h!` a
//! chain, `c!#f00` a tint. the grammar and every number are the extension's src/lib/modifiers.js (the
//! site keeps a byte copy), and `emote-mods.vectors.json` is the rule as data, vendored byte for byte from
//! the site (scripts/sync-emote-vectors.sh) and run below: a line reads the same in every app. what moves
//! (`p!`, `ffzSpin`, …) moves as the extension's css moves it: emotes/motion.rs

use super::motion::{Affine, Motion, Pose};

const MAX_SCALE: i32 = 4;

#[derive(Clone, Copy)]
enum Mod {
  Scale(i32, i32),
  Turn(i32),
  Paint(Paint),
  Move(Motion),
  // `z!` (zero space): the extension's chat draws nothing for it, so neither does this app
  Zero,
}

// a colour look, each a chain of css filters. party's and rainbow's end on the turn of the colours their
// animation drives (`hue-rotate(var(--hs-fx-hue))`), none at rest
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paint {
  Cursed,
  Hyper,
  Party,
  Rainbow,
}

use Mod::{Move, Paint as P, Scale, Turn};

// longest first: a chain peels `ffzWide` before `ffzW`
const TOKENS: [(&str, &[Mod]); 25] = [
  ("ffzRainbow", &[P(Paint::Rainbow), Move(Motion::Rainbow)]),
  ("ffzCursed", &[P(Paint::Cursed)]),
  ("ffzArrive", &[Move(Motion::Arrive)]),
  ("ffzBounce", &[Move(Motion::Bounce)]),
  ("ffzHyper", &[P(Paint::Hyper), Move(Motion::Hyper)]),
  ("ffzLeave", &[Move(Motion::Leave)]),
  ("ffzSlide", &[Move(Motion::Slide)]),
  ("ffzWide", &[Scale(2, 1)]),
  ("ffzTall", &[Scale(1, 2)]),
  ("ffzSpin", &[Move(Motion::Spin)]),
  ("ffzJam", &[Move(Motion::Jam)]),
  ("ffzX", &[Scale(-1, 1)]),
  ("ffzY", &[Scale(1, -1)]),
  ("ffzW", &[Scale(2, 1)]),
  ("w!", &[Scale(2, 1)]),
  ("h!", &[Scale(-1, 1)]),
  ("v!", &[Scale(1, -1)]),
  ("z!", &[Mod::Zero]),
  ("c!", &[P(Paint::Cursed)]),
  ("l!", &[Turn(-1)]),
  ("r!", &[Turn(1)]),
  ("p!", &[P(Paint::Party), Move(Motion::Party)]),
  ("s!", &[Move(Motion::Shake)]),
  ("x!", &[Scale(-1, 1)]),
  ("y!", &[Scale(1, -1)]),
];

// one step of a css filter on a pixel's colour, each channel 0..1 and clamped after every step
#[derive(Clone, Copy)]
enum Op {
  Mat([f32; 9]),
  Lin(f32, f32),
}

fn gray() -> Op {
  Op::Mat([0.2126, 0.7152, 0.0722, 0.2126, 0.7152, 0.0722, 0.2126, 0.7152, 0.0722])
}

fn sepia(a: f32) -> Op {
  let t = 1.0 - a;
  Op::Mat([
    0.393 + 0.607 * t,
    0.769 - 0.769 * t,
    0.189 - 0.189 * t,
    0.349 - 0.349 * t,
    0.686 + 0.314 * t,
    0.168 - 0.168 * t,
    0.272 - 0.272 * t,
    0.534 - 0.534 * t,
    0.131 + 0.869 * t,
  ])
}

fn saturate(s: f32) -> Op {
  Op::Mat([
    0.213 + 0.787 * s,
    0.715 - 0.715 * s,
    0.072 - 0.072 * s,
    0.213 - 0.213 * s,
    0.715 + 0.285 * s,
    0.072 - 0.072 * s,
    0.213 - 0.213 * s,
    0.715 - 0.715 * s,
    0.072 + 0.928 * s,
  ])
}

fn hue_rotate(deg: f32) -> Op {
  let (s, c) = deg.to_radians().sin_cos();
  Op::Mat([
    0.213 + c * 0.787 - s * 0.213,
    0.715 - c * 0.715 - s * 0.715,
    0.072 - c * 0.072 + s * 0.928,
    0.213 - c * 0.213 + s * 0.143,
    0.715 + c * 0.285 + s * 0.140,
    0.072 - c * 0.072 - s * 0.283,
    0.213 - c * 0.213 - s * 0.787,
    0.715 - c * 0.715 + s * 0.715,
    0.072 + c * 0.928 + s * 0.072,
  ])
}

// `c!#f00` names a colour; what it turns the emote by is that colour's hue, in whole degrees
fn hue_of(hex: &str) -> u16 {
  let wide: String = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex.into() };
  let ch = |i: usize| u8::from_str_radix(&wide[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
  let (r, g, b) = (ch(0), ch(2), ch(4));
  let (max, min) = (r.max(g).max(b), r.min(g).min(b));
  let d = max - min;
  if d == 0.0 {
    return 0;
  }
  let h = if max == r {
    ((g - b) / d) % 6.0
  } else if max == g {
    (b - r) / d + 2.0
  } else {
    (r - g) / d + 4.0
  } * 60.0;
  (if h < 0.0 { h + 360.0 } else { h } + 0.5).floor() as u16
}

// a tint at the head of `s` (`c!` + an optional `#` + six hex digits, else three): its hue and its length
fn tint_at(s: &str) -> Option<(u16, usize)> {
  let rest = s.strip_prefix("c!")?;
  let hex = rest.strip_prefix('#').unwrap_or(rest);
  let n = hex.bytes().take(6).take_while(u8::is_ascii_hexdigit).count();
  let n = match n {
    6 => 6,
    3..=5 => 3,
    _ => return None,
  };
  Some((hue_of(&hex[..n]), s.len() - hex.len() + n))
}

// what the modifiers on an emote add up to
#[derive(Clone, PartialEq, Debug)]
pub struct Fx {
  sx: i32,
  sy: i32,
  turn: i32,
  paint: Vec<Paint>,
  tint: Option<u16>,
  // the moving effects named, one bit each (`Motion::bit`)
  moves: u16,
}

impl Default for Fx {
  fn default() -> Self {
    Fx { sx: 1, sy: 1, turn: 0, paint: Vec::new(), tint: None, moves: 0 }
  }
}

// a modifier word: a whole token, a tint, or a chain of them with nothing left over. case is exact
pub fn is(word: &str) -> bool {
  let b = word.as_bytes();
  b.len() >= 2 && (b[1] == b'!' || b.starts_with(b"ffz")) && Fx::default().add(word)
}

// a modifier word that changes the emote AFTER it when there is one: betterttv's own (`w!`), a tint, a
// chain led by one. frankerfacez's (`ffzX`) change the emote before them
fn forward(word: &str) -> bool {
  word.as_bytes().get(1) == Some(&b'!')
}

impl Fx {
  // `word` read as modifiers and added to these; false, and nothing changed, when it is an ordinary word
  pub fn add(&mut self, word: &str) -> bool {
    let mut next = self.clone();
    let mut rem = word;
    while !rem.is_empty() {
      if let Some((hue, n)) = tint_at(rem) {
        next.tint = Some(hue);
        rem = &rem[n..];
        continue;
      }
      let Some((tok, ms)) = TOKENS.iter().find(|t| rem.starts_with(t.0)) else { return false };
      rem = &rem[tok.len()..];
      for m in *ms {
        match *m {
          Mod::Scale(x, y) => {
            next.sx = next.sx.saturating_mul(x);
            next.sy = next.sy.saturating_mul(y);
          }
          Mod::Turn(q) => next.turn += q,
          Mod::Paint(p) => next.paint.push(p),
          Mod::Move(m) => next.moves |= m.bit(),
          Mod::Zero => {}
        }
      }
    }
    if word.is_empty() {
      return false;
    }
    *self = next;
    true
  }

  // the stretch each way, a minus mirrored, at most four
  fn scale(&self) -> (i32, i32) {
    (self.sx.clamp(-MAX_SCALE, MAX_SCALE), self.sy.clamp(-MAX_SCALE, MAX_SCALE))
  }

  // quarter turns clockwise, 0..4
  pub fn turn(&self) -> u8 {
    self.turn.rem_euclid(4) as u8
  }

  // mirrored left-right, top-bottom (after the turn)
  pub fn mirror(&self) -> (bool, bool) {
    (self.sx < 0, self.sy < 0)
  }

  // the turn and the stretch as a map of the picture's points (its middle at 0, 0): turned, then stretched
  // and mirrored, as the extension's `scale(sx, sy) rotate()`
  pub fn map(&self) -> Affine {
    let (sx, sy) = self.scale();
    Affine::scale(sx as f32, sy as f32).then_from(Affine::rotate(90.0 * self.turn() as f32))
  }

  // the room a w×h picture takes once turned and stretched
  pub fn size(&self, (w, h): (u32, u32)) -> (u32, u32) {
    let (w, h) = if self.turn() % 2 == 1 { (h, w) } else { (w, h) };
    let (sx, sy) = self.scale();
    (w * sx.unsigned_abs(), h * sy.unsigned_abs())
  }

  // the moving effect that runs: of the ones named, the last in the extension's css (each sets `animation`,
  // so one wins). None = it stands still
  pub fn motion(&self) -> Option<Motion> {
    (0..16).rev().find(|b| self.moves & (1 << b) != 0).and_then(|b| Motion::ALL.get(b as usize).copied())
  }

  // a bounce moves the picture's turning point to its bottom middle (`transform-origin: bottom center`), for
  // whichever animation runs
  pub fn grows_up(&self) -> bool {
    self.moves & Motion::Bounce.bit() != 0
  }

  // where the moving effect has the picture `ms` into its clock (at rest without one, or when not playing)
  pub fn pose(&self, ms: u64, play: bool) -> Pose {
    self.motion().filter(|_| play).map_or(Pose::REST, |m| m.pose(ms))
  }

  // what the colours go through, None when they are untouched: the looks in the order typed, the tint last.
  // `hue`: the turn the party and rainbow looks are at (the pose's), 0 at rest
  pub fn ink(&self, hue: f32) -> Option<Ink> {
    let mut ops = Vec::new();
    let turn = |ops: &mut Vec<Op>| {
      if hue.rem_euclid(360.0) != 0.0 {
        ops.push(hue_rotate(hue));
      }
    };
    for p in &self.paint {
      match p {
        Paint::Cursed => ops.extend([gray(), Op::Lin(0.7, 0.0), Op::Lin(2.5, -0.75)]),
        Paint::Hyper => {
          ops.extend([Op::Lin(0.2, 0.0), sepia(1.0), Op::Lin(2.2, 0.0), Op::Lin(3.0, -1.0), saturate(8.0)])
        }
        Paint::Party => {
          ops.extend([sepia(0.5), saturate(2.5)]);
          turn(&mut ops);
        }
        Paint::Rainbow => turn(&mut ops),
      }
    }
    if let Some(h) = self.tint {
      ops.extend([hue_rotate(h as f32), saturate(1.6)]);
    }
    (!ops.is_empty()).then_some(Ink(ops))
  }
}

// the css filters of one emote, ready to run on its pixels
pub struct Ink(Vec<Op>);

impl Ink {
  pub fn of(&self, [r, g, b, a]: [u8; 4]) -> [u8; 4] {
    if a == 0 {
      return [r, g, b, a];
    }
    let mut p = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
    for op in &self.0 {
      p = match *op {
        Op::Mat(m) => [
          m[0] * p[0] + m[1] * p[1] + m[2] * p[2],
          m[3] * p[0] + m[4] * p[1] + m[5] * p[2],
          m[6] * p[0] + m[7] * p[1] + m[8] * p[2],
        ],
        Op::Lin(k, d) => p.map(|c| c * k + d),
      }
      .map(|c| c.clamp(0.0, 1.0));
    }
    let q = |c: f32| (c * 255.0).round() as u8;
    [q(p[0]), q(p[1]), q(p[2]), a]
  }
}

// a word of a line, as the binding rule sees it
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Word {
  // nothing there (two spaces in a row, a folded word)
  Gap,
  Text,
  Emote,
  // a modifier word; `emote`: the room also has an emote of this name
  Mod { emote: bool },
}

// which emote each modifier word of a line changes: for word i, the index of that emote's word, or None
// when it stays as typed (a word, or the emote of its name). each provider's own direction first, so a
// line means here what it means to that provider's users: betterttv's take the emote after them,
// frankerfacez's the emote before; the other side is the fallback. only modifiers may stand between a
// modifier and its emote: it never reaches over text
pub fn bind(line: &[(Word, &str)]) -> Vec<Option<usize>> {
  let mut out = vec![None; line.len()];
  // the emote a word reaching back would change
  let mut last = None;
  for (i, &(kind, word)) in line.iter().enumerate() {
    match kind {
      Word::Gap => {}
      Word::Text => last = None,
      Word::Emote => last = Some(i),
      Word::Mod { emote } => {
        let next = (i + 1..line.len())
          .find(|&j| !matches!(line[j].0, Word::Gap | Word::Mod { .. }))
          .filter(|&j| line[j].0 == Word::Emote);
        out[i] = if forward(word) { next.or(last) } else { last.or(next) };
        if out[i].is_none() {
          last = emote.then_some(i);
        }
      }
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::Value;

  const FILE: &str = include_str!("emote-mods.vectors.json");

  fn fx(words: &str) -> Option<Fx> {
    let mut f = Fx::default();
    words.split(' ').all(|w| f.add(w)).then_some(f)
  }

  fn rows(key: &str) -> Vec<Value> {
    let v: Value = serde_json::from_str(FILE).expect("the vectors parse");
    let rows = v[key].as_array().expect(key).clone();
    assert!(rows.len() >= 40, "{key}: {} rows", rows.len());
    rows
  }

  // the file is the site's, byte for byte: its hash is kept beside it and compared across the apps
  #[test]
  fn the_vendored_vectors_are_the_ones_synced() {
    let sum = ring::digest::digest(&ring::digest::SHA256, FILE.as_bytes());
    let hex: String = sum.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, include_str!("emote-mods.vectors.sha256").trim(), "run scripts/sync-emote-vectors.sh");
  }

  #[test]
  fn every_word_adds_up_as_the_vectors_say() {
    for r in rows("words") {
      let w = r["words"].as_str().unwrap();
      let known = ["words", "plain", "sx", "sy", "turn", "hue", "paint"];
      assert!(
        r.as_object().unwrap().keys().all(|k| known.contains(&k.as_str())),
        "{w}: a field this runner does not know"
      );
      if r["plain"] == true {
        assert!(!is(w) && fx(w).is_none(), "{w:?} is an ordinary word");
        continue;
      }
      assert!(w.split(' ').all(is), "{w:?}: every word is a modifier");
      let f = fx(w).unwrap();
      // rainbow only turns the colours as it moves: at rest it paints nothing, and the vectors list looks at rest
      let paint = |p: &Paint| match p {
        Paint::Cursed => Some("cursed"),
        Paint::Hyper => Some("hyper"),
        Paint::Party => Some("party"),
        Paint::Rainbow => None,
      };
      let got = serde_json::json!({
        "words": w, "sx": f.scale().0, "sy": f.scale().1, "turn": f.turn(), "hue": f.tint,
        "paint": f.paint.iter().filter_map(paint).collect::<Vec<_>>(),
      });
      assert_eq!(got, r, "{w}");
    }
  }

  #[test]
  fn every_line_binds_as_the_vectors_say() {
    for r in rows("lines") {
      let (name, line) = (r["name"].as_str().unwrap(), r["line"].as_str().unwrap());
      assert_eq!(r.as_object().unwrap().len(), 4, "{name}: a field this runner does not know");
      let emotes: Vec<&str> = r["emotes"].as_array().unwrap().iter().map(|e| e.as_str().unwrap()).collect();
      let words: Vec<(Word, &str)> = line
        .split(' ')
        .map(|w| {
          let kind = match (w.is_empty(), is(w), emotes.contains(&w)) {
            (true, ..) => Word::Gap,
            (_, true, emote) => Word::Mod { emote },
            (_, _, true) => Word::Emote,
            _ => Word::Text,
          };
          (kind, w)
        })
        .collect();
      let got: serde_json::Map<String, Value> =
        bind(&words).iter().enumerate().filter_map(|(i, t)| t.map(|t| (i.to_string(), t.into()))).collect();
      assert_eq!(Value::Object(got), r["want"], "{name}: {line:?}");
    }
  }

  #[test]
  fn a_shape_is_the_turned_picture_stretched() {
    let size = |w: &str| fx(w).unwrap().size((30, 10));
    assert_eq!(size("w!"), (60, 10));
    assert_eq!(size("ffzTall"), (30, 20));
    assert_eq!(size("l!"), (10, 30));
    assert_eq!(size("w!l!"), (20, 30), "the stretch is of the turned picture");
    assert_eq!(size("w! w! w!"), (120, 10), "four times is the most");
    assert_eq!(size("h! p! c!#f00 z!"), (30, 10));
    // a word that fails part way adds nothing
    let mut f = Fx::default();
    assert!(!f.add("w!h!nope") && f == Fx::default());
  }

  // each answer is the css filter spec's arithmetic for that pixel, worked by hand
  #[test]
  fn the_colours_are_the_css_filters() {
    let ink = |w: &str, p: [u8; 4]| fx(w).unwrap().ink(0.0).map_or(p, |k| k.of(p));
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= 1);
    // nothing to do is nothing done, and a clear pixel stays as it was
    assert!(fx("w!").unwrap().ink(0.0).is_none() && fx("ffzRainbow").unwrap().ink(0.0).is_none());
    assert_eq!(ink("c!", [9, 9, 9, 0]), [9, 9, 9, 0]);
    // cursed: grey, darker, then hard contrast. red goes black, white stays white
    assert!(near(ink("c!", [255, 0, 0, 255]), [0, 0, 0, 255]));
    assert!(near(ink("c!", [255, 255, 255, 255]), [255, 255, 255, 255]));
    assert!(near(ink("c!", [200, 200, 200, 128]), [159, 159, 159, 128]));
    // a tint turns the hue: red by 120 degrees is a green
    let g = ink("c!#0f0", [255, 0, 0, 255]);
    assert!(g[1] > g[0] && g[1] > g[2], "{g:?}");
    // a grey has no hue to turn
    assert!(near(ink("c!#00f", [128, 128, 128, 255]), [128, 128, 128, 255]));
    // party at rest: half sepia, more colour
    let p = ink("p!", [100, 150, 200, 255]);
    assert!(p[3] == 255 && p != [100, 150, 200, 255]);
    // looks stack in the order typed, the tint last
    assert_eq!(fx("c! c!#0f0 p!").unwrap().ink(0.0).unwrap().0.len(), 3 + 2 + 2);
  }
}
