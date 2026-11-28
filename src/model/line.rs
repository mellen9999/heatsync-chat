// the message being typed: text + cursor (byte index on a char boundary) + sent history.
// words named in `chips` show as their emote and edit as one unit (backspace, delete, arrows); an emoji
// (a grapheme: 👍🏽, 🇯🇵) is one step too
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;

// twitch's cap; kick and youtube are lower or equal
pub const MAX_CHARS: usize = 500;
const HISTORY: usize = 50;
// ctrl-z steps kept (ext undo-manager)
const UNDO: usize = 100;

// one undo step: the text, the cursor, and the draft's picks
type Step = (String, usize, super::catalog::Registry);

#[derive(Default)]
pub struct Line {
  pub text: String,
  pub cursor: usize,
  // emote names drawn as pictures: picked, tab-completed, recalled, or one of yours typed then spaced
  pub chips: HashSet<Box<str>>,
  // the chips that are overlays (zero-width): one right after another chip stacks onto it
  pub overlays: HashSet<Box<str>>,
  // the pictures picked on purpose in this draft, by name (tab, the picker, a clicked emote). the draft's
  // own: gone when the line is sent, swapped out or loaded over
  pub picks: super::catalog::Registry,
  // the draft's picks, set aside with it while a sent line is looked at
  draft_picks: super::catalog::Registry,
  history: VecDeque<String>,
  // the copies picked on purpose in a sent line, by its text (`keep`): a recalled line comes back with
  // them. this run only: the sent file holds the words alone
  kept: HashMap<String, super::catalog::Registry>,
  // Some(i) while browsing history; `draft` holds what was typed before
  browsing: Option<usize>,
  draft: String,
  // ctrl-z / ctrl-y: the box before each edit (text, cursor, and what each picked word meant); a run of
  // typed letters is one step. an undo is the one way back for a pick whose word left the box
  undo: Vec<Step>,
  redo: Vec<Step>,
  typing: bool,
}

impl Line {
  pub fn insert(&mut self, s: &str) {
    let room = MAX_CHARS.saturating_sub(self.text.chars().count());
    // newlines and control bytes never reach a chat line
    let clean: String = crate::util::blank(s).take(room).collect();
    self.text.insert_str(self.cursor, &clean);
    self.cursor += clean.len();
    self.browsing = None;
  }

  // text entered at the cursor from outside the box: a key, a paste, an input method, a line the app puts
  // in. a pick whose word left the box is over before it lands, so the name entered again is a hand-typed
  // word however it came (undo, the one way back, comes before this). and once it has landed: a letter
  // typed against a picked word makes it another word, so the name is hand-typed when the letter is gone
  pub fn enter(&mut self, s: &str) {
    self.picks.prune(&self.text);
    self.insert(s);
    self.picks.prune(&self.text);
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
    if same {
      return;
    }
    // the same text again (an edit that changed nothing): one step, with what its words mean now
    if let Some(last) = self.undo.last_mut().filter(|u| u.0 == self.text) {
      last.2 = self.picks.clone();
      return;
    }
    if self.undo.len() == UNDO {
      self.undo.remove(0);
    }
    self.undo.push((self.text.clone(), self.cursor, self.picks.clone()));
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
    // the box as it was, picks and all: text entered after a delete ended the deleted word's pick, and
    // the undone word used to come back as the room's emote
    let Some((text, cursor, picks)) = from.pop() else { return };
    let was = std::mem::replace(&mut self.picks, picks);
    to.push((std::mem::replace(&mut self.text, text), self.cursor, was));
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

  // the modifier words of the box (`Okayge p!`, `w! Kappa`) and the word of the chip each changes, by the rule
  // a chat line is read by (emotes::mods::bind), so the box shows what the line will: a chip is an emote, and
  // a modifier word is a modifier first even when it is a chip itself (a picked bttv `p!`). the word the
  // caret is still in is left as typed until it is finished (the ext's settled sweep)
  pub fn mod_binds(&self) -> Vec<(Range<usize>, Range<usize>)> {
    use crate::emotes::mods::{self, Word};
    if self.chips.is_empty() || !self.text.split(' ').any(mods::is) {
      return Vec::new();
    }
    let mut at = 0;
    let mut words = Vec::new();
    for w in self.text.split(' ') {
      let r = at..at + w.len();
      at = r.end + 1;
      let open = r.start < self.cursor && self.cursor <= r.end;
      let kind = match (w.is_empty(), mods::is(w) && !open, self.chips.contains(w)) {
        (true, ..) => Word::Gap,
        (_, true, chip) => Word::Mod { emote: chip },
        (_, _, true) => Word::Emote,
        _ => Word::Text,
      };
      words.push((kind, w, r));
    }
    let line: Vec<(Word, &str)> = words.iter().map(|(k, w, _)| (*k, *w)).collect();
    let binds = mods::bind(&line);
    binds.iter().enumerate().filter_map(|(i, t)| Some((words[i].2.clone(), words[(*t)?].2.clone()))).collect()
  }

  // the emotes as drawn: an overlay chip right after another chip joins its stack
  // (an overlay right after a lone emoji stacks on the emoji, like a chat line: the stack starts there), and
  // a modifier word folds into the stack beside it (`mod_binds`): drawn as the change, stepped over whole
  pub fn stacks(&self) -> Vec<Range<usize>> {
    let binds = self.mod_binds();
    let mut out: Vec<Range<usize>> = Vec::new();
    // a picked modifier (`p!` chosen in the picker) that folds is the change, not a picture of its own
    for r in self.chip_ranges().into_iter().filter(|r| !binds.iter().any(|(m, _)| m == r)) {
      let overlay = self.overlays.contains(&self.text[r.clone()]);
      if overlay
        && let Some(last) = out.last_mut()
        && last.end + 1 == r.start
      {
        last.end = r.end;
      } else if overlay && let Some(e) = self.emoji_before(r.start) {
        out.push(e..r.end);
      } else {
        out.push(r);
      }
    }
    if binds.is_empty() {
      return out;
    }
    // the stacks and the folded words in order; a folded word joins the stack it touches, through other
    // folded words: the one before it, else the one after
    let mut items: Vec<(Range<usize>, bool)> = out.into_iter().map(|r| (r, true)).collect();
    items.extend(binds.into_iter().map(|(m, _)| (m, false)));
    items.sort_by_key(|i| i.0.start);
    let mut groups: Vec<(Range<usize>, bool)> = Vec::new();
    for (r, stack) in items {
      match groups.last_mut() {
        Some(g) if g.0.end + 1 == r.start && !(g.1 && stack) => {
          g.0.end = r.end;
          g.1 |= stack;
        }
        _ => groups.push((r, stack)),
      }
    }
    groups.into_iter().filter(|g| g.1).map(|g| g.0).collect()
  }

  // where the word before byte `at` starts, when that word is one emoji and nothing else
  pub fn emoji_before(&self, at: usize) -> Option<usize> {
    let head = self.text.get(..at)?.strip_suffix(' ')?;
    let a = head.rfind(' ').map_or(0, |i| i + 1);
    crate::ui::text::lone_emoji(&head[a..]).then_some(a)
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

  // swap in a whole text with the cursor at its end and a fresh undo history, never into the sent history
  // (a post being edited, or the draft that was set aside for it)
  pub fn load(&mut self, text: String) {
    self.cursor = text.len();
    self.text = text;
    self.chips.clear();
    self.overlays.clear();
    self.picks = Default::default();
    self.undo.clear();
    self.redo.clear();
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
    self.picks = Default::default();
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
    // the newest send of a line is the one that comes back: sent again by hand, it has no pick (`keep`
    // files this send's, when it has any). a line that fell off the list takes its picks along
    self.kept.remove(&msg);
    self.kept.retain(|l, _| self.history.contains(l));
    Some(msg)
  }

  // the line just sent (what `take` handed back) went out with these picks, each a copy chosen over what
  // its word would have sent: recalled, the line shows and ships them again. the emote law: a pick is the
  // line's, never the name's, so only a recall brings one back
  pub fn keep(&mut self, line: &str, picks: super::catalog::Registry) {
    if !picks.is_empty() && self.history.back().map(String::as_str) == Some(line) {
      self.kept.insert(line.into(), picks);
    }
  }

  // the sent line being looked at, as it went out: its words and the copies picked in it
  fn show(&mut self, i: usize) {
    self.browsing = Some(i);
    self.text = self.history[i].clone();
    self.picks = self.kept.get(&self.text).cloned().unwrap_or_default();
    self.cursor = self.text.len();
  }

  // what was sent, oldest first (the file `sent_file` keeps between runs)
  pub fn sent(&self) -> impl DoubleEndedIterator<Item = &str> {
    self.history.iter().map(String::as_str)
  }

  // the last run's sent lines come back, oldest first. lines sent before they arrived stay newest
  pub fn restore_sent(&mut self, old: Vec<String>) {
    let now: Vec<String> = self.history.drain(..).collect();
    for l in old.into_iter().chain(now) {
      if self.history.back() != Some(&l) {
        if self.history.len() == HISTORY {
          self.history.pop_front();
        }
        self.history.push_back(l);
      }
    }
    self.browsing = None;
  }

  // forget every sent line (and stop browsing them)
  pub fn clear_sent(&mut self) {
    if self.browsing.take().is_some() {
      self.text = std::mem::take(&mut self.draft);
      self.picks = std::mem::take(&mut self.draft_picks);
      self.cursor = self.text.len();
    }
    self.history.clear();
    self.kept.clear();
  }

  // up: older sent message; down: newer, then back to the draft
  pub fn older(&mut self) {
    let i = match self.browsing {
      None if self.history.is_empty() => return,
      None => {
        self.draft = std::mem::take(&mut self.text);
        self.draft_picks = std::mem::take(&mut self.picks);
        self.history.len() - 1
      }
      Some(0) => return,
      Some(i) => i - 1,
    };
    self.show(i);
  }

  pub fn newer(&mut self) {
    let Some(i) = self.browsing else { return };
    if i + 1 < self.history.len() {
      return self.show(i + 1);
    }
    self.browsing = None;
    self.text = std::mem::take(&mut self.draft);
    self.picks = std::mem::take(&mut self.draft_picks);
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
  fn an_overlay_after_an_emoji_stacks_on_it() {
    let mut l = Line::default();
    l.insert("hi 🏙 fog0 x");
    l.chips.insert("fog0".into());
    assert_eq!(l.stacks(), vec![8..12], "not an overlay: just the emote");
    l.overlays.insert("fog0".into());
    assert_eq!(l.stacks(), vec![3..12], "the emoji and its overlay, one stack");
    // a word that is more than one emoji is no base
    let mut l = Line::default();
    l.insert("a🏙 fog0");
    l.chips.insert("fog0".into());
    l.overlays.insert("fog0".into());
    assert_eq!(l.stacks(), vec![6..10]);
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

  #[test]
  fn sent_lines_come_back_behind_the_ones_sent_since() {
    let mut l = Line::default();
    for m in ["new one", "newer"] {
      l.insert(m);
      l.take();
    }
    l.restore_sent(vec!["old".into(), "older".into(), "new one".into()]);
    assert_eq!(l.sent().collect::<Vec<_>>(), ["old", "older", "new one", "newer"]);
    l.older();
    assert_eq!(l.text, "newer");
    // the cap holds across a restore
    let many: Vec<String> = (0..80).map(|i| i.to_string()).collect();
    l.restore_sent(many);
    assert_eq!(l.sent().count(), HISTORY);
    assert_eq!(l.sent().last(), Some("newer"));
  }

  // a draft's picks are the draft's: a sent line looked at meanwhile has none of them (it has its own, or
  // none), coming back to the draft brings them back, and a line that leaves or is loaded over takes them
  #[test]
  fn picks_stay_with_their_draft() {
    use crate::model::catalog::Rec;
    let rec = || Rec { url: "https://cdn.7tv.app/emote/A/1x.webp".into(), zero_width: false, provider: "7tv" };
    let mut l = Line::default();
    l.insert("W old");
    l.take();
    l.insert("W");
    l.picks.put("W", Some(rec()));
    l.older();
    assert_eq!((l.text.as_str(), l.picks.is_empty()), ("W old", true), "the sent line's W is a plain word");
    l.newer();
    assert_eq!((l.text.as_str(), l.picks.get("W").is_some()), ("W", true));
    l.take();
    assert!(l.picks.is_empty(), "sent: the pick went with it");
    l.picks.put("W", Some(rec()));
    l.load("other".into());
    assert!(l.picks.is_empty());
  }

  // a copy picked on purpose comes back with its line. it re-sent the room's emote of the name instead
  #[test]
  fn a_recalled_line_comes_back_with_the_copies_picked_in_it() {
    use crate::model::catalog::{Rec, Registry};
    let rec = |id: &str| Rec {
      url: format!("https://cdn.7tv.app/emote/{id}/1x.webp").into(),
      zero_width: false,
      provider: "7tv",
    };
    let picks = |id: &str| {
      let mut r = Registry::default();
      r.put("W", Some(rec(id)));
      r
    };
    let url = |l: &Line| l.picks.get("W").map(|r| r.url.to_string());
    let mut l = Line::default();
    for (line, id) in [("W one", Some("MINE")), ("plain", None), ("W two", Some("OTHER"))] {
      l.insert(line);
      let sent = l.take().unwrap();
      l.keep(&sent, id.map(picks).unwrap_or_default());
    }
    // a draft with a pick of its own is set aside, and each sent line shows what it went out with
    l.insert("W");
    l.picks.put("W", Some(rec("DRAFT")));
    l.older();
    assert_eq!((l.text.as_str(), url(&l).as_deref()), ("W two", Some("https://cdn.7tv.app/emote/OTHER/1x.webp")));
    l.older();
    assert_eq!((l.text.as_str(), url(&l)), ("plain", None));
    l.older();
    assert_eq!((l.text.as_str(), url(&l).as_deref()), ("W one", Some("https://cdn.7tv.app/emote/MINE/1x.webp")));
    l.newer();
    l.newer();
    assert_eq!((l.text.as_str(), url(&l).as_deref()), ("W two", Some("https://cdn.7tv.app/emote/OTHER/1x.webp")));
    l.newer();
    assert_eq!((l.text.as_str(), url(&l).as_deref()), ("W", Some("https://cdn.7tv.app/emote/DRAFT/1x.webp")));
    l.take();
    // the newest send of a line is the one that comes back: sent again by hand, it has no pick
    l.insert("W one");
    let sent = l.take().unwrap();
    l.keep(&sent, Registry::default());
    l.older();
    assert_eq!((l.text.as_str(), url(&l)), ("W one", None));
    l.newer();
    // picks are filed for the line just sent and no other
    l.keep("W two", picks("LATE"));
    l.keep("never sent", picks("LATE"));
    assert_eq!(l.kept.len(), 1, "W two's own, from its send");
    // a line that falls off the list takes its picks with it, and clearing forgets them all
    for i in 0..HISTORY {
      l.insert(&format!("m{i}"));
      l.take();
    }
    assert!(l.kept.is_empty());
    l.insert("W three");
    let sent = l.take().unwrap();
    l.keep(&sent, picks("MINE"));
    l.clear_sent();
    assert!(l.kept.is_empty());
  }

  #[test]
  fn clearing_forgets_everything_and_gives_back_the_draft() {
    let mut l = Line::default();
    l.insert("sent");
    l.take();
    l.insert("draft");
    l.older();
    assert_eq!(l.text, "sent");
    l.clear_sent();
    assert_eq!((l.text.as_str(), l.sent().count()), ("draft", 0));
    l.older();
    assert_eq!(l.text, "draft", "nothing to recall");
  }
}
