// one window at a time: two copies would each save their own tabs and settings over the other's.
// an flock on a file in the runtime folder (else beside the settings); the system drops it the moment
// the process ends, however it ends, so there is never a stale lock to clean up
use std::fs::File;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

const NAME: &str = "heatsync-chat.lock";

// keep this alive for as long as the app runs
pub struct Held(#[allow(dead_code)] File);

fn path() -> Option<PathBuf> {
  std::env::var_os("XDG_RUNTIME_DIR")
    .map(PathBuf::from)
    .filter(|p| p.is_absolute() && p.is_dir())
    .map(|p| p.join(NAME))
    .or_else(|| super::ensure_dir().ok().map(|d| d.join(NAME)))
}

// Some(Err(())) = someone else holds it. None = we couldn't make a lock at all: run anyway (a broken
// runtime folder shouldn't keep the app from opening; the saves are atomic either way)
pub fn lock_instance() -> Option<Result<Held, ()>> {
  take(&path()?)
}

fn take(p: &std::path::Path) -> Option<Result<Held, ()>> {
  let f = File::options().create(true).truncate(false).write(true).mode(0o600).open(p).ok()?;
  // SAFETY: a plain flock on a descriptor we own and keep open
  let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
  if r == 0 {
    return Some(Ok(Held(f)));
  }
  match std::io::Error::last_os_error().raw_os_error() {
    Some(libc::EWOULDBLOCK) => Some(Err(())),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_second_taker_is_told_and_the_lock_frees_with_the_first() {
    let p = std::env::temp_dir().join(format!("hsc-lock-{}", std::process::id()));
    let first = take(&p).unwrap().unwrap();
    assert!(take(&p).unwrap().is_err(), "held: the second launch must stand down");
    drop(first);
    assert!(take(&p).unwrap().is_ok(), "released with the first");
    let _ = std::fs::remove_file(&p);
  }
}
