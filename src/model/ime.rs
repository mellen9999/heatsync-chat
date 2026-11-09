// input-method state: the uncommitted composition (preedit) shown inline at the cursor.
// commits arrive only via the ime commit event; while composing, key text is ignored so
// nothing is inserted twice
const MAX_PREEDIT: usize = 256;

#[derive(Default)]
pub struct Ime {
  preedit: String,
  // caret inside the preedit (byte offset), when the input method gives one
  caret: Option<usize>,
}

impl Ime {
  pub fn composing(&self) -> bool {
    !self.preedit.is_empty()
  }

  pub fn preedit(&self) -> &str {
    &self.preedit
  }

  // byte offset of the caret within the preedit (end when the input method gave none)
  pub fn caret(&self) -> usize {
    self.caret.unwrap_or(self.preedit.len())
  }

  // an empty string ends the composition. control bytes become spaces, size is bounded
  pub fn set(&mut self, text: &str, cursor: Option<(usize, usize)>) {
    self.preedit.clear();
    self.preedit.extend(crate::util::blank(text));
    if self.preedit.len() > MAX_PREEDIT {
      let mut n = MAX_PREEDIT;
      while !self.preedit.is_char_boundary(n) {
        n -= 1;
      }
      self.preedit.truncate(n);
    }
    self.caret = cursor.map(|c| c.1).filter(|&i| i <= self.preedit.len() && self.preedit.is_char_boundary(i));
  }

  pub fn clear(&mut self) {
    self.preedit.clear();
    self.caret = None;
  }

  // a commit replaces the composition
  pub fn commit(&mut self, text: &str) -> String {
    self.clear();
    text.to_owned()
  }

  // plain key text is only trusted when no composition is in flight
  pub fn accepts_key_text(&self) -> bool {
    !self.composing()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn composing_blocks_key_text_until_commit() {
    let mut i = Ime::default();
    assert!(i.accepts_key_text());
    i.set("にほ", Some((0, 6)));
    assert!(i.composing() && !i.accepts_key_text());
    assert_eq!(i.caret(), 6);
    assert_eq!(i.commit("日本"), "日本");
    assert!(!i.composing() && i.accepts_key_text());
  }

  #[test]
  fn empty_preedit_ends_composition() {
    let mut i = Ime::default();
    i.set("a", None);
    i.set("", None);
    assert!(!i.composing());
  }

  #[test]
  fn hostile_input_is_bounded_and_clean() {
    let mut i = Ime::default();
    i.set(&"あ\n".repeat(500), Some((0, 99999)));
    assert!(i.preedit().len() <= MAX_PREEDIT && !i.preedit().contains('\n'));
    assert_eq!(i.caret(), i.preedit().len());
    i.set("あい", Some((0, 1)));
    assert_eq!(i.caret(), 6);
  }
}
