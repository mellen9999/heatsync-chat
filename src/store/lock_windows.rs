// one window at a time: two copies would each save their own tabs and settings over the other's.
// a named mutex in this login session (`Local\`, so two people on one machine don't collide); windows drops
// it the moment the process ends, however it ends, so there is never a stale lock to clean up
use crate::util::win::{Handle, RESTART_VAR, wide};
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Threading::CreateMutexW;

const NAME: &str = "Local\\heatsync-chat";

// keep this alive for as long as the app runs
pub struct Held(#[allow(dead_code)] Handle);

const WAITS: u32 = 15;
const WAIT_MS: u64 = 200;

// Some(Err(())) = someone else holds it. None = we couldn't make a lock at all: run anyway
pub fn lock_instance() -> Option<Result<Held, ()>> {
  let restarted = std::env::var_os(RESTART_VAR).is_some();
  for n in 0..=WAITS {
    match take(NAME) {
      Some(Err(())) if restarted && n < WAITS => std::thread::sleep(std::time::Duration::from_millis(WAIT_MS)),
      r => return r,
    }
  }
  None
}

fn take(name: &str) -> Option<Result<Held, ()>> {
  let n = wide(name);
  // SAFETY: a plain named mutex; the name is null-terminated
  let h = unsafe { CreateMutexW(std::ptr::null(), 0, n.as_ptr()) };
  // read right away: the next call overwrites it
  let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
  let h = Handle::ok(h)?;
  if exists { Some(Err(())) } else { Some(Ok(Held(h))) }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_second_taker_is_told_and_the_lock_frees_with_the_first() {
    let name = format!("Local\\heatsync-chat-test-{}", std::process::id());
    let first = take(&name).unwrap().unwrap();
    assert!(take(&name).unwrap().is_err(), "held: the second launch must stand down");
    drop(first);
    assert!(take(&name).unwrap().is_ok(), "released with the first");
  }
}
