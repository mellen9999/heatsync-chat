// a short piece of text that lives inside the struct holding it, so `Settings` stays `Copy`. it is set from
// a &str, trimmed; one that is too long or holds a control character (a line break would split the settings
// file) is refused whole, never cut or cleaned
#[derive(Clone, Copy)]
pub struct Inline<const N: usize> {
  len: u8,
  buf: [u8; N],
}

impl<const N: usize> Inline<N> {
  const FITS: () = assert!(N <= u8::MAX as usize, "the length is one byte");

  pub const fn new() -> Self {
    let () = Self::FITS;
    Inline { len: 0, buf: [0; N] }
  }

  pub fn as_str(&self) -> &str {
    std::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or_default()
  }

  pub fn is_empty(&self) -> bool {
    self.len == 0
  }

  // false = refused, nothing changed
  pub fn set(&mut self, s: &str) -> bool {
    let s = s.trim();
    if s.len() > N || s.chars().any(char::is_control) {
      return false;
    }
    self.buf = [0; N];
    self.buf[..s.len()].copy_from_slice(s.as_bytes());
    self.len = s.len() as u8;
    true
  }
}

impl<const N: usize> Default for Inline<N> {
  fn default() -> Self {
    Self::new()
  }
}

impl<const N: usize> PartialEq for Inline<N> {
  fn eq(&self, o: &Self) -> bool {
    self.as_str() == o.as_str()
  }
}

impl<const N: usize> Eq for Inline<N> {}

impl<const N: usize> std::fmt::Debug for Inline<N> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::fmt::Debug::fmt(self.as_str(), f)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn text_goes_in_trimmed_and_comes_out_the_same() {
    let mut t = Inline::<16>::new();
    assert!(t.is_empty() && t.as_str().is_empty());
    assert!(t.set("  Fira Code "));
    assert_eq!(t.as_str(), "Fira Code");
    assert!(!t.is_empty());
    assert!(t.set("日本語"));
    assert_eq!(t.as_str(), "日本語");
    assert!(t.set(""));
    assert!(t.is_empty());
  }

  #[test]
  fn too_long_or_with_a_control_character_is_refused_and_changes_nothing() {
    let mut t = Inline::<8>::new();
    assert!(t.set("12345678"), "exactly the room");
    assert!(!t.set("123456789"));
    assert!(!t.set("日本語"), "nine bytes, not three letters");
    assert!(!t.set("a\nb"));
    assert!(!t.set("a\u{1b}[0m"));
    assert!(!t.set("a\tb"));
    assert_eq!(t.as_str(), "12345678");
  }

  #[test]
  fn a_shorter_text_leaves_nothing_behind_and_equals_the_same_text() {
    let (mut a, mut b) = (Inline::<8>::new(), Inline::<8>::new());
    assert!(a.set("longword") && a.set("hi"));
    assert!(b.set("hi"));
    assert_eq!(a, b);
    assert_ne!(a, Inline::new());
    assert_eq!(format!("{a:?}"), "\"hi\"");
  }

  #[test]
  fn it_is_copy() {
    let mut a = Inline::<8>::new();
    a.set("x");
    let b = a;
    a.set("y");
    assert_eq!((a.as_str(), b.as_str()), ("y", "x"));
  }
}
