// the system clipboard, through the tools every desktop has: wl-copy / wl-paste (wl-clipboard) on
// wayland, xclip on x11. each runs with an argument list (never a shell), a deadline and a size cap,
// and every call blocks, so callers run it off the ui thread (see net/clip.rs). what is asked for is
// always one of our own mime names picked from the owner's list, never a string the owner sent
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

// what one paste or copy may carry
pub const MAX_TEXT: usize = 1 << 20;
pub const MAX_IMAGE: usize = 20 << 20;
// the owner's list of what it offers, and a tool's error line: a few lines each
const MAX_LIST: usize = 16 << 10;
const MAX_ERR: usize = 512;

const LIST_WAIT: Duration = Duration::from_secs(2);
const TEXT_WAIT: Duration = Duration::from_secs(3);
const IMAGE_WAIT: Duration = Duration::from_secs(10);
const COPY_WAIT: Duration = Duration::from_secs(3);

// utf-8 text names, best first (x11's STRING is latin-1, so it is not here)
const TEXT: [&str; 3] = ["UTF8_STRING", "text/plain;charset=utf-8", "text/plain"];
// pictures heatsync takes; the upload checks the bytes again
const IMAGE: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Backend {
  Wayland,
  X11,
}

pub enum Paste {
  Text(String),
  // the picture's own bytes (png, jpeg, webp or gif)
  Image(Vec<u8>),
  Nothing,
}

#[derive(Debug, PartialEq)]
pub enum Fail {
  // neither a wayland nor an x11 display to ask
  NoDisplay,
  // the tool isn't installed: the package that has it
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
      Fail::NoDisplay => format!("couldn't {verb}: no desktop to reach the clipboard of"),
      Fail::Timeout => format!("couldn't {verb}: the clipboard didn't answer"),
      Fail::TooBig(n) => format!("couldn't {verb}: too big ({}MB max)", (n >> 20).max(1)),
      Fail::Tool(prog) => format!("couldn't {verb}: {prog} failed"),
    }
  }
}

// a program and its arguments, as a list
#[derive(Debug, PartialEq)]
struct Tool {
  prog: &'static str,
  args: Vec<&'static str>,
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
      Backend::Wayland => Tool { prog: "wl-copy", args: vec!["--type", "text/plain;charset=utf-8"] },
      Backend::X11 => Tool { prog: "xclip", args: vec!["-selection", "clipboard", "-target", "UTF8_STRING", "-in"] },
    }
  }

  // what the clipboard offers, one name per line
  fn offered(self) -> Tool {
    match self {
      Backend::Wayland => Tool { prog: "wl-paste", args: vec!["--list-types"] },
      Backend::X11 => Tool { prog: "xclip", args: vec!["-selection", "clipboard", "-target", "TARGETS", "-out"] },
    }
  }

  // the clipboard as that one type, raw
  fn read(self, ty: &'static str) -> Tool {
    match self {
      Backend::Wayland => Tool { prog: "wl-paste", args: vec!["--no-newline", "--type", ty] },
      Backend::X11 => Tool { prog: "xclip", args: vec!["-selection", "clipboard", "-target", ty, "-out"] },
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

struct Ran {
  out: Vec<u8>,
  err: Vec<u8>,
  ok: bool,
}

// at most `cap` + 1 bytes: one more than allowed says it went over
fn take(r: impl Read, cap: usize) -> Vec<u8> {
  let mut buf = Vec::new();
  let _ = r.take(cap as u64 + 1).read_to_end(&mut buf);
  buf
}

fn reap(child: &mut Child) {
  let _ = child.kill();
  let _ = child.wait();
}

// run a program with a list of arguments, feed it `input`, and read its stdout (at most `cap` bytes)
// and a line of stderr, all before `wait` is up. `cap` None = it forks to serve a copy and keeps its
// pipes, so none are read. a program that is too slow or too chatty is killed
fn run(tool: &Tool, input: Option<&[u8]>, cap: Option<usize>, wait: Duration, pkg: &'static str) -> Result<Ran, Fail> {
  let piped = |on: bool| if on { Stdio::piped() } else { Stdio::null() };
  let mut child = Command::new(tool.prog)
    .args(&tool.args)
    .stdin(piped(input.is_some()))
    .stdout(piped(cap.is_some()))
    .stderr(piped(cap.is_some()))
    .spawn()
    .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { Fail::Missing(pkg) } else { Fail::Tool(tool.prog) })?;
  let deadline = Instant::now() + wait;
  let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
  let input = input.map(<[u8]>::to_vec);
  let (tx, rx) = mpsc::channel();
  std::thread::spawn(move || {
    if let (Some(mut w), Some(b)) = (stdin, input) {
      // a tool that quit early closes the pipe: its exit says so
      let _ = w.write_all(&b);
    }
    let max = cap.unwrap_or(0);
    let out = stdout.map(|r| take(r, max)).unwrap_or_default();
    // past the cap the tool is about to be killed: nothing more is waited for
    let err = if out.len() > max { Vec::new() } else { stderr.map(|r| take(r, MAX_ERR)).unwrap_or_default() };
    let _ = tx.send((out, err));
  });
  let left = |d: Instant| d.saturating_duration_since(Instant::now());
  let Ok((out, err)) = rx.recv_timeout(left(deadline)) else {
    reap(&mut child);
    return Err(Fail::Timeout);
  };
  if let Some(c) = cap.filter(|c| out.len() > *c) {
    reap(&mut child);
    return Err(Fail::TooBig(c));
  }
  loop {
    match child.try_wait() {
      Ok(Some(s)) => return Ok(Ran { out, err, ok: s.success() }),
      Ok(None) if !left(deadline).is_zero() => std::thread::sleep(Duration::from_millis(3)),
      Ok(None) => {
        reap(&mut child);
        return Err(Fail::Timeout);
      }
      Err(_) => return Err(Fail::Tool(tool.prog)),
    }
  }
}

// put text on the clipboard
pub fn copy(text: &str) -> Result<(), Fail> {
  if text.len() > MAX_TEXT {
    return Err(Fail::TooBig(MAX_TEXT));
  }
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

  fn tool(prog: &'static str, args: &[&'static str]) -> Tool {
    Tool { prog, args: args.to_vec() }
  }

  fn go(t: &Tool, input: Option<&[u8]>, cap: Option<usize>, wait: Duration) -> Result<Ran, Fail> {
    run(t, input, cap, wait, "coreutils")
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
    assert_eq!(e, Some(Fail::Missing("wl-clipboard")));
    assert_eq!(Fail::Missing("wl-clipboard").said(Act::Copy), "copying needs wl-clipboard: paru -S wl-clipboard");
    assert_eq!(Fail::Missing("xclip").said(Act::Paste), "pasting needs xclip: paru -S xclip");
  }

  #[test]
  fn failures_say_plainly_what_happened() {
    assert_eq!(Fail::Timeout.said(Act::Paste), "couldn't paste: the clipboard didn't answer");
    assert_eq!(Fail::Tool("wl-copy").said(Act::Copy), "couldn't copy: wl-copy failed");
    assert_eq!(Fail::TooBig(MAX_IMAGE).said(Act::Paste), "couldn't paste: too big (20MB max)");
    assert_eq!(Fail::TooBig(MAX_TEXT).said(Act::Copy), "couldn't copy: too big (1MB max)");
    assert!(Fail::NoDisplay.said(Act::Copy).starts_with("couldn't copy"));
  }

  #[test]
  fn input_goes_in_and_output_comes_back_whole() {
    let r = go(&tool("cat", &[]), Some("héllo\nwörld".as_bytes()), Some(MAX_TEXT), Duration::from_secs(5)).ok();
    let r = r.expect("cat ran");
    assert!(r.ok);
    assert_eq!(String::from_utf8_lossy(&r.out), "héllo\nwörld");
  }

  #[test]
  fn a_tool_that_says_no_is_not_ok_and_its_error_line_is_kept() {
    let r = go(&tool("cat", &["/hsc/no/such/file"]), None, Some(MAX_LIST), Duration::from_secs(5)).ok();
    let r = r.expect("cat ran");
    assert!(!r.ok);
    assert!(!r.err.is_empty() && r.out.is_empty());
  }

  #[test]
  fn output_past_the_cap_is_cut_off_and_refused() {
    let t = tool("yes", &[]);
    assert_eq!(go(&t, None, Some(100), Duration::from_secs(5)).err(), Some(Fail::TooBig(100)));
  }

  #[test]
  fn a_tool_that_never_answers_is_killed_at_the_deadline() {
    let t = tool("sleep", &["30"]);
    let start = Instant::now();
    assert_eq!(go(&t, None, Some(8), Duration::from_millis(200)).err(), Some(Fail::Timeout));
    assert!(start.elapsed() < Duration::from_secs(5));
  }

  #[test]
  fn a_copy_that_never_finishes_is_killed_too() {
    // cap None = nothing is read; the exit is what is waited for
    let t = tool("sleep", &["30"]);
    assert_eq!(go(&t, None, None, Duration::from_millis(200)).err(), Some(Fail::Timeout));
  }

  #[test]
  fn text_past_the_limit_is_not_sent() {
    let big = "x".repeat(MAX_TEXT + 1);
    assert_eq!(copy(&big), Err(Fail::TooBig(MAX_TEXT)));
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
