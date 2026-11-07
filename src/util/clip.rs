// the system clipboard: text in and out, and a picture out. what is on each platform's side of it is in
// clip_unix.rs (wl-clipboard / xclip) and clip_windows.rs (the win32 clipboard). every call blocks, so
// callers run it off the ui thread (see net/clip.rs)
#[cfg_attr(unix, path = "clip_unix.rs")]
#[cfg_attr(windows, path = "clip_windows.rs")]
mod os;

// what one paste or copy may carry
pub const MAX_TEXT: usize = 1 << 20;
pub const MAX_IMAGE: usize = 20 << 20;

pub enum Paste {
  Text(String),
  // the picture's own bytes (png, jpeg, webp or gif)
  Image(Vec<u8>),
  Nothing,
}

#[derive(Debug, PartialEq)]
pub enum Fail {
  // neither a wayland nor an x11 display to ask
  #[cfg_attr(windows, allow(dead_code))]
  NoDisplay,
  // the tool isn't installed: the package that has it
  #[cfg_attr(windows, allow(dead_code))]
  Missing(&'static str),
  Timeout,
  TooBig(usize),
  // the tool ran and said no
  Tool(&'static str),
}

#[derive(Clone, Copy)]
pub enum Act {
  Copy,
  Paste,
}

impl Fail {
  // one plain status line: what went wrong and, if it is a missing tool, the command that fixes it
  pub fn said(&self, act: Act) -> String {
    let (ing, verb) = match act {
      Act::Copy => ("copying", "copy"),
      Act::Paste => ("pasting", "paste"),
    };
    match self {
      Fail::Missing(pkg) => format!("{ing} needs {pkg}: paru -S {pkg}"),
      Fail::NoDisplay => format!("couldn't {verb}: no display found"),
      Fail::Timeout => format!("couldn't {verb}: the clipboard didn't answer"),
      Fail::TooBig(n) => format!("couldn't {verb}: too big ({}MB max)", (n >> 20).max(1)),
      Fail::Tool(prog) => format!("couldn't {verb}: {prog} failed"),
    }
  }
}

// put text on the clipboard
pub fn copy(text: &str) -> Result<(), Fail> {
  if text.len() > MAX_TEXT {
    return Err(Fail::TooBig(MAX_TEXT));
  }
  os::copy(text)
}

// what is on the clipboard: text if there is some, else a picture
pub fn paste() -> Result<Paste, Fail> {
  os::paste()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn failures_say_plainly_what_happened() {
    assert_eq!(Fail::Timeout.said(Act::Paste), "couldn't paste: the clipboard didn't answer");
    assert_eq!(Fail::Tool("wl-copy").said(Act::Copy), "couldn't copy: wl-copy failed");
    assert_eq!(Fail::TooBig(MAX_IMAGE).said(Act::Paste), "couldn't paste: too big (20MB max)");
    assert_eq!(Fail::TooBig(MAX_TEXT).said(Act::Copy), "couldn't copy: too big (1MB max)");
    assert!(Fail::NoDisplay.said(Act::Copy).starts_with("couldn't copy"));
  }

  #[test]
  fn text_past_the_limit_is_not_sent() {
    let big = "x".repeat(MAX_TEXT + 1);
    assert_eq!(copy(&big), Err(Fail::TooBig(MAX_TEXT)));
  }
}
