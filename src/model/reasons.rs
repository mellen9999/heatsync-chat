// the ext's ban reasons (setting modBanReasons): one reason per line, offered as chips when you ban. the
// account's text is hostile, so it is bounded: a value over the caps is refused whole, never half-taken
// (a half would be written back over the ext's)

// the ext's default maxLen for a text setting
pub const MAX_TEXT: usize = 4096;
pub const MAX_REASONS: usize = 20;
// twitch cuts a ban reason at 500
pub const MAX_ONE: usize = 500;

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Reasons {
  list: Vec<Box<str>>,
}

impl Reasons {
  pub fn list(&self) -> &[Box<str>] {
    &self.list
  }

  // the ext's text, lines joined by newlines
  pub fn text(&self) -> String {
    self.list.join("\n")
  }

  // the ext's text in: lines trimmed, blanks dropped, repeats kept once. false = over a cap, nothing changes
  pub fn set_text(&mut self, text: &str) -> bool {
    if text.chars().count() > MAX_TEXT {
      return false;
    }
    let mut list: Vec<Box<str>> = Vec::new();
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
      if l.chars().count() > MAX_ONE {
        return false;
      }
      if !list.iter().any(|x| **x == *l) {
        list.push(l.into());
      }
    }
    if list.len() > MAX_REASONS {
      return false;
    }
    self.list = list;
    true
  }

  // add / remove one. false = nothing changed (empty, repeat, full, too long, not there)
  pub fn reason(&mut self, add: bool, r: &str) -> bool {
    let r = r.trim();
    if r.is_empty() || r.chars().count() > MAX_ONE || r.contains(['\n', '\r']) {
      return false;
    }
    let at = self.list.iter().position(|x| **x == *r);
    match (add, at) {
      (true, None) if self.list.len() < MAX_REASONS && self.text().chars().count() + r.chars().count() < MAX_TEXT => {
        self.list.push(r.into())
      }
      (false, Some(i)) => _ = self.list.remove(i),
      _ => return false,
    }
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn text_round_trips_and_is_tidied() {
    let mut r = Reasons::default();
    assert!(r.set_text("spam\n  hate speech  \n\nspam\r\nbot"));
    assert_eq!(r.list().len(), 3);
    assert_eq!(r.text(), "spam\nhate speech\nbot");
    assert!(r.set_text(""));
    assert!(r.list().is_empty());
  }

  #[test]
  fn hostile_text_is_refused_whole() {
    let mut r = Reasons::default();
    assert!(r.set_text("keep"));
    let many: String = (0..=MAX_REASONS).map(|i| format!("r{i}\n")).collect();
    assert!(!r.set_text(&many));
    assert!(!r.set_text(&"x".repeat(MAX_ONE + 1)));
    assert!(!r.set_text(&format!("{}\n", "y".repeat(MAX_TEXT))));
    assert_eq!(r.text(), "keep", "refused values change nothing");
    assert!(r.set_text(&"x".repeat(MAX_ONE)));
  }

  #[test]
  fn add_and_remove_one() {
    let mut r = Reasons::default();
    assert!(r.reason(true, " spam "));
    assert!(!r.reason(true, "spam") && !r.reason(true, "") && !r.reason(true, "a\nb"));
    assert!(!r.reason(false, "nope"));
    assert!(r.reason(false, "spam") && r.list().is_empty());
    for i in 0..MAX_REASONS {
      assert!(r.reason(true, &format!("r{i}")));
    }
    assert!(!r.reason(true, "one more"));
  }
}
