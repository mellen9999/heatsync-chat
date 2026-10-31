// the message being typed: text + cursor (byte index on a char boundary) + sent history
use std::collections::VecDeque;

// twitch's cap; kick and youtube are lower or equal
pub const MAX_CHARS: usize = 500;
const HISTORY: usize = 50;

#[derive(Default)]
pub struct Line {
  pub text: String,
  pub cursor: usize,
  history: VecDeque<String>,
  // Some(i) while browsing history; `draft` holds what was typed before
  browsing: Option<usize>,
  draft: String,
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
    if !self.text[..self.cursor].is_empty() && !self.text[..self.cursor].ends_with(' ') {
      self.insert(" ");
    }
    self.insert(w);
    self.insert(" ");
  }

  // swap the whole text (vi edits); drops out of history browsing
  pub fn set_text(&mut self, text: String) {
    self.text = text;
    self.browsing = None;
  }

  fn prev(&self, i: usize) -> usize {
    self.text[..i].char_indices().next_back().map_or(0, |(j, _)| j)
  }

  fn next(&self, i: usize) -> usize {
    self.text[i..].chars().next().map_or(i, |c| i + c.len_utf8())
  }

  pub fn left(&mut self) {
    self.cursor = self.prev(self.cursor);
  }

  pub fn right(&mut self) {
    self.cursor = self.next(self.cursor);
  }

  pub fn home(&mut self) {
    self.cursor = 0;
  }

  pub fn end(&mut self) {
    self.cursor = self.text.len();
  }

  pub fn backspace(&mut self) {
    let p = self.prev(self.cursor);
    self.text.replace_range(p..self.cursor, "");
    self.cursor = p;
  }

  pub fn delete(&mut self) {
    let n = self.next(self.cursor);
    self.text.replace_range(self.cursor..n, "");
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

  // the word under/behind the cursor and where it starts (tab-complete target)
  pub fn current_word(&self) -> (usize, &str) {
    let a = self.word_back(self.cursor);
    let a = if self.text[a..self.cursor].contains(' ') { self.cursor } else { a };
    (a, &self.text[a..self.cursor])
  }

  pub fn replace(&mut self, range: std::ops::Range<usize>, with: &str) {
    self.text.replace_range(range.clone(), with);
    self.cursor = range.start + with.len();
  }

  // Enter: hand back the trimmed message and remember it. None = nothing to send
  pub fn take(&mut self) -> Option<String> {
    let msg = self.text.trim().to_string();
    self.text.clear();
    self.cursor = 0;
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
    assert_eq!(l.current_word(), (9, "three"));
    l.insert(" ");
    assert_eq!(l.current_word(), (15, ""));
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
