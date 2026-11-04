// twitch refuses the same line twice in a row in a channel (msg_duplicate). the site, the ext and
// the tui all send the repeat with a trailing " U+E0000" (an invisible tag char), which twitch
// counts as different. it toggles: hi, hi+tag, hi. we never show the tag, ours or anyone's.
use std::collections::HashMap;

const TAG: &str = " \u{E0000}";

// what goes on the wire: the text, or the text plus the tag when it repeats the last wire text
pub fn wire(text: &str, last: Option<&str>) -> String {
  if last == Some(text) { format!("{text}{TAG}") } else { text.to_owned() }
}

// the text as people read it (and as echo matching sees it)
pub fn strip(text: &str) -> &str {
  text.strip_suffix(TAG).unwrap_or(text)
}

// the last wire text we sent, per channel
#[derive(Default)]
pub struct Dupes(HashMap<Box<str>, String>);

impl Dupes {
  pub fn wire(&mut self, chan: &str, text: &str) -> String {
    let w = wire(text, self.0.get(chan).map(String::as_str));
    match self.0.get_mut(chan) {
      Some(l) => l.clone_from(&w),
      None => {
        self.0.insert(chan.into(), w.clone());
      }
    }
    w
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn toggles() {
    let mut d = Dupes::default();
    assert_eq!(d.wire("a", "hi"), "hi");
    assert_eq!(d.wire("a", "hi"), "hi \u{E0000}");
    assert_eq!(d.wire("a", "hi"), "hi");
    assert_eq!(d.wire("a", "yo"), "yo");
    assert_eq!(d.wire("a", "hi"), "hi");
  }

  #[test]
  fn channels_are_independent() {
    let mut d = Dupes::default();
    assert_eq!(d.wire("a", "hi"), "hi");
    assert_eq!(d.wire("b", "hi"), "hi");
    assert_eq!(d.wire("a", "hi"), "hi \u{E0000}");
    assert_eq!(d.wire("b", "hi"), "hi \u{E0000}");
  }

  #[test]
  fn action_text_toggles_too() {
    let mut d = Dupes::default();
    assert_eq!(d.wire("a", "/me waves"), "/me waves");
    assert_eq!(d.wire("a", "/me waves"), "/me waves \u{E0000}");
  }

  #[test]
  fn strip_only_the_trailing_tag() {
    assert_eq!(strip("hi \u{E0000}"), "hi");
    assert_eq!(strip("hi"), "hi");
    assert_eq!(strip("a \u{E0000} b"), "a \u{E0000} b");
    assert_eq!(strip(&wire("hi", Some("hi"))), "hi");
  }
}
