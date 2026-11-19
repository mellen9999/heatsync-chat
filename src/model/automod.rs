// the regex helpers the filter rules use (the ext's multichat/automod.js, minus its all-caps check, which is a
// rule now). a pattern now is a hide rule (model/rules.rs). your own lines are never touched; mods and the broadcaster are not exempt (the ext
// only spares your own). the ext guards its regexes against catastrophic backtracking with a heuristic and
// falls back to a literal match; rust's regex is linear-time, so the only guard left is size: a pattern that
// is too long or too big to compile is matched as plain text instead, never dropped
use regex::{Regex, RegexBuilder};

// a user's own regex reads this much of a line, as in the ext: it holds the longest message of every site (500).
// the ext also answers a slow pattern from the first 256 only, because js cannot stop a regex once it runs; rust's
// is linear-time, so there is no slow pattern and no timing. keywords and `contains` read the whole line
pub const SCAN: usize = 512;
// what the ext treats as too long to trust as a regex
const MAX_REGEX: usize = 512;

// the first SCAN characters of a line, for a regex
pub fn scan(text: &str) -> &str {
  if text.len() <= SCAN {
    return text;
  }
  text.char_indices().nth(SCAN).map_or(text, |(i, _)| &text[..i])
}

// what a word is made of, for a keyword's edges: letters, digits, the underscore and combining marks, in any script
fn is_word(c: char) -> bool {
  c.is_alphanumeric()
    || c == '_'
    || matches!(c, '\u{300}'..='\u{36f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}' | '\u{20d0}'..='\u{20ff}' | '\u{fe20}'..='\u{fe2f}')
}

// a keyword is a whole word: on a side where it is itself a word character it may not run on into another, so
// "here" is in "(here)", "here-ish" and "here🥹" and not in "there" or "hereby". a keyword that starts or ends
// with a mark or an emoji needs no edge on that side (the ext's rule)
pub fn word_at(text: &str, val: &str) -> bool {
  let (pre, post) = (val.chars().next().is_some_and(is_word), val.chars().next_back().is_some_and(is_word));
  text.match_indices(val).any(|(i, _)| {
    (!pre || !text[..i].chars().next_back().is_some_and(is_word))
      && (!post || !text[i + val.len()..].chars().next().is_some_and(is_word))
  })
}

pub fn build(src: &str, ci: bool) -> Option<Regex> {
  RegexBuilder::new(src).case_insensitive(ci).size_limit(1 << 20).dfa_size_limit(1 << 20).nest_limit(40).build().ok()
}

// a pattern the rules run: one regex, or two that must both match. rust's regex has no lookahead, so the one
// shape the ext's own rules use, `^(?=A)B` (a single lookahead at the very start), runs as `^(?:A)` and
// `^(?:B)` both matching at the start of the line, which is the same thing. any other lookaround is not a
// regex here, as before
#[derive(Clone, Debug)]
pub struct Pat {
  re: Regex,
  and: Option<Regex>,
}

impl Pat {
  pub fn is_match(&self, text: &str) -> bool {
    self.re.is_match(text) && self.and.as_ref().is_none_or(|r| r.is_match(text))
  }
}

// where the lookahead at the start of `^(?=A)B` closes: A and B. None = not that shape (or a lookaround is nested
// in A). escapes and character classes are stepped over so a paren inside them does not count
fn split_lookahead(p: &str) -> Option<(&str, &str)> {
  let body = p.strip_prefix("^(?=")?;
  let b = body.as_bytes();
  let (mut i, mut depth, mut class) = (0, 0u32, false);
  while i < b.len() {
    match b[i] {
      b'\\' => i += 1,
      b']' if class => class = false,
      _ if class => {}
      b'[' => {
        class = true;
        // a leading ^ and a leading ] belong to the class
        if b.get(i + 1) == Some(&b'^') {
          i += 1;
        }
        if b.get(i + 1) == Some(&b']') {
          i += 1;
        }
      }
      b'(' => {
        depth += 1;
        let rest = &body[i + 1..];
        if rest.starts_with("?=") || rest.starts_with("?!") || rest.starts_with("?<=") || rest.starts_with("?<!") {
          return None;
        }
      }
      b')' if depth == 0 => return Some((&body[..i], &body[i + 1..])),
      b')' => depth -= 1,
      _ => {}
    }
    i += 1;
  }
  None
}

fn lookahead(p: &str, ci: bool) -> Option<Pat> {
  let (a, rest) = split_lookahead(p)?;
  Some(Pat { re: build(&format!("^(?:{a})"), ci)?, and: Some(build(&format!("^(?:{rest})"), ci)?) })
}

// a user pattern as a regex, or as plain text when it is too long or not valid. None only if even the
// plain text won't compile (cannot happen inside the size limits)
pub fn safe_regex(p: &str, ci: bool) -> Option<Pat> {
  let plain = |re| Pat { re, and: None };
  if p.chars().count() <= MAX_REGEX {
    if let Some(re) = build(p, ci) {
      return Some(plain(re));
    }
    if let Some(pat) = lookahead(p, ci) {
      return Some(pat);
    }
  }
  build(&regex::escape(p), ci).map(plain)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn hits(src: &str, text: &str) -> bool {
    safe_regex(src, true).is_some_and(|re| re.is_match(scan(text)))
  }

  #[test]
  fn a_keyword_ends_where_letters_and_digits_end() {
    for t in [
      "I was here\u{1f979}",
      "\u{1f979}here",
      "(here)",
      "here-ish",
      "here/there",
      "\u{ab}here\u{bb}",
      "here\u{2026}",
      "here",
    ] {
      assert!(word_at(t, "here"), "{t}");
    }
    for t in
      ["there", "hereby", "here2", "here_now", "where", "her\u{e8}", "\u{3053}\u{3053}here\u{3067}\u{3059}", "3here"]
    {
      assert!(!word_at(t, "here"), "{t}");
    }
    // a keyword that starts or ends with a mark or an emoji needs no edge on that side
    assert!(
      word_at("here\u{1f979}\u{1f979}", "\u{1f979}") && word_at("type!drop now", "!drop") && word_at("c++ is", "c++")
    );
    assert!(!word_at("!dropped", "!drop") && !word_at("abc++", "c++") && !word_at("axb", "a.b"));
    assert!(word_at(
      "\u{43d}\u{443} \u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}!",
      "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}"
    ));
    assert!(!word_at(
      "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}\u{438}\u{43a}",
      "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}"
    ));
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
    // a regex reads the first SCAN characters
    assert!(!hits("needle", &format!("{}needle", "x".repeat(SCAN + 40))));
    assert!(hits("needle", &format!("needle{}", "x".repeat(SCAN + 40))));
    assert!(hits("needle$", &format!("{}needle", "x ".repeat(247))));
  }
}
