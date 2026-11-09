pub mod clip;
pub mod crash;
// one file per platform, same functions in each: the app never asks which os it is on
#[cfg_attr(unix, path = "dirs_unix.rs")]
#[cfg_attr(windows, path = "dirs_windows.rs")]
pub mod dirs;
pub mod font;
pub mod mem;
pub mod notify;
pub mod open;
#[cfg_attr(unix, path = "owner_unix.rs")]
#[cfg_attr(windows, path = "owner_windows.rs")]
pub mod owner;
#[cfg_attr(unix, path = "rss_unix.rs")]
#[cfg_attr(windows, path = "rss_windows.rs")]
pub mod rss;
pub mod sound;
// the unix halves (clipboard, font) run tools; windows asks the system directly
#[cfg(unix)]
pub mod spawn;
#[cfg_attr(unix, path = "stdio_unix.rs")]
#[cfg_attr(windows, path = "stdio_windows.rs")]
pub mod stdio;
pub mod time;
pub mod wav;
#[cfg(windows)]
pub mod win;

// `s` with every control character (tab, newline, escape, ...) as one space: text from outside stays one
// line. callers cut it by their own rule after (`.take(n)` chars, `clamp` bytes, ...)
pub fn blank(s: &str) -> impl Iterator<Item = char> + '_ {
  s.chars().map(|c| if c.is_control() { ' ' } else { c })
}

// `s` with every control character dropped
pub fn strip(s: &str) -> impl Iterator<Item = char> + '_ {
  s.chars().filter(|c| !c.is_control())
}

// at most `max` bytes of `s`, cut on a char boundary
pub fn clamp(s: &str, max: usize) -> &str {
  if s.len() <= max {
    return s;
  }
  let mut end = max;
  while !s.is_char_boundary(end) {
    end -= 1;
  }
  &s[..end]
}

// one url path segment, RFC 3986 unreserved bytes kept and everything else %XX
pub fn path_seg(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for b in s.bytes() {
    if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn control_characters_blank_or_go() {
    assert_eq!(blank("a\tb\nc\u{1b}d é").collect::<String>(), "a b c d é");
    assert_eq!(strip("a\tb\nc\u{1b}d é").collect::<String>(), "abcd é");
    assert_eq!(blank("a\u{85}b").collect::<String>(), "a b", "c1 controls too");
  }

  #[test]
  fn path_segments_keep_only_unreserved() {
    assert_eq!(path_seg("ab/cd+e=="), "ab%2Fcd%2Be%3D%3D");
    assert_eq!(path_seg("01ABC-x_y.z~"), "01ABC-x_y.z~");
  }
}
