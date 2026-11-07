// the system clipboard on unix, through the tools every desktop has: wl-copy / wl-paste (wl-clipboard) on
// wayland, xclip on x11. each runs with an argument list (never a shell), a deadline and a size cap,
// and every call blocks, so callers run it off the ui thread (see net/clip.rs). what is asked for is
// always one of our own mime names picked from the owner's list, never a string the owner sent
use std::time::Duration;

use super::{Fail, MAX_IMAGE, MAX_TEXT, Paste};
use crate::util::spawn::{self, Tool, run};

// the owner's list of what it offers: a few lines
const MAX_LIST: usize = 16 << 10;

const LIST_WAIT: Duration = Duration::from_secs(2);
const TEXT_WAIT: Duration = Duration::from_secs(3);
const IMAGE_WAIT: Duration = Duration::from_secs(10);
const COPY_WAIT: Duration = Duration::from_secs(3);

// utf-8 text names, best first (x11's STRING is latin-1, so it is not here)
const TEXT: [&str; 3] = ["UTF8_STRING", "text/plain;charset=utf-8", "text/plain"];
// pictures heatsync takes; the upload checks the bytes again
const IMAGE: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Backend {
  Wayland,
  X11,
}

impl From<spawn::Fail> for Fail {
  fn from(f: spawn::Fail) -> Fail {
    match f {
      spawn::Fail::Missing(pkg) => Fail::Missing(pkg),
      spawn::Fail::Timeout => Fail::Timeout,
      spawn::Fail::TooBig(n) => Fail::TooBig(n),
      spawn::Fail::Tool(prog) => Fail::Tool(prog),
    }
  }
}

impl Backend {
  fn of(wayland: Option<&str>, x11: Option<&str>) -> Option<Backend> {
    let set = |v: Option<&str>| v.is_some_and(|s| !s.is_empty());
    if set(wayland) {
      Some(Backend::Wayland)
    } else if set(x11) {
      Some(Backend::X11)
    } else {
      None
    }
  }

  // the same pick winit makes: wayland when there is a wayland display, else x11
  fn detect() -> Result<Backend, Fail> {
    let get = |k| std::env::var(k).ok();
    Backend::of(get("WAYLAND_DISPLAY").as_deref(), get("DISPLAY").as_deref()).ok_or(Fail::NoDisplay)
  }

  // the arch package that has this backend's tools
  fn pkg(self) -> &'static str {
    match self {
      Backend::Wayland => "wl-clipboard",
      Backend::X11 => "xclip",
    }
  }

  // text in on stdin; the tool forks and keeps serving it until something else is copied
  fn copy(self) -> Tool {
    match self {
      Backend::Wayland => Tool::new("wl-copy", &["--type", "text/plain;charset=utf-8"]),
      Backend::X11 => Tool::new("xclip", &["-selection", "clipboard", "-target", "UTF8_STRING", "-in"]),
    }
  }

  // what the clipboard offers, one name per line
  fn offered(self) -> Tool {
    match self {
      Backend::Wayland => Tool::new("wl-paste", &["--list-types"]),
      Backend::X11 => Tool::new("xclip", &["-selection", "clipboard", "-target", "TARGETS", "-out"]),
    }
  }

  // the clipboard as that one type, raw
  fn read(self, ty: &'static str) -> Tool {
    match self {
      Backend::Wayland => Tool::new("wl-paste", &["--no-newline", "--type", ty]),
      Backend::X11 => Tool::new("xclip", &["-selection", "clipboard", "-target", ty, "-out"]),
    }
  }
}

// the first of ours that the owner offers. exact names only, so what is asked for is ours
fn pick(list: &str, ours: &[&'static str]) -> Option<&'static str> {
  ours.iter().copied().find(|t| list.lines().any(|l| l.trim() == *t))
}

// a tool's "there is nothing on the clipboard" (wl-paste, xclip), as opposed to "couldn't ask"
fn empty(err: &str) -> bool {
  let e = err.to_ascii_lowercase();
  e.contains("nothing is copied") || e.contains("no selection") || e.contains("not available")
}

// put text on the clipboard (the size was checked by the caller)
pub fn copy(text: &str) -> Result<(), Fail> {
  let b = Backend::detect()?;
  let t = b.copy();
  let ran = run(&t, Some(text.as_bytes()), None, COPY_WAIT, b.pkg())?;
  if ran.ok { Ok(()) } else { Err(Fail::Tool(t.prog)) }
}

fn fetch(b: Backend, ty: &'static str, cap: usize, wait: Duration) -> Result<Vec<u8>, Fail> {
  let t = b.read(ty);
  let ran = run(&t, None, Some(cap), wait, b.pkg())?;
  if ran.ok { Ok(ran.out) } else { Err(Fail::Tool(t.prog)) }
}

// what is on the clipboard: text if there is some, else a picture
pub fn paste() -> Result<Paste, Fail> {
  let b = Backend::detect()?;
  let t = b.offered();
  let ran = run(&t, None, Some(MAX_LIST), LIST_WAIT, b.pkg())?;
  if !ran.ok {
    return if empty(&String::from_utf8_lossy(&ran.err)) { Ok(Paste::Nothing) } else { Err(Fail::Tool(t.prog)) };
  }
  let list = String::from_utf8_lossy(&ran.out);
  if let Some(ty) = pick(&list, &TEXT) {
    let text = String::from_utf8_lossy(&fetch(b, ty, MAX_TEXT, TEXT_WAIT)?).into_owned();
    if !text.trim().is_empty() {
      return Ok(Paste::Text(text));
    }
  }
  if let Some(ty) = pick(&list, &IMAGE) {
    let bytes = fetch(b, ty, MAX_IMAGE, IMAGE_WAIT)?;
    if !bytes.is_empty() {
      return Ok(Paste::Image(bytes));
    }
  }
  Ok(Paste::Nothing)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::util::clip::Act;

  use std::io::Write;
  use std::process::{Command, Stdio};

  fn tool(prog: &'static str, args: &[&str]) -> Tool {
    Tool::new(prog, args)
  }

  #[test]
  fn wayland_wins_when_both_displays_exist() {
    assert_eq!(Backend::of(Some("wayland-0"), Some(":0")), Some(Backend::Wayland));
    assert_eq!(Backend::of(None, Some(":0")), Some(Backend::X11));
    assert_eq!(Backend::of(Some(""), Some(":0")), Some(Backend::X11));
    assert_eq!(Backend::of(None, None), None);
    assert_eq!(Backend::of(Some(""), Some("")), None);
  }

  #[test]
  fn each_backend_builds_its_argument_lists() {
    assert_eq!(Backend::Wayland.copy(), tool("wl-copy", &["--type", "text/plain;charset=utf-8"]));
    assert_eq!(Backend::Wayland.offered(), tool("wl-paste", &["--list-types"]));
    assert_eq!(Backend::Wayland.read("image/png"), tool("wl-paste", &["--no-newline", "--type", "image/png"]));
    assert_eq!(Backend::X11.copy(), tool("xclip", &["-selection", "clipboard", "-target", "UTF8_STRING", "-in"]));
    assert_eq!(Backend::X11.offered(), tool("xclip", &["-selection", "clipboard", "-target", "TARGETS", "-out"]));
    assert_eq!(
      Backend::X11.read("image/png"),
      tool("xclip", &["-selection", "clipboard", "-target", "image/png", "-out"])
    );
  }

  #[test]
  fn packages_name_the_tools() {
    assert_eq!(Backend::Wayland.pkg(), "wl-clipboard");
    assert_eq!(Backend::X11.pkg(), "xclip");
  }

  #[test]
  fn the_pick_is_ours_from_the_owners_list() {
    let wl = "text/plain;charset=utf-8\ntext/plain\nUTF8_STRING\nSTRING\nTEXT\nimage/png\n";
    assert_eq!(pick(wl, &TEXT), Some("UTF8_STRING"));
    assert_eq!(pick(wl, &IMAGE), Some("image/png"));
    let x = "TARGETS\r\nTIMESTAMP\r\nSTRING\r\nimage/jpeg\r\n";
    assert_eq!(pick(x, &TEXT), None, "latin-1 STRING alone is not taken");
    assert_eq!(pick(x, &IMAGE), Some("image/jpeg"));
    assert_eq!(pick("text/plain\n", &TEXT), Some("text/plain"));
  }

  #[test]
  fn a_hostile_list_picks_nothing_of_its_own() {
    let evil = "--watch\n; rm -rf ~\ntext/plain-evil\nimage/png-x\n image/gif-less\n";
    assert_eq!(pick(evil, &TEXT), None);
    assert_eq!(pick(evil, &IMAGE), None);
    assert_eq!(pick("", &TEXT), None);
    // padding around a real name is only whitespace
    assert_eq!(pick("  image/webp \n", &IMAGE), Some("image/webp"));
  }

  #[test]
  fn an_empty_clipboard_is_told_apart_from_no_tool_answer() {
    assert!(empty("Nothing is copied\n"));
    assert!(empty("No selection"));
    assert!(empty("Error: target TARGETS not available\n"));
    assert!(!empty("Failed to connect to a Wayland server: No such file or directory\n"));
    assert!(!empty("Error: Can't open display: :9\n"));
    assert!(!empty(""));
  }

  #[test]
  fn a_missing_tool_names_its_package() {
    let e = run(&tool("hsc-no-such-tool", &[]), None, Some(8), Duration::from_secs(1), "wl-clipboard").err();
    assert_eq!(e.map(Fail::from), Some(Fail::Missing("wl-clipboard")));
    assert_eq!(Fail::Missing("wl-clipboard").said(Act::Copy), "copying needs wl-clipboard: paru -S wl-clipboard");
    assert_eq!(Fail::Missing("xclip").said(Act::Paste), "pasting needs xclip: paru -S xclip");
  }

  // needs wl-clipboard and a wayland session; it uses the real clipboard, so it puts back what was there:
  // WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000 cargo test --release -- --ignored live_wayland
  #[test]
  #[ignore]
  fn live_wayland_round_trip() {
    let before = match paste() {
      Ok(Paste::Text(t)) => Some(t),
      _ => None,
    };
    let text = "round trip \u{1f9ea} \u{e9}\u{e8}\nsecond line, `$(not run)` --type";
    copy(text).expect("copy");
    match paste().expect("paste") {
      Paste::Text(t) => assert_eq!(t, text),
      _ => panic!("expected the text back"),
    }
    // a picture someone else put there comes back byte for byte
    let png = b"\x89PNG\r\n\x1a\n....not a real picture, the clipboard doesn't look".to_vec();
    let mut c = Command::new("wl-copy")
      .args(["--type", "image/png"])
      .stdin(Stdio::piped())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()
      .expect("wl-copy");
    c.stdin.take().expect("stdin").write_all(&png).expect("write");
    assert!(c.wait().expect("wait").success());
    match paste().expect("paste") {
      Paste::Image(b) => assert_eq!(b, png),
      _ => panic!("expected the picture back"),
    }
    // an empty clipboard is nothing, not a failure
    Command::new("wl-copy").arg("--clear").status().expect("clear");
    assert!(matches!(paste(), Ok(Paste::Nothing)));
    if let Some(t) = before {
      let _ = copy(&t);
    }
  }
}
