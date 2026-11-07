// the marks in a feed post's words (ext social.js formatText, the part a vt320 can draw):
//   **bold** or __bold__  -> bold
//   *under* or _under_    -> underline. the ext italicises these; a vt320 has no italic, so (like a man page)
//                            it is underlined
// the ext has no escape and no underline syntax of its own; its strike, spoiler, code and greentext marks are
// left as typed here. pure and bounded: bytes in, bytes out, no regex
//
// the ext's rules, kept: the words between a pair are one or more characters on one line; pairs close
// leftmost and shortest; bold goes first, then `*`, then `_`, and each pass sees the markers of the one
// before as gone (they are tags there); a `*` or `_` next to another is no single marker; text inside a link
// is never marked. a marker with no partner stays as typed
//
// one deliberate difference: `_` and `__` count only at a word's edge (not straight after a letter or digit
// on the way in, not straight before one on the way out), so `snake_case_name`, `@some_user_name` and an
// emote called `a_b_c` keep their underscores (the ext would underline the middle and break the name)
use std::ops::Range;

// most marks one post draws; the markers past it stay as typed
pub const MAX_MARKS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
  Bold,
  Under,
}

// a styled run of the cleaned text, in bytes
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
  pub range: Range<u32>,
  pub style: Style,
}

// bytes the passes see as nothing: a marker an earlier pass took, and the inside of a link
const GONE: u8 = 1;
const LINK: u8 = 2;

fn word_byte(b: u8) -> bool {
  b.is_ascii_alphanumeric() || b >= 0x80
}

struct Scan {
  // the text as the passes see it (taken markers and links masked, same length)
  work: Vec<u8>,
  // marker bytes to cut, and the runs to style, both in the original text's offsets
  cut: Vec<Range<usize>>,
  runs: Vec<(Range<usize>, Style)>,
}

impl Scan {
  fn full(&self) -> bool {
    self.runs.len() >= MAX_MARKS
  }

  fn take(&mut self, open: Range<usize>, close: Range<usize>, style: Style) {
    self.work[open.clone()].fill(GONE);
    self.work[close.clone()].fill(GONE);
    self.runs.push((open.end..close.start, style));
    self.cut.push(open);
    self.cut.push(close);
  }

  // `**` or `__`: the first pair on a line, then the next after it
  fn double(&mut self, m: u8) {
    let w = self.work.clone();
    // `__` is a marker at a word's edge only
    let edge = |k: usize, line: &Range<usize>| m != b'_' || k == line.start || !word_byte(w[k - 1]);
    for line in lines(&w) {
      let mut i = line.start;
      while i + 1 < line.end && !self.full() {
        if !(w[i] == m && w[i + 1] == m && edge(i, &line)) {
          i += 1;
          continue;
        }
        let close = (i + 3..line.end.saturating_sub(1))
          .find(|&j| w[j] == m && w[j + 1] == m && (m != b'_' || j + 2 >= line.end || !word_byte(w[j + 2])));
        match close {
          Some(j) => {
            self.take(i..i + 2, j..j + 2, Style::Bold);
            i = j + 2;
          }
          // none on this line: none for a later opener either
          None => break,
        }
      }
    }
  }

  // a lone `*` or `_`: not beside another of its kind
  fn single(&mut self, m: u8) {
    let w = self.work.clone();
    let lone = |k: usize, line: &Range<usize>| {
      w[k] == m && (k == line.start || w[k - 1] != m) && (k + 1 >= line.end || w[k + 1] != m)
    };
    let before = |k: usize, line: &Range<usize>| m != b'_' || k == line.start || !word_byte(w[k - 1]);
    let after = |k: usize, line: &Range<usize>| m != b'_' || k + 1 >= line.end || !word_byte(w[k + 1]);
    for line in lines(&w) {
      let mut i = line.start;
      while i + 2 < line.end && !self.full() {
        if !(lone(i, &line) && before(i, &line)) {
          i += 1;
          continue;
        }
        match (i + 2..line.end).find(|&j| lone(j, &line) && after(j, &line)) {
          Some(j) => {
            self.take(i..i + 1, j..j + 1, Style::Under);
            i = j + 1;
          }
          None => break,
        }
      }
    }
  }
}

// the lines of `w`, newline excluded (a pair never crosses one)
fn lines(w: &[u8]) -> Vec<Range<usize>> {
  let mut out = Vec::new();
  let mut from = 0;
  for (k, &b) in w.iter().enumerate() {
    if b == b'\n' {
      out.push(from..k);
      from = k + 1;
    }
  }
  out.push(from..w.len());
  out
}

// the words with their markers cut, and the runs to style (offsets into the cut text). None = nothing to
// do: the words are drawn as they are
pub fn parse(text: &str) -> Option<(String, Vec<Mark>)> {
  if !text.bytes().any(|b| b == b'*' || b == b'_') {
    return None;
  }
  let mut s = Scan { work: text.as_bytes().to_vec(), cut: Vec::new(), runs: Vec::new() };
  for l in super::links::find(text) {
    s.work[l.range].fill(LINK);
  }
  s.double(b'*');
  s.double(b'_');
  s.single(b'*');
  s.single(b'_');
  if s.runs.is_empty() {
    return None;
  }
  s.cut.sort_by_key(|r| r.start);
  let mut out = String::with_capacity(text.len());
  let mut from = 0;
  for c in &s.cut {
    out.push_str(&text[from..c.start]);
    from = c.end;
  }
  out.push_str(&text[from..]);
  // an offset in the original text -> where it lands once the cut markers are gone
  let moved =
    |at: usize| at - s.cut.iter().take_while(|c| c.start < at).map(|c| c.end.min(at) - c.start).sum::<usize>();
  let marks = s
    .runs
    .iter()
    .map(|(r, style)| Mark { range: moved(r.start) as u32..moved(r.end) as u32, style: *style })
    .filter(|m| m.range.start < m.range.end)
    .collect();
  Some((out, marks))
}

#[cfg(test)]
mod tests {
  use super::*;

  // the cut text, and each run as (style letter, its text)
  fn see(text: &str) -> (String, Vec<(char, String)>) {
    let Some((out, marks)) = parse(text) else { return (text.to_owned(), Vec::new()) };
    let runs = marks
      .iter()
      .map(|m| {
        let c = if m.style == Style::Bold { 'b' } else { 'u' };
        (c, out[m.range.start as usize..m.range.end as usize].to_owned())
      })
      .collect();
    (out, runs)
  }

  fn run(c: char, s: &str) -> (char, String) {
    (c, s.to_owned())
  }

  fn plain(t: &str) -> (String, Vec<(char, String)>) {
    (t.to_owned(), Vec::new())
  }

  #[test]
  fn bold_and_underline_in_the_exts_two_spellings() {
    assert_eq!(see("a **b** c"), ("a b c".into(), vec![run('b', "b")]));
    assert_eq!(see("a __b__ c"), ("a b c".into(), vec![run('b', "b")]));
    assert_eq!(see("a *b* c"), ("a b c".into(), vec![run('u', "b")]));
    assert_eq!(see("a _b_ c"), ("a b c".into(), vec![run('u', "b")]));
    // more than a word, more than one
    assert_eq!(
      see("**one two** and *three four* **five**"),
      ("one two and three four five".into(), vec![run('b', "one two"), run('b', "five"), run('u', "three four")])
    );
  }

  #[test]
  fn text_with_no_pair_is_left_alone() {
    assert!(parse("plain words, 2 * 3").is_none());
    assert!(parse("").is_none());
    assert!(parse("no pair * here").is_none());
  }

  #[test]
  fn a_marker_without_a_partner_stays_as_typed() {
    for t in ["**bold", "bold**", "*under", "_under", "__x", "a ** b", "**", "****", "*", "_", "***", "*a **b"] {
      assert_eq!(see(t), plain(t), "{t}");
    }
    // an unclosed one after a good pair
    assert_eq!(see("**a **b**"), ("a b**".into(), vec![run('b', "a ")]));
  }

  #[test]
  fn a_pair_is_one_line_and_not_empty() {
    assert_eq!(see("**a\nb**"), plain("**a\nb**"), "never across a newline");
    assert_eq!(see("**a**\n**b**"), ("a\nb".into(), vec![run('b', "a"), run('b', "b")]));
    assert_eq!(see("a **** b"), plain("a **** b"), "nothing between");
    assert_eq!(see("a ** ** b"), ("a   b".into(), vec![run('b', " ")]), "a space is something, as in the ext");
  }

  #[test]
  fn nesting_is_flat_runs_not_a_tree() {
    assert_eq!(see("**a *b* c**"), ("a b c".into(), vec![run('b', "a b c"), run('u', "b")]));
    assert_eq!(see("***both***"), ("both".into(), vec![run('b', "both"), run('u', "both")]));
    assert_eq!(see("*a **b** c*"), ("a b c".into(), vec![run('b', "b"), run('u', "a b c")]));
  }

  #[test]
  fn underscores_keep_names_and_words() {
    for t in ["snake_case_name", "@some_user_name hi", "a_b_c and d_e", "x__y__z", "foo_bar_"] {
      assert_eq!(see(t), plain(t), "{t}");
    }
    assert_eq!(see("_start_ and end_"), ("start and end_".into(), vec![run('u', "start")]));
    assert_eq!(see("(_x_) and __init__."), ("(x) and init.".into(), vec![run('b', "init"), run('u', "x")]));
    // stars are not words: they mark inside one, as in the ext
    assert_eq!(see("2*3*4"), ("234".into(), vec![run('u', "3")]));
  }

  #[test]
  fn a_link_is_never_marked() {
    assert_eq!(
      see("see https://x.com/a_b_c now _ok_"),
      ("see https://x.com/a_b_c now ok".into(), vec![run('u', "ok")])
    );
    // a pair around a link takes it along, the link's own text untouched; a marker glued to it belongs to the link
    assert_eq!(
      see("**go https://x.com/a_b now**"),
      ("go https://x.com/a_b now".into(), vec![run('b', "go https://x.com/a_b now")])
    );
    // markers glued to a bare link bold it, the link's own text whole
    assert_eq!(see("**https://x.com/a**"), ("https://x.com/a".into(), vec![run('b', "https://x.com/a")]));
  }

  #[test]
  fn multibyte_text_keeps_its_offsets() {
    assert_eq!(see("é **日本** ü"), ("é 日本 ü".into(), vec![run('b', "日本")]));
    assert_eq!(see("日_本_"), plain("日_本_"), "a letter of any script is a word");
  }

  #[test]
  fn a_hostile_post_is_bounded() {
    // a wall of pairs: marks stop at the cap, the rest stays as typed
    let t = "*a* ".repeat(1000);
    let (out, marks) = parse(&t).unwrap();
    assert_eq!(marks.len(), MAX_MARKS);
    assert!(out.len() < t.len() && out.contains("*a*"));
    // nothing but markers, unbalanced or many, at the post size: quick, and every run inside the text
    let walls = ["*_".repeat(2000), "_".repeat(4000), "**__".repeat(1000), "* ".repeat(2000), "\n*".repeat(2000)];
    for t in walls {
      let started = std::time::Instant::now();
      let r = parse(&t);
      assert!(started.elapsed().as_millis() < 500, "slow");
      if let Some((out, marks)) = r {
        assert!(marks.len() <= MAX_MARKS);
        assert!(marks.iter().all(|m| (m.range.end as usize) <= out.len() && m.range.start < m.range.end));
      }
    }
  }
}
