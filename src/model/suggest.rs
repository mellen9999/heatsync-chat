// the lists that open over the message box as you type (ext input.js dropdowns): `:ke` (2+ letters)
// lists emotes, `@` lists people, a lone `/word` lists commands. up/down move, tab or enter picks,
// esc closes until the text changes
use std::ops::Range;

use super::complete::Pick;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
  Emote,
  People,
  Command,
}

impl Kind {
  // the ext's caps: 30 emotes, 20 people, 8 commands
  pub fn max(self) -> usize {
    match self {
      Kind::Emote => 30,
      Kind::People => 20,
      Kind::Command => 8,
    }
  }
}

pub struct Suggest {
  pub kind: Kind,
  // the typed bytes a pick replaces
  pub range: Range<usize>,
  pub items: Vec<Pick>,
  pub sel: usize,
}

impl Suggest {
  pub fn step(&mut self, down: bool) {
    let n = self.items.len().max(1);
    self.sel = if down { (self.sel + 1) % n } else { (self.sel + n - 1) % n };
  }
}

fn name_byte(b: u8) -> bool {
  b.is_ascii_alphanumeric() || b == b'_'
}

// what the text before the cursor asks for: the list, the bytes a pick replaces, the query (with its `:`/`@`/`/`)
pub fn trigger(text: &str, cursor: usize) -> Option<(Kind, Range<usize>)> {
  let t = text.as_bytes();
  if t.first() == Some(&b'/') && t[1..].iter().all(|b| b.is_ascii_alphabetic() || *b == b'?') {
    return Some((Kind::Command, 0..text.len()));
  }
  let head = &t[..cursor];
  let run = head.iter().rev().take_while(|b| name_byte(**b)).count();
  let mark = cursor.checked_sub(run + 1)?;
  match head[mark] {
    b':' if run >= 2 => Some((Kind::Emote, mark..cursor)),
    b'@' => Some((Kind::People, mark..cursor)),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn triggers() {
    let at = |s: &str| trigger(s, s.len());
    assert_eq!(at("hi :ke"), Some((Kind::Emote, 3..6)));
    assert_eq!(at("hi :k"), None);
    // like the ext, the colon needn't start a word
    assert_eq!(at("12:30"), Some((Kind::Emote, 2..5)));
    assert_eq!(at("hey @"), Some((Kind::People, 4..5)));
    assert_eq!(at("hey @al_1"), Some((Kind::People, 4..9)));
    assert_eq!(at("/ba"), Some((Kind::Command, 0..3)));
    assert_eq!(at("/"), Some((Kind::Command, 0..1)));
    assert_eq!(at("/ban "), None);
    assert_eq!(at("hey @al "), None);
    assert_eq!(at("plain"), None);
    assert_eq!(trigger("hey @al there", 7), Some((Kind::People, 4..7)));
  }

  #[test]
  fn steps_wrap() {
    let p = |n: &str| Pick { name: n.into(), what: "", emote: true };
    let mut s = Suggest { kind: Kind::Emote, range: 0..0, items: vec![p("a"), p("b")], sel: 0 };
    s.step(false);
    assert_eq!(s.sel, 1);
    s.step(true);
    assert_eq!(s.sel, 0);
  }
}
