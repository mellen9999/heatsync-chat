// small persisted state under $XDG_CONFIG_HOME/heatsync-chat, written atomically (tmp + rename).
// the folder is owner-only (0700) and so is every file in it (0600): the login token, notes and
// reading habits stay yours. nothing here fails quietly: a save that fails and a file that can't be
// read are both kept as a plain note for the ui to say once (`take_notes`)
mod lock;

pub use lock::lock_instance;

use std::io::{Error, ErrorKind, Result, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

#[cfg(not(test))]
fn dir() -> Option<PathBuf> {
  let base = std::env::var_os("XDG_CONFIG_HOME")
    .map(PathBuf::from)
    .filter(|p| p.is_absolute())
    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
  Some(base.join("heatsync-chat"))
}

// tests never touch the real config: each test thread gets its own scratch folder
#[cfg(test)]
fn dir() -> Option<PathBuf> {
  Some(scratch().join("heatsync-chat"))
}

// one test thread's own corner of the temp folder (the image cache keeps its scratch here too)
#[cfg(test)]
pub fn scratch() -> PathBuf {
  let t = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "");
  std::env::temp_dir().join("hsc-test").join(format!("{}-{t}", std::process::id()))
}

// what the store remembers between calls: sentences waiting for the ui, the files whose last save
// failed, and whether this run's tabs are for this run only
struct Notes {
  say: Vec<String>,
  failed: Vec<String>,
  pinned: bool,
}

const NO_NOTES: Notes = Notes { say: Vec::new(), failed: Vec::new(), pinned: false };

// a handful is plenty: a disk that is full fails every save the same way
const MAX_NOTES: usize = 8;

#[cfg(not(test))]
fn notes<R>(f: impl FnOnce(&mut Notes) -> R) -> R {
  use std::sync::{Mutex, PoisonError};
  static N: Mutex<Notes> = Mutex::new(NO_NOTES);
  f(&mut N.lock().unwrap_or_else(PoisonError::into_inner))
}

// one per test thread, like the folder
#[cfg(test)]
fn notes<R>(f: impl FnOnce(&mut Notes) -> R) -> R {
  thread_local!(static N: std::cell::RefCell<Notes> = const { std::cell::RefCell::new(NO_NOTES) });
  N.with(|n| f(&mut n.borrow_mut()))
}

fn note(s: String) {
  notes(|n| {
    if n.say.len() < MAX_NOTES {
      n.say.push(s);
    }
  });
}

// everything the ui has not said yet, oldest first
pub fn take_notes() -> Vec<String> {
  notes(|n| std::mem::take(&mut n.say))
}

fn what(name: &str) -> String {
  match name {
    "token" => "your login".into(),
    "tabs" => "your tabs".into(),
    n => format!("your {n}"),
  }
}

// a failed save is said once per file until that file saves again
fn save_failed(name: &str, e: &Error) {
  eprintln!("save {name}: {e}");
  notes(|n| {
    if n.failed.iter().any(|f| f == name) {
      return;
    }
    n.failed.push(name.to_owned());
    if n.say.len() < MAX_NOTES {
      n.say.push(format!("couldn't save {} ({}): changes may be lost when you quit", what(name), e.kind()));
    }
  });
}

fn saved(name: &str) {
  notes(|n| n.failed.retain(|f| f != name));
}

// the folder, made owner-only if it is missing or was left looser by an older version
fn ensure_dir() -> Result<PathBuf> {
  let d = dir().ok_or_else(|| Error::other("no home directory"))?;
  match std::fs::metadata(&d) {
    Ok(m) if !m.is_dir() => return Err(Error::new(ErrorKind::NotADirectory, "not a folder")),
    Ok(m) if m.permissions().mode() & 0o077 != 0 => std::fs::set_permissions(&d, PermissionsExt::from_mode(0o700))?,
    Ok(_) => {}
    Err(_) => std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&d)?,
  }
  Ok(d)
}

// at start: the folder and everything already in it go owner-only (files from before 0.3 were 0644)
pub fn init() {
  let Ok(d) = ensure_dir() else { return };
  let Ok(rd) = std::fs::read_dir(d) else { return };
  for e in rd.filter_map(std::result::Result::ok) {
    let Ok(m) = e.metadata() else { continue };
    if m.is_file() && m.permissions().mode() & 0o077 != 0 {
      let _ = std::fs::set_permissions(e.path(), PermissionsExt::from_mode(0o600));
    }
  }
}

#[cfg(test)]
pub fn folder() -> PathBuf {
  dir().expect("test folder")
}

#[cfg(test)]
pub fn path_of(name: &str) -> Option<PathBuf> {
  dir().map(|d| d.join(name))
}

// None = no such file, or one we can't use. a file that exists but can't be read as text is moved
// aside as `<name>.bak` and said so: the next save starts a fresh one, nothing is lost
pub fn read(name: &str) -> Option<String> {
  match std::fs::read_to_string(dir()?.join(name)) {
    Ok(s) => Some(s),
    Err(e) if e.kind() == ErrorKind::NotFound => None,
    Err(_) => {
      set_aside(name, None);
      None
    }
  }
}

// `<name>` → `<name>.bak` (a copy from an earlier fall is replaced), and a plain line saying so.
// `part`: how many lines were unusable when the rest still is
pub fn set_aside(name: &str, part: Option<usize>) {
  let Some(d) = dir() else { return };
  let moved = std::fs::rename(d.join(name), d.join(format!("{name}.bak"))).is_ok();
  note(match (moved, part) {
    (true, None) => format!("your {name} file was unreadable; saved a copy as {name}.bak and started fresh"),
    (true, Some(1)) => format!("1 line in your {name} file wasn't usable; saved a copy as {name}.bak"),
    (true, Some(n)) => format!("{n} lines in your {name} file weren't usable; saved a copy as {name}.bak"),
    (false, _) => format!("your {name} file was unreadable and couldn't be set aside; the next save replaces it"),
  });
}

// default owner-only; see `ensure_dir`
pub fn write(name: &str, data: &str) -> Result<()> {
  let r = write_file(name, data);
  match &r {
    Ok(()) => saved(name),
    Err(e) => save_failed(name, e),
  }
  r
}

// the login token. every file here is owner-only already; the name keeps its call sites plain
pub fn write_secret(name: &str, data: &str) -> Result<()> {
  write(name, data)
}

pub fn remove(name: &str) {
  if let Some(d) = dir() {
    let _ = std::fs::remove_file(d.join(name));
  }
}

fn write_file(name: &str, data: &str) -> Result<()> {
  let d = ensure_dir()?;
  let tmp = d.join(format!(".{name}.tmp"));
  let _ = std::fs::remove_file(&tmp);
  let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
  f.write_all(data.as_bytes())?;
  f.sync_all()?;
  std::fs::rename(tmp, d.join(name))
}

// one line onto a small log (owner-only); past `max` bytes the log starts over
pub fn append(name: &str, line: &str, max: u64) -> Result<()> {
  let d = ensure_dir()?;
  let p = d.join(name);
  let full = std::fs::metadata(&p).is_ok_and(|m| m.len() > max);
  let mut f = std::fs::OpenOptions::new().create(true).append(!full).write(true).truncate(full).mode(0o600).open(p)?;
  f.write_all(line.as_bytes())
}

// a run that was told its channels on the command line: they are for that run only, so the saved
// tabs stay as they were
pub fn pin_tabs() {
  notes(|n| n.pinned = true);
}

pub fn load_tabs() -> Vec<Box<str>> {
  read("tabs").map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(Into::into).collect()).unwrap_or_default()
}

// the saved tabs had lines that aren't channels: keep the original as tabs.bak, write the good ones back
pub fn salvage_tabs(good: &[Box<str>], junk: usize) {
  set_aside("tabs", Some(junk));
  write_tabs(good);
}

pub fn save_tabs(tabs: &[Box<str>]) {
  if !notes(|n| n.pinned) {
    write_tabs(tabs);
  }
}

fn write_tabs(tabs: &[Box<str>]) {
  let mut s = tabs.join("\n");
  s.push('\n');
  let _ = write("tabs", &s);
}

#[cfg(test)]
mod tests {
  use super::*;

  fn mode(p: &std::path::Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
  }

  fn fresh() -> PathBuf {
    let d = dir().unwrap();
    let _ = std::fs::remove_dir_all(d.parent().unwrap());
    d
  }

  #[test]
  fn the_folder_and_its_files_are_owner_only() {
    let d = fresh();
    write("settings", "zoom=100\n").unwrap();
    write_secret("token", "x").unwrap();
    append("crash.log", "line\n", 1000).unwrap();
    assert_eq!(mode(&d), 0o700);
    for f in ["settings", "token", "crash.log"] {
      assert_eq!(mode(&d.join(f)), 0o600, "{f}");
    }
    assert!(!d.join(".settings.tmp").exists());
  }

  #[test]
  fn init_tightens_what_an_older_version_left_open() {
    let d = fresh();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("settings"), "zoom=100\n").unwrap();
    std::fs::set_permissions(&d, PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::set_permissions(d.join("settings"), PermissionsExt::from_mode(0o644)).unwrap();
    init();
    assert_eq!(mode(&d), 0o700);
    assert_eq!(mode(&d.join("settings")), 0o600);
    assert_eq!(read("settings").as_deref(), Some("zoom=100\n"));
    // and a write into a loose folder fixes it too
    std::fs::set_permissions(&d, PermissionsExt::from_mode(0o750)).unwrap();
    write("tabs", "a\n").unwrap();
    assert_eq!(mode(&d), 0o700);
  }

  #[test]
  fn a_failed_save_is_said_once_until_it_works_again() {
    let d = fresh();
    // a file where the folder should be: every save fails
    std::fs::create_dir_all(d.parent().unwrap()).unwrap();
    std::fs::write(&d, "in the way").unwrap();
    assert!(write("settings", "a").is_err());
    assert!(write("settings", "b").is_err());
    let said = take_notes();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("couldn't save your settings"), "{said:?}");
    assert!(write("tabs", "a").is_err());
    assert_eq!(take_notes().len(), 1, "another file is another note");
    std::fs::remove_file(&d).unwrap();
    write("settings", "c").unwrap();
    assert!(take_notes().is_empty());
    // saved again, so the next failure is news again
    std::fs::remove_dir_all(&d).unwrap();
    std::fs::write(&d, "in the way").unwrap();
    assert!(write("settings", "d").is_err());
    assert_eq!(take_notes().len(), 1);
  }

  #[test]
  fn an_unreadable_file_is_set_aside_and_said() {
    let d = fresh();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("settings"), [0xff, 0xfe, 0x00, 0xc3, 0x28]).unwrap();
    assert!(read("settings").is_none());
    assert_eq!(std::fs::read(d.join("settings.bak")).unwrap(), [0xff, 0xfe, 0x00, 0xc3, 0x28]);
    assert!(!d.join("settings").exists());
    let said = take_notes();
    assert_eq!(said, ["your settings file was unreadable; saved a copy as settings.bak and started fresh"]);
    // fresh now: no file is no news
    assert!(read("settings").is_none());
    assert!(take_notes().is_empty());
  }

  #[test]
  fn a_run_with_its_own_channels_leaves_the_saved_tabs_alone() {
    fresh();
    let one: [Box<str>; 1] = ["forsen".into()];
    save_tabs(&one);
    pin_tabs();
    save_tabs(&["xqc".into(), "kick:somebody".into()]);
    assert_eq!(load_tabs(), one, "pinned: not overwritten");
    // a damaged saved file is still kept, with its good lines written back, in a pinned run too
    write("tabs", "forsen\nnope:::\n").unwrap();
    salvage_tabs(&one, 1);
    assert_eq!(load_tabs(), one);
    assert!(path_of("tabs.bak").unwrap().exists());
  }

  #[test]
  fn tabs_with_junk_keep_the_original_and_write_back_the_good_ones() {
    let d = fresh();
    write("tabs", "forsen\nnope:::\nkick:xqc\n").unwrap();
    let good: Vec<Box<str>> = vec!["forsen".into(), "kick:xqc".into()];
    salvage_tabs(&good, 1);
    assert_eq!(std::fs::read_to_string(d.join("tabs.bak")).unwrap(), "forsen\nnope:::\nkick:xqc\n");
    assert_eq!(load_tabs(), good);
    assert_eq!(take_notes(), ["1 line in your tabs file wasn't usable; saved a copy as tabs.bak"]);
  }
}
