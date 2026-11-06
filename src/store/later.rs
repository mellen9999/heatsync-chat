// a write that can wait (settings: one per ctrl+wheel zoom step): the latest text for a file is kept
// in memory and written 500ms after the last change, on a thread of its own, so the ui never waits
// on a disk sync. the write itself is the store's usual one (tmp + fsync + rename, failures said).
// `flush` writes what is waiting right now: the app calls it on the way out and before a restart,
// so nothing is lost. the thread sleeps on a condvar: idle costs nothing
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, MutexGuard, Once, OnceLock, PoisonError};
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(500);

struct Job {
  dir: PathBuf,
  name: String,
  data: String,
}

struct Queue {
  jobs: Vec<Job>,
  // when the newest job arrived
  last: Option<Instant>,
}

struct Shared {
  q: Mutex<Queue>,
  wake: Condvar,
  // one writer at a time: a flush waits for a write already under way, so an older text never
  // lands after a newer one
  io: Mutex<()>,
}

fn shared() -> &'static Shared {
  static S: OnceLock<Shared> = OnceLock::new();
  S.get_or_init(|| Shared {
    q: Mutex::new(Queue { jobs: Vec::new(), last: None }),
    wake: Condvar::new(),
    io: Mutex::new(()),
  })
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
  m.lock().unwrap_or_else(PoisonError::into_inner)
}

// the folder is fixed now, on the caller's thread: the writer may not see what the caller sees
pub fn put(name: &str, data: &str) {
  let Some(dir) = super::dir() else { return drop(super::write(name, data)) };
  let s = shared();
  {
    let mut q = lock(&s.q);
    q.jobs.retain(|j| !(j.dir == dir && j.name == name));
    q.jobs.push(Job { dir, name: name.to_owned(), data: data.to_owned() });
    q.last = Some(Instant::now());
  }
  static START: Once = Once::new();
  START.call_once(|| {
    if std::thread::Builder::new().name("store-writer".into()).spawn(writer).is_err() {
      eprintln!("save: no thread for the settings writer; they are written when the app closes");
    }
  });
  s.wake.notify_one();
}

fn writer() {
  let s = shared();
  let mut q = lock(&s.q);
  loop {
    let Some(t) = q.last else {
      q = s.wake.wait(q).unwrap_or_else(PoisonError::into_inner);
      continue;
    };
    let left = (t + DEBOUNCE).saturating_duration_since(Instant::now());
    if left.is_zero() {
      drop(q);
      drain(None);
      q = lock(&s.q);
    } else {
      q = s.wake.wait_timeout(q, left).unwrap_or_else(PoisonError::into_inner).0;
    }
  }
}

// write what is waiting (only this folder's, when `only` is set)
fn drain(only: Option<&PathBuf>) {
  let s = shared();
  // io first, then the queue: whoever holds io takes the newest text
  let _io = lock(&s.io);
  let jobs = {
    let mut q = lock(&s.q);
    let (mine, rest): (Vec<Job>, Vec<Job>) =
      std::mem::take(&mut q.jobs).into_iter().partition(|j| only.is_none_or(|d| &j.dir == d));
    q.jobs = rest;
    if q.jobs.is_empty() {
      q.last = None;
    }
    mine
  };
  for j in jobs {
    super::write_job(&j.dir, &j.name, &j.data);
  }
}

pub fn flush() {
  if let Some(d) = super::dir() {
    drain(Some(&d));
  }
}

// the text still waiting to be written for this file, so a read sees the newest
pub fn pending(name: &str) -> Option<String> {
  let dir = super::dir()?;
  lock(&shared().q).jobs.iter().find(|j| j.dir == dir && j.name == name).map(|j| j.data.clone())
}

// the file is going away: a waiting text must not bring it back
pub fn cancel(name: &str) {
  if let Some(dir) = super::dir() {
    lock(&shared().q).jobs.retain(|j| !(j.dir == dir && j.name == name));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fresh() -> PathBuf {
    let d = super::super::dir().unwrap();
    let _ = std::fs::remove_dir_all(d.parent().unwrap());
    d
  }

  #[test]
  fn a_burst_of_changes_waits_then_lands_as_the_last_one() {
    let d = fresh();
    for z in [110, 120, 130, 140] {
      put("settings", &format!("zoom={z}\n"));
    }
    // not on disk yet, but a read already sees the newest text
    assert!(!d.join("settings").exists(), "the ui thread wrote nothing");
    assert_eq!(super::super::read("settings").as_deref(), Some("zoom=140\n"));
    // the writer thread gets there by itself
    let t = Instant::now();
    while !d.join("settings").exists() && t.elapsed() < Duration::from_secs(5) {
      std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(std::fs::read_to_string(d.join("settings")).unwrap(), "zoom=140\n");
    assert!(t.elapsed() >= Duration::from_millis(400), "it waited out the burst");
    assert!(pending("settings").is_none());
  }

  #[test]
  fn flush_writes_right_now_and_cancel_drops_a_waiting_text() {
    let d = fresh();
    put("settings", "zoom=150\n");
    flush();
    assert_eq!(std::fs::read_to_string(d.join("settings")).unwrap(), "zoom=150\n");
    assert!(!d.join(".settings.tmp").exists());
    put("settings", "zoom=90\n");
    cancel("settings");
    flush();
    assert_eq!(std::fs::read_to_string(d.join("settings")).unwrap(), "zoom=150\n");
  }

  #[test]
  fn a_direct_write_or_remove_beats_a_waiting_text() {
    let d = fresh();
    put("settings", "zoom=150\n");
    super::super::write("settings", "zoom=100\n").unwrap();
    flush();
    assert_eq!(std::fs::read_to_string(d.join("settings")).unwrap(), "zoom=100\n");
    put("settings", "zoom=150\n");
    super::super::remove("settings");
    flush();
    assert!(!d.join("settings").exists());
  }

  #[test]
  fn a_failed_write_is_said_once_like_any_other() {
    let d = fresh();
    std::fs::create_dir_all(d.parent().unwrap()).unwrap();
    std::fs::write(&d, "in the way").unwrap();
    put("settings", "zoom=150\n");
    flush();
    put("settings", "zoom=160\n");
    flush();
    let said = super::super::take_notes();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("couldn't save your settings"), "{said:?}");
  }
}
