// the message being typed: text + cursor (byte index on a char boundary) + sent history.
// words named in `chips` show as their emote and edit as one unit (backspace, delete, arrows); an emoji
// (a grapheme: 👍🏽, 🇯🇵) is one step too
use std::collections::{HashSet, VecDeque};
use std::ops::Range;

// twitch's cap; kick and youtube are lower or equal
pub const MAX_CHARS: usize = 500;
const HISTORY: usize = 50;
// ctrl-z steps kept (ext undo-manager)
const UNDO: usize = 100;

#[derive(Default)]
pub struct Line {
  pub text: String,
  pub cursor: usize,
  // emote names drawn as pictures: picked, tab-completed, recalled, or one of yours typed then spaced
  pub chips: HashSet<Box<str>>,
  // the chips that are overlays (zero-width): one right after another chip stacks onto it
  pub overlays: HashSet<Box<str>>,
  history: VecDeque<String>,
  // Some(i) while browsing history; `draft` holds what was typed before
  browsing: Option<usize>,
  draft: String,
  // ctrl-z / ctrl-y: (text, cursor) before each edit; a run of typed letters is one step
  undo: Vec<(String, usize)>,
  redo: Vec<(String, usize)>,
  typing: bool,
}

impl Line {
  pub fn insert(&mut self, s: &str) {
    let room = MAX_CHARS.saturating_sub(self.text.chars().count());
    // newlines and control bytes never reach a chat line
    let clean: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(room).collect();
    self.text.insert_str(self.cursor, &clean);
    self.cursor += clean.len();
    self.browsing = None;
  }

  // an emote name at the cursor, spaced the way the ext pastes it: a space before unless the
  // text so far is empty or ends in one, and a trailing space to keep typing
  pub fn insert_word(&mut self, w: &str) {
    self.checkpoint(false);
    if !self.text[..self.cursor].is_empty() && !self.text[..self.cursor].ends_with(' ') {
      self.insert(" ");
    }
    self.insert(w);
    self.insert(" ");
  }

  // the same, drawn as its emote
  pub fn insert_chip(&mut self, w: &str) {
    self.chips.insert(w.into());
    self.insert_word(w);
  }

  // an edit is about to happen: remember the text for ctrl-z. letters typed in a row share one step
  pub fn checkpoint(&mut self, typing: bool) {
    let same = typing && self.typing;
    self.typing = typing;
    if same || self.undo.last().is_some_and(|u| u.0 == self.text) {
      return;
    }
    if self.undo.len() == UNDO {
      self.undo.remove(0);
    }
    self.undo.push((self.text.clone(), self.cursor));
    self.redo.clear();
  }

  // anything but typing ends the current step
  pub fn seal(&mut self) {
    self.typing = false;
  }

  pub fn undo(&mut self) {
    self.step(true);
  }

  pub fn redo(&mut self) {
    self.step(false);
  }

  fn step(&mut self, back: bool) {
    let (from, to) = if back { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
    // a snapshot equal to now (an edit that changed nothing) is no step
    while from.last().is_some_and(|u| u.0 == self.text) {
      from.pop();
    }
    let Some((text, cursor)) = from.pop() else { return };
    to.push((std::mem::replace(&mut self.text, text), self.cursor));
    self.cursor = cursor.min(self.text.len());
    self.typing = false;
    self.browsing = None;
  }

  // byte ranges of the words shown as emotes, in order
  pub fn chip_ranges(&self) -> Vec<Range<usize>> {
    if self.chips.is_empty() {
      return Vec::new();
    }
    let mut out = Vec::new();
    let mut at = 0;
    for w in self.text.split(' ') {
      if !w.is_empty() && self.chips.contains(w) {
        out.push(at..at + w.len());
      }
      at += w.len() + 1;
    }
    out
  }

  // the emotes as drawn: an overlay chip right after another chip joins its stack
  pub fn stacks(&self) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for r in self.chip_ranges() {
      if self.overlays.contains(&self.text[r.clone()])
        && let Some(last) = out.last_mut()
        && last.end + 1 == r.start
      {
        last.end = r.end;
      } else {
        out.push(r);
      }
    }
    out
  }

  // the text cut into what vi steps over: each emote stack whole (true), else one grapheme
  pub fn units(&self) -> Vec<(Range<usize>, bool)> {
    let stacks = self.stacks();
    let inside = |b: usize| stacks.iter().any(|r| r.start < b && b < r.end);
    let mut cuts: Vec<usize> = breaks(&self.text).filter(|&b| !inside(b)).collect();
    cuts.extend(stacks.iter().flat_map(|r| [r.start, r.end]));
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2).map(|w| (w[0]..w[1], stacks.iter().any(|r| r.start == w[0]))).collect()
  }

  // the chip the cursor would edit: one ending at it (or around it) going back, starting at it going forward
  fn chip_touching(&self, back: bool) -> Option<Range<usize>> {
    let c = self.cursor;
    self.chip_ranges().into_iter().find(|r| if back { r.start < c && c <= r.end } else { r.start <= c && c < r.end })
  }

  // swap the whole text (vi edits); drops out of history browsing
  pub fn set_text(&mut self, text: String) {
    self.text = text;
    self.browsing = None;
  }

  fn prev(&self, i: usize) -> usize {
    breaks(&self.text).take_while(|&b| b < i).last().unwrap_or(0)
  }

  fn next(&self, i: usize) -> usize {
    next_break(&self.text, i)
  }

  pub fn left(&mut self) {
    self.cursor = self.chip_touching(true).map_or_else(|| self.prev(self.cursor), |r| r.start);
  }

  pub fn right(&mut self) {
    self.cursor = self.chip_touching(false).map_or_else(|| self.next(self.cursor), |r| r.end);
  }

  pub fn home(&mut self) {
    self.cursor = 0;
  }

  pub fn end(&mut self) {
    self.cursor = self.text.len();
  }

  pub fn backspace(&mut self) {
    let r = self.chip_touching(true).unwrap_or_else(|| self.prev(self.cursor)..self.cursor);
    self.text.replace_range(r.clone(), "");
    self.cursor = r.start;
  }

  pub fn delete(&mut self) {
    let r = self.chip_touching(false).unwrap_or_else(|| self.cursor..self.next(self.cursor));
    self.text.replace_range(r.clone(), "");
    self.cursor = r.start;
  }

  // start of the word before the cursor (shell ctrl-w / alt-b)
  fn word_back(&self, from: usize) -> usize {
    let s = &self.text[..from];
    let t = s.trim_end_matches(' ');
    t.rfind(' ').map_or(0, |i| i + 1)
  }

  fn word_fwd(&self, from: usize) -> usize {
    let s = &self.text[from..];
    let skip = s.len() - s.trim_start_matches(' ').len();
    s[skip..].find(' ').map_or(self.text.len(), |i| from + skip + i)
  }

  pub fn delete_word_back(&mut self) {
    let w = self.word_back(self.cursor);
    self.text.replace_range(w..self.cursor, "");
    self.cursor = w;
  }

  pub fn word_left(&mut self) {
    self.cursor = self.word_back(self.cursor);
  }

  pub fn word_right(&mut self) {
    self.cursor = self.word_fwd(self.cursor);
  }

  pub fn clear_to_start(&mut self) {
    self.text.replace_range(..self.cursor, "");
    self.cursor = 0;
  }

  pub fn clear_to_end(&mut self) {
    self.text.truncate(self.cursor);
  }

  // the whole run of non-spaces the cursor touches (tab replaces all of it, past the cursor too)
  pub fn word_around(&self) -> Range<usize> {
    let a = self.text[..self.cursor].rfind(' ').map_or(0, |i| i + 1);
    let b = self.text[self.cursor..].find(' ').map_or(self.text.len(), |i| self.cursor + i);
    a..b
  }

  pub fn replace(&mut self, range: std::ops::Range<usize>, with: &str) {
    self.text.replace_range(range.clone(), with);
    self.cursor = range.start + with.len();
  }

  // Enter: hand back the trimmed message and remember it. None = nothing to send
  pub fn take(&mut self) -> Option<String> {
    let msg = self.text.trim().to_string();
    self.text.clear();
    self.chips.clear();
    self.overlays.clear();
    self.cursor = 0;
    self.undo.clear();
    self.redo.clear();
    self.browsing = None;
    if msg.is_empty() {
      return None;
    }
    if self.history.back() != Some(&msg) {
      if self.history.len() == HISTORY {
        self.history.pop_front();
      }
      self.history.push_back(msg.clone());
    }
    Some(msg)
  }

  // up: older sent message; down: newer, then back to the draft
  pub fn older(&mut self) {
    let i = match self.browsing {
      None if self.history.is_empty() => return,
      None => {
        self.draft = std::mem::take(&mut self.text);
        self.history.len() - 1
      }
      Some(0) => return,
      Some(i) => i - 1,
    };
    self.browsing = Some(i);
    self.text = self.history[i].clone();
    self.cursor = self.text.len();
  }

  pub fn newer(&mut self) {
    let Some(i) = self.browsing else { return };
    if i + 1 < self.history.len() {
      self.browsing = Some(i + 1);
      self.text = self.history[i + 1].clone();
    } else {
      self.browsing = None;
      self.text = std::mem::take(&mut self.draft);
    }
    self.cursor = self.text.len();
  }
}

// grapheme boundaries (byte offsets, 0 and the end included)
pub fn breaks(s: &str) -> impl Iterator<Item = usize> + '_ {
  icu_segmenter::GraphemeClusterSegmenter::new().segment_str(s)
}

// end of the grapheme starting at or containing byte `i` (`i` at the end)
pub fn next_break(s: &str, i: usize) -> usize {
  breaks(s).find(|&b| b > i).unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn edit_on_char_boundaries() {
    let mut l = Line::default();
    l.insert("héllo wörld");
    l.left();
    l.backspace();
    assert_eq!(l.text, "héllo wörd");
    l.home();
    l.right();
    l.right();
    l.delete();
    assert_eq!(l.text, "hélo wörd");
    l.end();
    l.delete_word_back();
    assert_eq!(l.text, "hélo ");
  }

  #[test]
  fn sanitizes_and_caps() {
    let mut l = Line::default();
    l.insert("a\nb\u{7}");
    assert_eq!(l.text, "a b ");
    l.insert(&"x".repeat(1000));
    assert_eq!(l.text.chars().count(), MAX_CHARS);
  }

  #[test]
  fn emote_names_are_spaced() {
    let mut l = Line::default();
    l.insert_word("Kappa");
    assert_eq!(l.text, "Kappa ");
    l.insert_word("LUL");
    assert_eq!(l.text, "Kappa LUL ");
    l.insert("hi");
    l.insert_word("PogU");
    assert_eq!(l.text, "Kappa LUL hi PogU ");
    l.home();
    l.insert_word("A");
    assert_eq!(l.text, "A Kappa LUL hi PogU ");
  }

  #[test]
  fn words() {
    let mut l = Line::default();
    l.insert("one two  three");
    l.word_left();
    assert_eq!(l.cursor, 9);
    l.word_left();
    assert_eq!(l.cursor, 4);
    l.word_right();
    assert_eq!(l.cursor, 7);
    l.end();
    assert_eq!(l.word_around(), 9..14);
    l.insert(" ");
    assert_eq!(l.word_around(), 15..15);
    l.cursor = 11;
    assert_eq!(l.word_around(), 9..14);
  }

  #[test]
  fn chips_edit_whole() {
    let mut l = Line::default();
    l.insert("hi ");
    l.insert_chip("Kappa");
    l.insert("x");
    assert_eq!((l.chip_ranges().len(), l.chip_ranges().first()), (1, Some(&(3..8))));
    // x, then the space, then the whole emote
    l.backspace();
    l.backspace();
    assert_eq!(l.text, "hi Kappa");
    l.backspace();
    assert_eq!(l.text, "hi ");
    l.insert_chip("Kappa");
    l.home();
    l.right();
    l.right();
    l.right();
    assert_eq!(l.cursor, 3);
    l.right();
    assert_eq!(l.cursor, 8);
    l.left();
    assert_eq!(l.cursor, 3);
    l.delete();
    assert_eq!(l.text, "hi  ");
    // a chip name inside a longer word is just text
    l.set_text("Kappas".into());
    assert!(l.chip_ranges().is_empty());
    l.take();
    assert!(l.chips.is_empty());
  }

  #[test]
  fn emoji_edits_whole() {
    let mut l = Line::default();
    l.insert("a👍🏽🇯🇵b");
    l.left();
    l.left();
    assert_eq!(l.cursor, "a👍🏽".len());
    l.backspace();
    assert_eq!(l.text, "a🇯🇵b");
    l.delete();
    assert_eq!(l.text, "ab");
    l.right();
    assert_eq!(l.cursor, 2);
  }

  #[test]
  fn stacks_and_units() {
    let mut l = Line::default();
    l.insert("a Kappa frozen0 🌇");
    l.chips.extend(["Kappa".into(), "frozen0".into()]);
    assert_eq!(l.stacks(), vec![2..7, 8..15], "not an overlay: two emotes");
    l.overlays.insert("frozen0".into());
    assert_eq!(l.stacks(), vec![2..15]);
    let u = l.units();
    assert_eq!(u.len(), 5);
    assert_eq!(u[2], (2..15, true));
    assert_eq!(u[4], (16..l.text.len(), false));
  }

  #[test]
  fn undo_steps_by_word_and_edit() {
    let mut l = Line::default();
    for c in ["h", "i"] {
      l.checkpoint(true);
      l.insert(c);
    }
    l.checkpoint(false);
    l.insert(" ");
    for c in ["y", "o"] {
      l.checkpoint(true);
      l.insert(c);
    }
    l.seal();
    l.checkpoint(false);
    l.backspace();
    assert_eq!(l.text, "hi y");
    l.undo();
    assert_eq!(l.text, "hi yo");
    l.undo();
    assert_eq!(l.text, "hi ");
    l.undo();
    assert_eq!(l.text, "hi");
    l.undo();
    assert_eq!(l.text, "");
    l.undo();
    assert_eq!(l.text, "");
    l.redo();
    l.redo();
    assert_eq!((l.text.as_str(), l.cursor), ("hi ", 3));
    // a new edit drops what could be redone
    l.checkpoint(false);
    l.insert("!");
    l.redo();
    assert_eq!(l.text, "hi !");
  }

  #[test]
  fn history_keeps_the_draft() {
    let mut l = Line::default();
    for m in ["first", "second"] {
      l.insert(m);
      assert_eq!(l.take().as_deref(), Some(m));
    }
    assert_eq!(l.take(), None);
    l.insert("draft");
    l.older();
    assert_eq!(l.text, "second");
    l.older();
    l.older();
    assert_eq!(l.text, "first");
    l.newer();
    l.newer();
    assert_eq!(l.text, "draft");
  }
}
