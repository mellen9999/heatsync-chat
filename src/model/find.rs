// chat search (ext live-search.js): a plain substring over the name and the words, or `@name` to keep one
// person's lines (their name or login starts with it). case-insensitive for plain letters; the ext's
// /regex/ form is left out on purpose (no regex engine here, and a substring covers what chat needs)
use std::ops::Range;

// a term longer than this is cut: nobody types more, and it bounds the work per line
const MAX_TERM: usize = 128;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Find {
  // lowercased
  term: Box<str>,
  // `@name`: one sender, not text
  user: bool,
}

impl Find {
  // None = nothing to look for (empty, or a bare @)
  pub fn parse(q: &str) -> Option<Find> {
    let q = q.trim();
    let (user, t) = match q.strip_prefix('@') {
      Some(rest) => (true, rest.trim()),
      None => (false, q),
    };
    let term: String = t.chars().take(MAX_TERM).collect();
    (!term.is_empty()).then(|| Find { term: term.to_ascii_lowercase().into(), user })
  }

  // @name: the matches are people, so their names are what lights up
  pub fn user(&self) -> bool {
    self.user
  }

  pub fn test(&self, nick: &str, login: &str, body: &str) -> bool {
    let low = |s: &str| s.to_ascii_lowercase();
    if self.user {
      return low(nick).starts_with(&*self.term) || low(login).starts_with(&*self.term);
    }
    [nick, login, body].into_iter().any(|s| low(s).contains(&*self.term))
  }

  // where the term sits in `s`, for reverse video
  pub fn spans(&self, s: &str) -> Vec<Range<usize>> {
    s.to_ascii_lowercase().match_indices(&*self.term).map(|(i, m)| i..i + m.len()).collect()
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
    assert!(f.user() && &*f.term == "bo");
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
    assert_eq!(Find::parse(&"a".repeat(500)).unwrap().term.len(), MAX_TERM);
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
}
