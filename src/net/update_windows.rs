// the self-update's platform half on windows: the swap and the restart. a running exe can't
// be written over, but it can be renamed: the running file is moved aside, the new one renamed into its
// place, and the new copy started. if the second rename fails the first is undone, so the app is never missing
use super::exe;
use crate::util::win::RESTART_VAR;
use std::io::{Error, Result, Write};
use std::path::Path;
use std::time::Duration;

// there is no package manager's copy to leave alone: if the folder isn't ours to write (Program Files),
// the write probe in `self_updates` already says so
pub fn managed(_: &Path) -> bool {
  false
}

const NEW: &str = ".heatsync-chat.new";
const OLD: &str = ".heatsync-chat.old";
// antivirus can hold a new file for a moment: a rename is tried this many times, this far apart
const TRIES: u32 = 5;
const PAUSE: Duration = Duration::from_millis(200);

// the copy that was moved aside by an update is unlocked once its process is gone: this run removes it
pub fn tidy() {
  if let Some(d) = exe().and_then(Path::parent) {
    let _ = std::fs::remove_file(d.join(OLD));
    let _ = std::fs::remove_file(d.join(NEW));
  }
}

type Rename<'a> = &'a dyn Fn(&Path, &Path) -> Result<()>;

fn rename_again(ren: Rename, from: &Path, to: &Path, pause: Duration) -> Result<()> {
  let mut last = Ok(());
  for n in 0..TRIES {
    last = ren(from, to);
    if last.is_ok() {
      break;
    }
    if n + 1 < TRIES {
      std::thread::sleep(pause);
    }
  }
  last
}

// `exe` becomes the new bytes or stays as it was: written whole and flushed under a side name, the running
// file moved to `old`, the new one renamed in; a failed second step puts the first back
pub fn replace(exe: &Path, bin: &[u8]) -> Result<()> {
  let new = exe.with_file_name(NEW);
  let old = exe.with_file_name(OLD);
  swap(&|a, b| std::fs::rename(a, b), exe, &new, &old, bin, PAUSE)
}

fn swap(ren: Rename, exe: &Path, new: &Path, old: &Path, bin: &[u8], pause: Duration) -> Result<()> {
  // a leftover from a crashed update goes first, then create_new refuses to follow anything planted there
  let _ = std::fs::remove_file(new);
  let put = || -> Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(new)?;
    f.write_all(bin)?;
    f.sync_all()
  };
  put().inspect_err(|_| {
    let _ = std::fs::remove_file(new);
  })?;
  // an older swap's file is gone by now unless its process still runs: then there is nowhere to move this one
  let _ = std::fs::remove_file(old);
  if let Err(e) = rename_again(ren, exe, old, pause) {
    let _ = std::fs::remove_file(new);
    return Err(e);
  }
  if let Err(e) = rename_again(ren, new, exe, pause) {
    let back = rename_again(ren, old, exe, pause);
    let _ = std::fs::remove_file(new);
    return Err(match back {
      Ok(()) => e,
      Err(b) => Error::other(format!("{e}; and the old copy could not be put back: {b}")),
    });
  }
  Ok(())
}

// start the new copy and close this one, same arguments. only returns when the new copy couldn't start.
// the child waits a moment for this one's lock (see store::lock_instance)
pub fn restart() -> Error {
  let Some(p) = exe() else { return Error::other("no path") };
  match std::process::Command::new(p).args(std::env::args_os().skip(1)).env(RESTART_VAR, "1").spawn() {
    Ok(_) => {
      crate::util::notify::shutdown();
      std::process::exit(0)
    }
    Err(e) => e,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn folder(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("hsc-upd-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
  }

  #[test]
  fn the_asset_is_the_windows_exe() {
    assert_eq!(crate::net::update::asset_name(), "heatsync-chat-x86_64-windows.exe");
  }

  #[test]
  fn replace_swaps_the_file_whole_and_moves_the_old_one_aside() {
    let d = folder("swap");
    let exe = d.join("heatsync-chat.exe");
    std::fs::write(&exe, b"old").unwrap();
    // a stale half-written leftover from a crash is cleared, not tripped over
    std::fs::write(d.join(NEW), b"torn").unwrap();
    replace(&exe, b"new bytes").unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"new bytes");
    assert_eq!(std::fs::read(d.join(OLD)).unwrap(), b"old", "the old copy waits to be tidied");
    assert!(!d.join(NEW).exists(), "no side file left behind");
    // the next swap replaces the leftover too
    replace(&exe, b"newer").unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"newer");
    assert_eq!(std::fs::read(d.join(OLD)).unwrap(), b"new bytes");
    // a folder we can't write leaves the old app alone
    assert!(replace(&d.join("nope").join("heatsync-chat.exe"), b"x").is_err());
    let _ = std::fs::remove_dir_all(&d);
  }

  const QUICK: Duration = Duration::from_millis(1);

  fn real(a: &Path, b: &Path) -> Result<()> {
    std::fs::rename(a, b)
  }

  #[test]
  fn a_swap_that_fails_halfway_puts_the_old_app_back() {
    let d = folder("back");
    let exe = d.join("heatsync-chat.exe");
    std::fs::write(&exe, b"old").unwrap();
    // the new copy can never be renamed in (a scanner holding it, say)
    let (new, old) = (d.join(NEW), d.join(OLD));
    let locked = |from: &Path, to: &Path| {
      if from == new { Err(Error::other("in use")) } else { real(from, to) }
    };
    let e = swap(&locked, &exe, &new, &old, b"new", QUICK).unwrap_err();
    assert!(e.to_string().contains("in use"), "{e}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"old", "the app is where it was");
    assert!(!new.exists() && !old.exists(), "no side files left");
    let _ = std::fs::remove_dir_all(&d);
  }

  #[test]
  fn a_busy_file_is_tried_again_before_giving_up() {
    let d = folder("retry");
    let exe = d.join("heatsync-chat.exe");
    std::fs::write(&exe, b"old").unwrap();
    let (new, old) = (d.join(NEW), d.join(OLD));
    let calls = std::cell::Cell::new(0);
    let flaky = |from: &Path, to: &Path| {
      calls.set(calls.get() + 1);
      if from == new && calls.get() < 4 { Err(Error::other("busy")) } else { real(from, to) }
    };
    swap(&flaky, &exe, &new, &old, b"new", QUICK).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"new");
    let _ = std::fs::remove_dir_all(&d);
  }

  #[test]
  fn an_exe_that_cannot_move_stays_and_nothing_is_left() {
    let d = folder("stay");
    // no exe at all: the first rename fails, the new file is cleaned up
    let exe = d.join("heatsync-chat.exe");
    assert!(swap(&real, &exe, &d.join(NEW), &d.join(OLD), b"new", QUICK).is_err());
    assert!(!d.join(NEW).exists());
    assert!(!exe.exists());
    let _ = std::fs::remove_dir_all(&d);
  }

  #[test]
  fn tidy_removes_what_a_swap_left() {
    // exe() is the test binary's folder: only the two side names are touched
    let d = exe().and_then(Path::parent).unwrap().to_path_buf();
    std::fs::write(d.join(OLD), b"x").unwrap();
    tidy();
    assert!(!d.join(OLD).exists());
  }
}
