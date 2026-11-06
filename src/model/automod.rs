// client automod (ext multichat/automod.js): a line that is mostly shouting is hidden, and the regex helpers
// the filter rules use. a pattern now is a hide rule (model/rules.rs). your own lines are never touched; mods and the broadcaster are not exempt (the ext
// only spares your own). the ext guards its regexes against catastrophic backtracking with a heuristic and
// falls back to a literal match; rust's regex is linear-time, so the only guard left is size: a pattern that
// is too long or too big to compile is matched as plain text instead, never dropped
use regex::{Regex, RegexBuilder};

// the ext reads at most this much of a line against patterns
pub const SCAN: usize = 256;
// what the ext treats as too long to trust as a regex
const MAX_REGEX: usize = 512;

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

  fn hits(src: &str, text: &str) -> bool {
    safe_regex(src, true).is_some_and(|re| re.is_match(scan(text)))
  }

  #[test]
  fn invalid_patterns_become_text_and_the_rest_keep_working() {
    // unbalanced paren, a js-only lookbehind and a backreference: none is a rust regex
    assert!(hits("oh(no", "well OH(NO then"));
    assert!(hits("(?<=a)b", "x (?<=a)b y"));
    assert!(hits("(a)\\1", "(a)\\1"));
    assert!(!hits("oh(no", "ohno"));
    assert!(hits(r"buy \d+ followers", "BUY 500 Followers now"));
  }

  #[test]
  fn catastrophic_shapes_are_linear() {
    let t = std::time::Instant::now();
    for p in ["(a+)+$", "(a|a)+b", "(.*)*x"] {
      assert!(!hits(p, &format!("{}!", "a".repeat(250))));
    }
    assert!(t.elapsed().as_millis() < 100);
  }

  #[test]
  fn huge_or_too_long_patterns_are_text() {
    // over 512 characters: plain text, so the | and . in it are not regex
    let long = format!("a.c|{}", "b".repeat(520));
    assert!(!hits(&long, "abc") && !hits(&long, "a.c"));
    // a repeat too big to compile falls back to text, never panics
    assert!(hits("x{9999}{9999}", "x{9999}{9999}"));
    // only the first 256 characters are read
    assert!(!hits("needle", &format!("{}needle", "x".repeat(300))));
    assert!(hits("needle", &format!("needle{}", "x".repeat(300))));
  }
}
