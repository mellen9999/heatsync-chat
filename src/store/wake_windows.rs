// the wake transport on windows: the running window serves a named pipe, one message per visit (message
// mode: the newcomer's whole "show ..." text is one message, so there is no "until it closes" to wait for).
// the pipe refuses other machines, and its default access list lets only this account write to it. every read
// is overlapped with a deadline and a size cap, so a silent or hostile client costs a second, never a hang.
// the thread sleeps in ConnectNamedPipe (idle cpu stays 0)
use super::wake::{DEADLINE, MAX_MSG, decode, encode};
use crate::util::win::{Handle, wide};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
  ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
  CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
  SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
  ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS,
  PIPE_TYPE_MESSAGE, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};

// one pipe end and the event its overlapped calls signal
pub struct Listener(Pipe);

struct Pipe {
  h: Handle,
  ev: Handle,
}

// `heatsync-chat-<account>`: each account on a shared machine has its own, and nothing odd in the name
fn pipe_name(user: &str) -> String {
  let safe: String = user.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(40).collect();
  format!("\\\\.\\pipe\\heatsync-chat-{}", if safe.is_empty() { "user" } else { &safe })
}

fn name() -> Vec<u16> {
  wide(&pipe_name(&std::env::var("USERNAME").unwrap_or_default()))
}

fn ms(d: std::time::Duration) -> u32 {
  d.as_millis().min(u128::from(u32::MAX - 1)) as u32
}

impl Pipe {
  fn new(h: Handle) -> Option<Pipe> {
    // SAFETY: a manual-reset event nobody else sees
    let ev = Handle::ok(unsafe { CreateEventW(null(), 1, 0, null()) })?;
    Some(Pipe { h, ev })
  }

  fn overlapped(&self) -> OVERLAPPED {
    // SAFETY: all-zero is the empty OVERLAPPED
    let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
    ov.hEvent = self.ev.0;
    ov
  }

  // an overlapped call that was just issued (`ok` is its return): how many bytes it moved, or the error
  // code. past `wait` ms it is cancelled and answered with the timeout code
  fn finish(&self, ov: &OVERLAPPED, ok: i32, wait: u32) -> Result<u32, u32> {
    if ok == 0 {
      // SAFETY: reading the thread's last error right after the call
      let e = unsafe { GetLastError() };
      if e != ERROR_IO_PENDING {
        return Err(e);
      }
      // SAFETY: our own event and pipe, and an OVERLAPPED that outlives the call
      unsafe {
        if WaitForSingleObject(self.ev.0, wait) != WAIT_OBJECT_0 {
          CancelIoEx(self.h.0, ov);
          let mut n = 0;
          GetOverlappedResult(self.h.0, ov, &mut n, 1);
          return Err(windows_sys::Win32::Foundation::WAIT_TIMEOUT);
        }
      }
    }
    let mut n = 0;
    // SAFETY: as above
    if unsafe { GetOverlappedResult(self.h.0, ov, &mut n, 0) } == 0 {
      return Err(unsafe { GetLastError() });
    }
    Ok(n)
  }

  fn read(&self, buf: &mut [u8], wait: std::time::Duration) -> Result<usize, u32> {
    let mut ov = self.overlapped();
    // SAFETY: buf lives through the call and finish() does not return before the read is over
    let ok = unsafe { ReadFile(self.h.0, buf.as_mut_ptr().cast(), buf.len() as u32, null_mut(), &mut ov) };
    self.finish(&ov, ok, ms(wait)).map(|n| n as usize)
  }

  fn write(&self, bytes: &[u8], wait: std::time::Duration) -> bool {
    let mut ov = self.overlapped();
    // SAFETY: as in read
    let ok = unsafe { WriteFile(self.h.0, bytes.as_ptr().cast(), bytes.len() as u32, null_mut(), &mut ov) };
    self.finish(&ov, ok, ms(wait)).is_ok_and(|n| n as usize == bytes.len())
  }
}

// the first instance of the pipe, made by the lock holder: if the name is already taken, someone else made it
// first, and nobody is to be woken through it
pub fn bind() -> Option<Listener> {
  let n = name();
  // SAFETY: a null-terminated name and default security
  let h = unsafe {
    CreateNamedPipeW(
      n.as_ptr(),
      PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
      PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
      1,
      4096,
      4096,
      0,
      null(),
    )
  };
  Pipe::new(Handle::ok(h)?).map(Listener)
}

// a thread that sleeps until someone connects; `on` gets each valid request and says whether the window
// took it. the thread ends with the process
pub fn serve(l: Listener, on: impl Fn(Vec<Box<str>>) -> bool + Send + 'static) {
  let _ = std::thread::Builder::new().name("wake".into()).spawn(move || {
    let p = l.0;
    loop {
      let mut ov = p.overlapped();
      // SAFETY: a connect on our own pipe; finish() waits for it
      let ok = unsafe { ConnectNamedPipe(p.h.0, &mut ov) };
      // a client that connected between our calls is already there
      // SAFETY: last error right after the call
      let connected = ok != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
      if !connected && p.finish(&ov, ok, INFINITE).is_err() {
        break;
      }
      visit(&p, &on);
      // SAFETY: our own pipe; the next connect reuses it
      unsafe { DisconnectNamedPipe(p.h.0) };
    }
  });
}

fn visit(p: &Pipe, on: &impl Fn(Vec<Box<str>>) -> bool) {
  let Some(open) = read_msg(p).and_then(|b| decode(&b)) else { return };
  if on(open) && p.write(b"ok\n", DEADLINE) {
    // the client closes after the ack; the pipe is not cut before it has read it
    let _ = p.read(&mut [0u8; 1], DEADLINE);
  }
}

// the one message, within the deadline and the size cap (one byte past it says it went over)
fn read_msg(p: &Pipe) -> Option<Vec<u8>> {
  let mut buf = vec![0u8; MAX_MSG + 1];
  match p.read(&mut buf, DEADLINE) {
    Ok(n) if n <= MAX_MSG => {
      buf.truncate(n);
      Some(buf)
    }
    Ok(_) => None,
    // too long for the buffer (ERROR_MORE_DATA), a client that hung up, a deadline: none of it is a request
    Err(_) => None,
  }
}

// true once the running window has taken it; false = nobody answered (an older version, no pipe)
pub fn ping(open: &[Box<str>]) -> bool {
  let n = name();
  let mut tries = 0;
  let h = loop {
    // identification only: a pipe someone else squatted on can't act as us
    // SAFETY: a null-terminated name; the handle is closed by Handle
    let h = unsafe {
      CreateFileW(
        n.as_ptr(),
        GENERIC_READ | GENERIC_WRITE,
        0,
        null(),
        OPEN_EXISTING,
        FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
        null_mut(),
      )
    };
    if let Some(h) = Handle::ok(h) {
      break h;
    }
    // the window is mid-way through someone else's visit: it frees up within the deadline
    // SAFETY: last error right after the call
    if tries < 3 && unsafe { GetLastError() } == ERROR_PIPE_BUSY {
      tries += 1;
      unsafe { WaitNamedPipeW(n.as_ptr(), ms(DEADLINE)) };
    } else {
      return false;
    }
  };
  let Some(p) = Pipe::new(h) else { return false };
  if !p.write(encode(open).as_bytes(), DEADLINE) {
    return false;
  }
  // the window may be mid-way through a slow client's deadline before it gets to us
  let mut ack = [0u8; 3];
  p.read(&mut ack, DEADLINE * 3).is_ok_and(|n| &ack[..n] == b"ok\n")
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::mpsc;
  use std::time::Duration;

  fn chans(v: &[&str]) -> Vec<Box<str>> {
    v.iter().map(|s| (*s).into()).collect()
  }

  #[test]
  fn the_pipe_name_is_plain() {
    assert_eq!(pipe_name("mellen"), "\\\\.\\pipe\\heatsync-chat-mellen");
    assert_eq!(pipe_name("a b\\c/d"), "\\\\.\\pipe\\heatsync-chat-abcd");
    assert_eq!(pipe_name(""), "\\\\.\\pipe\\heatsync-chat-user");
    assert!(pipe_name(&"x".repeat(500)).len() < 80);
  }

  // one test: the pipe has one name, so the cases share it (and the tests of this file never overlap)
  #[test]
  fn a_second_launch_reaches_the_first_and_hostile_clients_do_not() {
    assert!(!ping(&[]), "nobody listening yet");
    let l = bind().expect("first instance");
    assert!(bind().is_none(), "a second server on the same name is refused");
    let (tx, rx) = mpsc::channel();
    serve(l, move |o| tx.send(o).is_ok());
    assert!(ping(&chans(&["xqc", "kick:amouranth"])));
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), chans(&["xqc", "kick:amouranth"]));
    assert!(ping(&[]));
    assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().is_empty());
    // a client that says nothing holds the pipe for one deadline, then the next visitor is served
    let idle = std::fs::OpenOptions::new()
      .read(true)
      .write(true)
      .open(pipe_name(&std::env::var("USERNAME").unwrap_or_default()));
    assert!(ping(&[]), "served after the idle one times out");
    assert!(rx.recv_timeout(Duration::from_secs(4)).is_ok());
    drop(idle);
    // junk, an oversize message and a wrong shape are not requests
    use std::io::Write;
    let path = pipe_name(&std::env::var("USERNAME").unwrap_or_default());
    let big = format!("show\n{}", "open x\n".repeat(2000));
    for bad in [&b"hello\n"[..], b"\xff\xfe", b"", big.as_bytes(), b"open xqc\nshow\n"] {
      if let Ok(mut f) = std::fs::OpenOptions::new().read(true).write(true).open(&path) {
        let _ = f.write_all(bad);
      }
    }
    assert!(rx.recv_timeout(Duration::from_millis(500)).is_err(), "none of that is a request");
    assert!(ping(&[]), "and the window is still serving");
  }
}
