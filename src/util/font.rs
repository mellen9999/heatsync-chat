// which installed font a family name means. unix asks fontconfig's `fc-match` (font_unix.rs); windows has no
// tool to ask: the name is checked here and the font list itself (DirectWrite, through fontique) answers when
// the name reaches the text engine (font_windows.rs, app/font.rs). both come back as a `Found`, or one plain
// sentence for the status bar. it blocks (unix), so callers run it off the ui thread (see net/font.rs)
use std::path::PathBuf;

#[cfg_attr(unix, path = "font_unix.rs")]
#[cfg_attr(windows, path = "font_windows.rs")]
mod os;

pub use os::find;

// the longest family name taken (the ext's customFontName is 64 too)
pub const MAX_NAME: usize = 64;

// the font a name means: its family, and the file it lives in when the lookup knows one. without a file
// (windows) the text engine must find the family in the system's own list, and says so when it can't
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Found {
  pub family: String,
  pub file: Option<PathBuf>,
}

// a typed name that is worth looking up: not empty, short enough to keep as a setting, no control characters
#[cfg(any(windows, test))]
fn askable(name: &str) -> Result<&str, String> {
  let n = name.trim();
  if n.is_empty() || n.len() > MAX_NAME || n.chars().any(char::is_control) {
    return Err(crate::i18n::t("misc.font.bad_name").to_owned());
  }
  Ok(n)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_name_is_kept_only_when_it_could_be_a_setting() {
    assert_eq!(askable("  Fira Code "), Ok("Fira Code"));
    assert_eq!(askable("日本語 Mono"), Ok("日本語 Mono"));
    assert!(askable("   ").is_err());
    assert!(askable("a\u{1b}[0m").is_err());
    assert!(askable(&"a".repeat(MAX_NAME)).is_ok());
    assert!(askable(&"a".repeat(MAX_NAME + 1)).is_err());
  }
}
