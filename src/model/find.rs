// chat search (ext live-search.js): three forms, read from the query as typed (trimmed, case kept)
//   /pattern/ or /pattern/i  a regex over the name and the words (`/foo/bar` is plain text, as in the ext)
//   @name                    one person's lines (their name or login starts with it)
//   anything else            a substring over the name and the words
// plain forms ignore case. the ext degrades a dangerous or broken regex to plain text; here a broken one is
// flagged "bad pattern" and filters nothing, so a half-typed `/[/` never hides the chat
use std::ops::Range;

use regex::Regex;

// a term longer than this is cut: nobody types more, and it bounds the work per line
const MAX_TERM: usize = 128;
// the ext's cap on a regex (chars)
const MAX_REGEX: usize = 512;

#[derive(Clone, Debug)]
enum How {
  // lowercased
  Term(Box<str>),
  // lowercased `@name` prefix
  User(Box<str>),
  Re(Regex),
}

#[derive(Clone, Debug)]
pub struct Find(How);

impl PartialEq for Find {
  fn eq(&self, o: &Find) -> bool {
    match (&self.0, &o.0) {
      (How::Term(a), How::Term(b)) | (How::User(a), How::User(b)) => a == b,
      (How::Re(a), How::Re(b)) => a.as_str() == b.as_str(),
      _ => false,
    }
  }
}

// the ext's `^/(.+)/(i?)$`
fn slashes(q: &str) -> Option<(&str, bool)> {
  let inner = q.strip_prefix('/')?;
  let (src, ci) = match inner.strip_suffix("/i") {
    Some(src) if !src.is_empty() => (src, true),
    _ => (inner.strip_suffix('/')?, false),
  };
  (!src.is_empty()).then_some((src, ci))
}

impl Find {
  // Ok(None) = nothing to look for (empty, or a bare @); Err = a /regex/ that is too long, not valid or too
  // big. compiled once per query here, never per line. the regex engine is linear-time (no backtracking), so
  // the ext's ReDoS heuristics and timing probes have nothing to catch: the length cap and the size limits
  // are the whole guard
  pub fn compile(q: &str) -> Result<Option<Find>, ()> {
    let q = q.trim();
    if let Some((src, ci)) = slashes(q) {
      if src.chars().count() > MAX_REGEX {
        return Err(());
      }
      return crate::model::automod::build(src, ci).map(|re| Some(Find(How::Re(re)))).ok_or(());
    }
    let (user, t) = match q.strip_prefix('@') {
      Some(rest) => (true, rest.trim()),
      None => (false, q),
    };
    let term: String = t.chars().take(MAX_TERM).collect();
    let term: Box<str> = term.to_ascii_lowercase().into();
    Ok((!term.is_empty()).then(|| Find(if user { How::User(term) } else { How::Term(term) })))
  }

  // None = no search (nothing typed, or a bad pattern)
  #[cfg(test)]
  pub fn parse(q: &str) -> Option<Find> {
    Find::compile(q).ok().flatten()
  }

  // @name: the matches are people, so their names are what lights up
  pub fn user(&self) -> bool {
    matches!(self.0, How::User(_))
  }

  pub fn test(&self, nick: &str, login: &str, body: &str) -> bool {
    let low = |s: &str| s.to_ascii_lowercase();
    match &self.0 {
      How::User(t) => low(nick).starts_with(&**t) || low(login).starts_with(&**t),
      How::Term(t) => [nick, login, body].into_iter().any(|s| low(s).contains(&**t)),
      How::Re(re) => re.is_match(nick) || re.is_match(body),
    }
  }

  // where the term sits in `s`, for reverse video
  pub fn spans(&self, s: &str) -> Vec<Range<usize>> {
    match &self.0 {
      How::User(t) | How::Term(t) => s.to_ascii_lowercase().match_indices(&**t).map(|(i, m)| i..i + m.len()).collect(),
      How::Re(re) => re.find_iter(s).filter(|m| !m.is_empty()).map(|m| m.range()).collect(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_plain_term_matches_name_or_words_ignoring_case() {
    let f = Find::parse("  KaPPa ").unwrap();
    assert!(!f.user());
    assert!(f.test("bob", "bob", "so Kappa"));
    assert!(f.test("KappaFan", "kappafan", "hi"));
    assert!(!f.test("bob", "bob", "hello"));
  }

  #[test]
  fn at_name_keeps_one_senders_lines() {
    let f = Find::parse("@bo").unwrap();
    assert!(f.user());
    assert!(f.test("Bob", "bob", "anything"));
    assert!(f.test("Display", "bobby", "x"));
    // not a substring of the name, and never the words
    assert!(!f.test("rob", "rob", "bo"));
    assert!(!f.test("alice", "alice", "bob said"));
  }

  #[test]
  fn nothing_to_look_for_is_no_search() {
    assert!(Find::parse("").is_none());
    assert!(Find::parse("   ").is_none());
    assert!(Find::parse("@").is_none());
    assert!(Find::parse("@  ").is_none());
    assert_eq!(Find::parse(&"a".repeat(500)).unwrap(), Find::parse(&"a".repeat(MAX_TERM)).unwrap());
  }

  #[test]
  fn spans_find_every_hit_on_char_boundaries() {
    let f = Find::parse("ab").unwrap();
    assert_eq!(f.spans("xAB ab é Ab"), vec![1..3, 4..6, 10..12]);
    assert!(f.spans("nothing").is_empty());
    // multi-byte text around a hit stays valid
    let s = "日本 ab 語";
    let r = &f.spans(s)[0];
    assert_eq!(&s[r.clone()], "ab");
  }

  #[test]
  fn a_regex_matches_name_or_words() {
    let f = Find::parse("/fo+/").unwrap();
    assert!(f.test("bob", "bob", "a fooo b") && f.test("foo", "foo", "hi"));
    assert!(!f.test("bob", "bob", "f o") && !f.user());
    // case: kept unless the closing slash carries i
    assert!(Find::parse("/FOO/i").unwrap().test("x", "x", "a foo"));
    assert!(!Find::parse("/FOO/").unwrap().test("x", "x", "a foo"));
    // the login is not read by a regex (the ext's user/display name only)
    assert!(!Find::parse("/zed/").unwrap().test("bob", "zed", "hi"));
  }

  #[test]
  fn only_a_closed_slash_pair_is_a_regex() {
    // plain text, as in the ext
    for q in ["/foo/bar", "/foo", "//", "/", "/i", "//i"] {
      assert!(matches!(Find::parse(q).unwrap().0, How::Term(_)), "{q}");
    }
    assert!(matches!(Find::parse("/a/i").unwrap().0, How::Re(_)));
  }

  #[test]
  fn a_bad_pattern_is_flagged_and_filters_nothing() {
    assert!(Find::compile("/[/").is_err());
    assert!(Find::compile("/(a/i").is_err());
    assert!(Find::parse("/[/").is_none());
    // too long
    let long = format!("/{}/", "a".repeat(600));
    assert!(Find::compile(&long).is_err());
    assert!(Find::compile(&format!("/{}/", "a".repeat(512))).is_ok());
    // too big to compile
    assert!(Find::compile("/((a{100}){100}){100}/").is_err());
    // a quantified-quantifier is no blowup here
    let f = Find::parse("/(a+)+$/").unwrap();
    assert!(!f.test("x", "x", &format!("{}!", "a".repeat(5000))));
  }

  #[test]
  fn regex_spans_skip_empty_hits() {
    let f = Find::parse("/o+|x*/").unwrap();
    assert_eq!(f.spans("foo bo"), vec![1..3, 5..6]);
  }
}
