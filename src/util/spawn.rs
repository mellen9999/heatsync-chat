// running one outside program the safe way, for everything that asks the desktop's tools (the clipboard,
// the font lookup): an argument list and never a shell, a deadline, a cap on what it may print. every
// call blocks, so callers run it off the ui thread
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

// a tool's error line: a few lines at most
const MAX_ERR: usize = 512;

#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
  // the tool isn't installed: the package that has it
  Missing(&'static str),
  Timeout,
  TooBig(usize),
  // the tool ran and said no
  Tool(&'static str),
}

// a program and its arguments, as a list
#[derive(Debug, PartialEq)]
pub struct Tool {
  pub prog: &'static str,
  pub args: Vec<String>,
}

impl Tool {
  pub fn new(prog: &'static str, args: &[&str]) -> Tool {
    Tool { prog, args: args.iter().map(|a| (*a).to_owned()).collect() }
  }
}

pub struct Ran {
  pub out: Vec<u8>,
  // read by the clipboard tools on unix (to tell "empty" from "failed")
  #[cfg_attr(windows, allow(dead_code))]
  pub err: Vec<u8>,
  pub ok: bool,
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
pub fn run(
  tool: &Tool,
  input: Option<&[u8]>,
  cap: Option<usize>,
  wait: Duration,
  pkg: &'static str,
) -> Result<Ran, Fail> {
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

// the tools these run (cat, yes, sh) are unix
#[cfg(all(test, unix))]
mod tests {
  use super::*;

  fn go(t: &Tool, input: Option<&[u8]>, cap: Option<usize>, wait: Duration) -> Result<Ran, Fail> {
    run(t, input, cap, wait, "coreutils")
  }

  #[test]
  fn input_goes_in_and_output_comes_back_whole() {
    let r = go(&Tool::new("cat", &[]), Some("héllo\nwörld".as_bytes()), Some(1 << 20), Duration::from_secs(5)).ok();
    let r = r.expect("cat ran");
    assert!(r.ok);
    assert_eq!(String::from_utf8_lossy(&r.out), "héllo\nwörld");
  }

  #[test]
  fn a_tool_that_says_no_is_not_ok_and_its_error_line_is_kept() {
    let r = go(&Tool::new("cat", &["/hsc/no/such/file"]), None, Some(16 << 10), Duration::from_secs(5)).ok();
    let r = r.expect("cat ran");
    assert!(!r.ok);
    assert!(!r.err.is_empty() && r.out.is_empty());
  }

  #[test]
  fn output_past_the_cap_is_cut_off_and_refused() {
    let t = Tool::new("yes", &[]);
    assert_eq!(go(&t, None, Some(100), Duration::from_secs(5)).err(), Some(Fail::TooBig(100)));
  }

  #[test]
  fn a_tool_that_never_answers_is_killed_at_the_deadline() {
    let t = Tool::new("sleep", &["30"]);
    let start = Instant::now();
    assert_eq!(go(&t, None, Some(8), Duration::from_millis(200)).err(), Some(Fail::Timeout));
    assert!(start.elapsed() < Duration::from_secs(5));
  }

  #[test]
  fn a_copy_that_never_finishes_is_killed_too() {
    // cap None = nothing is read; the exit is what is waited for
    let t = Tool::new("sleep", &["30"]);
    assert_eq!(go(&t, None, None, Duration::from_millis(200)).err(), Some(Fail::Timeout));
  }

  #[test]
  fn a_missing_tool_names_its_package() {
    let e = run(&Tool::new("hsc-no-such-tool", &[]), None, Some(8), Duration::from_secs(1), "wl-clipboard").err();
    assert_eq!(e, Some(Fail::Missing("wl-clipboard")));
  }

  // an argument is one argument: no shell reads it, so these are plain text
  #[test]
  fn arguments_are_never_a_shell_line() {
    let r = go(&Tool::new("echo", &["$(id)", "; rm -rf ~", "`x`"]), None, Some(1 << 10), Duration::from_secs(5));
    assert_eq!(String::from_utf8_lossy(&r.expect("echo ran").out), "$(id) ; rm -rf ~ `x`\n");
  }
}
