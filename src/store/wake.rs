// a second launch wakes the first: the running window listens on a unix socket beside the lock, the
// newcomer says "show" (plus any channels it was given) and exits. only the lock holder binds, so a
// socket file left by a crashed run is always stale and is simply replaced. every byte read is hostile:
// the message is capped, the read has a deadline, and anything that isn't the expected shape is ignored
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

const NAME: &str = "heatsync-chat.sock";
const MAX_MSG: usize = 2048;
const MAX_OPEN: usize = 8;
const MAX_CHAN: usize = 100;
const DEADLINE: Duration = Duration::from_secs(1);

// "show\n" then "open <channel line>\n" for each channel the newcomer was given
fn encode(open: &[Box<str>]) -> String {
  let mut s = String::from("show\n");
  for c in open.iter().filter(|c| clean(c)).take(MAX_OPEN) {
    s.push_str(&format!("open {c}\n"));
  }
  s
}

fn clean(c: &str) -> bool {
  !c.is_empty() && c.len() <= MAX_CHAN && !c.chars().any(char::is_control)
}

// the channels asked for, or None when this isn't a "show" message at all
fn decode(bytes: &[u8]) -> Option<Vec<Box<str>>> {
  let mut lines = std::str::from_utf8(bytes).ok()?.lines();
  if lines.next()? != "show" {
    return None;
  }
  Some(lines.filter_map(|l| l.strip_prefix("open ")).filter(|c| clean(c)).take(MAX_OPEN).map(Into::into).collect())
}

// everything up to the sender closing, within a deadline and a size cap
fn read_msg(s: &mut UnixStream) -> Option<Vec<u8>> {
  let end = Instant::now() + DEADLINE;
  let (mut buf, mut chunk) = (Vec::new(), [0u8; 512]);
  loop {
    let left = end.checked_duration_since(Instant::now())?;
    s.set_read_timeout(Some(left.max(Duration::from_millis(1)))).ok()?;
    match s.read(&mut chunk) {
      Ok(0) => return Some(buf),
      Ok(n) => {
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_MSG {
          return None;
        }
      }
      Err(_) => return None,
    }
  }
}

// call only while holding the instance lock: whatever file is there is a dead run's
fn bind_at(p: &Path) -> Option<UnixListener> {
  let _ = std::fs::remove_file(p);
  let l = UnixListener::bind(p).ok()?;
  std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600)).ok()?;
  Some(l)
}

pub fn bind() -> Option<UnixListener> {
  bind_at(&super::lock::sibling(NAME)?)
}

// a thread that sleeps in accept (idle cpu stays 0); `on` gets each valid request and says whether
// the window took it. the thread ends with the process
pub fn serve(l: UnixListener, on: impl Fn(Vec<Box<str>>) -> bool + Send + 'static) {
  let _ = std::thread::Builder::new().name("wake".into()).spawn(move || {
    for mut s in l.incoming().flatten() {
      let Some(open) = read_msg(&mut s).and_then(|b| decode(&b)) else { continue };
      if on(open) {
        let _ = s.write_all(b"ok\n");
      }
    }
  });
}

fn ping_at(p: &Path, open: &[Box<str>]) -> bool {
  let go = || -> std::io::Result<bool> {
    let mut s = UnixStream::connect(p)?;
    s.set_write_timeout(Some(DEADLINE))?;
    s.write_all(encode(open).as_bytes())?;
    s.shutdown(std::net::Shutdown::Write)?;
    // the window may be mid-way through a slow client's deadline before it gets to us
    s.set_read_timeout(Some(DEADLINE * 3))?;
    let mut ack = [0u8; 3];
    s.read_exact(&mut ack)?;
    Ok(&ack == b"ok\n")
  };
  go().unwrap_or(false)
}

// true once the running window has taken it; false = nobody answered (an older version, no socket)
pub fn ping(open: &[Box<str>]) -> bool {
  super::lock::sibling(NAME).is_some_and(|p| ping_at(&p, open))
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::mpsc;

  fn tmp(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("hsc-wake-{tag}-{}", std::process::id()))
  }

  fn chans(v: &[&str]) -> Vec<Box<str>> {
    v.iter().map(|s| (*s).into()).collect()
  }

  #[test]
  fn lock_then_socket_round_trip() {
    let (lp, sp) = (tmp("rt-lock"), tmp("rt-sock"));
    let first = super::super::lock::take(&lp).unwrap().unwrap();
    assert!(super::super::lock::take(&lp).unwrap().is_err(), "second launch stands down");
    let (tx, rx) = mpsc::channel();
    serve(bind_at(&sp).unwrap(), move |o| tx.send(o).is_ok());
    assert!(ping_at(&sp, &chans(&["xqc", "kick:amouranth"])));
    assert_eq!(rx.recv_timeout(DEADLINE).unwrap(), chans(&["xqc", "kick:amouranth"]));
    assert!(ping_at(&sp, &[]));
    assert!(rx.recv_timeout(DEADLINE).unwrap().is_empty());
    drop(first);
    let _ = std::fs::remove_file(&lp);
    let _ = std::fs::remove_file(&sp);
  }

  #[test]
  fn hostile_messages_are_ignored() {
    let sp = tmp("hostile");
    let (tx, rx) = mpsc::channel();
    serve(bind_at(&sp).unwrap(), move |o| tx.send(o).is_ok());
    let big = format!("show\n{}", "open x\n".repeat(2000));
    for bad in [&b"hello\n"[..], b"\xff\xfe", b"", big.as_bytes(), b"open xqc\nshow\n"] {
      let mut s = UnixStream::connect(&sp).unwrap();
      let _ = s.write_all(bad);
      let _ = s.shutdown(std::net::Shutdown::Write);
    }
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "none of that is a request");
    // control bytes, empty lines and noise are dropped, not trusted
    let mut s = UnixStream::connect(&sp).unwrap();
    s.write_all(b"show\nopen bad\x1b[0m\nopen \nnonsense\nopen ok1\n").unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(rx.recv_timeout(DEADLINE).unwrap(), chans(&["ok1"]));
    // too many channels are capped on the way out and on the way in
    let many: Vec<Box<str>> = (0..20).map(|n| format!("c{n}").into()).collect();
    assert_eq!(encode(&many).lines().count(), 1 + MAX_OPEN);
    assert_eq!(decode(format!("show\n{}", "open c\n".repeat(20)).as_bytes()).unwrap().len(), MAX_OPEN);
    let _ = std::fs::remove_file(&sp);
  }

  #[test]
  fn a_silent_client_does_not_hold_the_listener() {
    let sp = tmp("slow");
    let (tx, rx) = mpsc::channel();
    serve(bind_at(&sp).unwrap(), move |o| tx.send(o).is_ok());
    let _idle = UnixStream::connect(&sp).unwrap();
    assert!(ping_at(&sp, &[]), "served after the idle one times out");
    assert!(rx.recv_timeout(Duration::from_secs(3)).is_ok());
    let _ = std::fs::remove_file(&sp);
  }

  #[test]
  fn the_socket_is_owner_only_and_a_stale_file_does_not_block() {
    let sp = tmp("stale");
    std::fs::write(&sp, b"left by a crash").unwrap();
    let l = bind_at(&sp).expect("a stale file is replaced");
    assert_eq!(std::fs::metadata(&sp).unwrap().permissions().mode() & 0o777, 0o600);
    // a dead run's socket (bound, never cleaned up) is replaced too
    drop(l);
    assert!(UnixStream::connect(&sp).is_err());
    assert!(bind_at(&sp).is_some());
    let _ = std::fs::remove_file(&sp);
  }

  #[test]
  fn nobody_listening_is_false() {
    assert!(!ping_at(&tmp("nobody"), &[]));
  }
}
