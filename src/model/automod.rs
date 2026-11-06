// client automod (ext multichat/automod.js): a line that is mostly shouting, or matches one of your
// patterns, is hidden. your own lines are never touched; mods and the broadcaster are not exempt (the ext
// only spares your own). the ext guards its regexes against catastrophic backtracking with a heuristic and
// falls back to a literal match; rust's regex is linear-time, so the only guard left is size: a pattern that
// is too long or too big to compile is matched as plain text instead, never dropped
use regex::{Regex, RegexBuilder};

// the ext reads at most this much of a line against patterns
pub const SCAN: usize = 256;
// the ext's maxLen for the pattern text, and what it treats as too long to trust as a regex
pub const MAX_TEXT: usize = 4096;
const MAX_REGEX: usize = 512;
const MAX_PATTERNS: usize = 64;

// the first SCAN characters of a line
pub fn scan(text: &str) -> &str {
  if text.len() <= SCAN {
    return text;
  }
  text.char_indices().nth(SCAN).map_or(text, |(i, _)| &text[..i])
}

pub fn build(src: &str, ci: bool) -> Option<Regex> {
  RegexBuilder::new(src).case_insensitive(ci).size_limit(1 << 20).dfa_size_limit(1 << 20).nest_limit(40).build().ok()
}

// a user pattern as a regex, or as plain text when it is too long or not valid. None only if even the
// plain text won't compile (cannot happen inside the size limits)
pub fn safe_regex(p: &str, ci: bool) -> Option<Regex> {
  Some(p).filter(|p| p.chars().count() <= MAX_REGEX).and_then(|p| build(p, ci)).or_else(|| build(&regex::escape(p), ci))
}

// the ext's all-caps rule: longer than 10, at least 8 letters (a-z), over 70% of them capitals
pub fn all_caps(text: &str) -> bool {
  if text.len() <= 10 || text.encode_utf16().count() <= 10 {
    return false;
  }
  let (mut letters, mut upper) = (0u32, 0u32);
  for b in text.bytes().filter(u8::is_ascii_alphabetic) {
    letters += 1;
    upper += u32::from(b.is_ascii_uppercase());
  }
  letters >= 8 && upper * 10 > letters * 7
}

#[derive(Default, Clone, Debug)]
pub struct Automod {
  // one pattern per entry, as typed
  pats: Vec<Box<str>>,
  // all of them in one regex; a pattern that is not valid is in it as plain text
  re: Option<Regex>,
}

impl Automod {
  pub fn patterns(&self) -> &[Box<str>] {
    &self.pats
  }

  // the ext's text setting: one pattern per line
  pub fn text(&self) -> String {
    self.pats.join("\n")
  }

  // the ext's text setting in, lines trimmed and blanks dropped. false = too much to take (nothing changes)
  pub fn set_text(&mut self, text: &str) -> bool {
    if text.chars().count() > MAX_TEXT {
      return false;
    }
    let pats: Vec<Box<str>> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(Into::into).collect();
    if pats.len() > MAX_PATTERNS {
      return false;
    }
    self.pats = pats;
    self.compile();
    true
  }

  // add / remove one pattern. false = nothing changed (empty, duplicate, full, not there)
  pub fn pattern(&mut self, add: bool, p: &str) -> bool {
    let p = p.trim();
    if p.is_empty() || p.contains(['\n', '\r']) {
      return false;
    }
    let at = self.pats.iter().position(|x| **x == *p);
    match (add, at) {
      (true, None) if self.pats.len() < MAX_PATTERNS && self.text().chars().count() + p.chars().count() < MAX_TEXT => {
        self.pats.push(p.into());
      }
      (false, Some(i)) => _ = self.pats.remove(i),
      _ => return false,
    }
    self.compile();
    true
  }

  fn compile(&mut self) {
    self.re = None;
    if self.pats.is_empty() {
      return;
    }
    // a pattern that won't compile alone is matched as text; the rest stay regexes
    let parts: Vec<String> = self
      .pats
      .iter()
      .map(|p| match p.chars().count() <= MAX_REGEX && build(p, true).is_some() {
        true => format!("(?:{p})"),
        false => regex::escape(p),
      })
      .collect();
    self.re = build(&parts.join("|"), true).or_else(|| {
      let all: Vec<String> = self.pats.iter().map(|p| regex::escape(p)).collect();
      build(&all.join("|"), true)
    });
  }

  // does a pattern match this line
  pub fn hits(&self, body: &str) -> bool {
    self.re.as_ref().is_some_and(|re| re.is_match(scan(body)))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn caps_thresholds_are_the_exts() {
    // 10 characters or fewer never counts
    assert!(!all_caps("HELLO THER"));
    assert!(all_caps("HELLO THERE FRIEND"));
    // fewer than 8 letters, however loud
    assert!(!all_caps("HEY YOU!!!!!!!!!!!!!"));
    // mostly lowercase
    assert!(!all_caps("ABCDEFG hijklm n o"));
  }

  #[test]
  fn caps_ratio_edges() {
    // 10 letters, 7 capitals = exactly 0.7: no
    assert!(!all_caps("ABCDEFGhij!!"));
    // 8 of 11 capitals: yes
    assert!(all_caps("ABCDEFGHijk!!"));
    // digits and punctuation are not letters
    assert!(all_caps("ABCDEFGH 1234567890 !!!!"));
    // non-ascii letters don't count either way
    assert!(!all_caps("ÀÉÎÕÜ ÀÉÎÕÜ ÀÉÎÕÜ"));
    // 11 utf-16 units counted, not bytes: 5 astral = 10 units
    assert!(!all_caps("😀😀😀😀😀"));
  }

  #[test]
  fn patterns_hit_case_insensitively() {
    let mut a = Automod::default();
    assert!(a.pattern(true, r"buy \d+ followers"));
    assert!(a.pattern(true, "free.*skins"));
    assert!(a.hits("BUY 500 Followers now"));
    assert!(a.hits("get FREE cs skins"));
    assert!(!a.hits("hello"));
    assert!(a.pattern(false, "free.*skins"));
    assert!(!a.hits("free skins"));
    assert!(!a.pattern(false, "free.*skins"));
  }

  #[test]
  fn invalid_patterns_become_text_and_the_rest_keep_working() {
    let mut a = Automod::default();
    // unbalanced paren, a js-only lookbehind and a backreference: none is a rust regex
    assert!(a.set_text("oh(no\n(?<=a)b\n(a)\\1\nplain"));
    assert!(a.hits("well OH(NO then"));
    assert!(a.hits("x (?<=a)b y"));
    assert!(a.hits("(a)\\1"));
    assert!(a.hits("a plain word"));
    assert!(!a.hits("ohno"));
  }

  #[test]
  fn catastrophic_shapes_are_linear() {
    let mut a = Automod::default();
    assert!(a.set_text("(a+)+$\n(a|a)+b\n(.*)*x"));
    let t = std::time::Instant::now();
    assert!(!a.hits(&format!("{}!", "a".repeat(250))));
    assert!(t.elapsed().as_millis() < 100);
  }

  #[test]
  fn huge_or_too_long_patterns_are_text() {
    let mut a = Automod::default();
    // over 512 characters: plain text, so the | and . in it are not regex
    assert!(a.pattern(true, &format!("a.c|{}", "b".repeat(520))));
    assert!(!a.hits("abc") && !a.hits("a.c"));
    // a repeat too big to compile falls back to text, never panics
    let mut b = Automod::default();
    assert!(b.pattern(true, "x{9999}{9999}"));
    assert!(b.hits("x{9999}{9999}"));
  }

  #[test]
  fn bounds() {
    let mut a = Automod::default();
    assert!(!a.set_text(&"x\n".repeat(MAX_PATTERNS + 1)));
    assert!(!a.set_text(&"y".repeat(MAX_TEXT + 1)));
    assert!(a.patterns().is_empty());
    assert!(!a.pattern(true, "  "));
    assert!(!a.pattern(true, "a\nb"));
    for i in 0..MAX_PATTERNS {
      assert!(a.pattern(true, &format!("p{i}")));
    }
    assert!(!a.pattern(true, "one more"));
    // only the first 256 characters are read
    let mut b = Automod::default();
    b.pattern(true, "needle");
    assert!(!b.hits(&format!("{}needle", "x".repeat(300))));
    assert!(b.hits(&format!("needle{}", "x".repeat(300))));
  }

  #[test]
  fn text_roundtrip() {
    let mut a = Automod::default();
    assert!(a.set_text("  one \n\n two\r\nthree"));
    assert_eq!(a.text(), "one\ntwo\nthree");
    assert!(a.set_text(""));
    assert!(a.patterns().is_empty() && !a.hits("one"));
  }
}
