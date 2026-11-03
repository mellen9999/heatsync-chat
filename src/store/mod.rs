// small persisted state under $XDG_CONFIG_HOME/heatsync-chat, written atomically (tmp + rename)
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
  let t = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "");
  Some(std::env::temp_dir().join("hsc-test").join(format!("{}-{t}", std::process::id())).join("heatsync-chat"))
}

pub fn read(name: &str) -> Option<String> {
  std::fs::read_to_string(dir()?.join(name)).ok()
}

pub fn write(name: &str, data: &str) -> std::io::Result<()> {
  write_mode(name, data, 0o644)
}

// owner-only (the login token): created 0600 before a byte is written, so it is never world-readable
pub fn write_secret(name: &str, data: &str) -> std::io::Result<()> {
  write_mode(name, data, 0o600)
}

pub fn remove(name: &str) {
  if let Some(d) = dir() {
    let _ = std::fs::remove_file(d.join(name));
  }
}

fn write_mode(name: &str, data: &str, mode: u32) -> std::io::Result<()> {
  use std::io::Write;
  use std::os::unix::fs::OpenOptionsExt;
  let d = dir().ok_or_else(|| std::io::Error::other("no home directory"))?;
  std::fs::create_dir_all(&d)?;
  let tmp = d.join(format!(".{name}.tmp"));
  let _ = std::fs::remove_file(&tmp);
  let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(&tmp)?;
  f.write_all(data.as_bytes())?;
  f.sync_all()?;
  std::fs::rename(tmp, d.join(name))
}

pub fn load_tabs() -> Vec<Box<str>> {
  read("tabs").map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(Into::into).collect()).unwrap_or_default()
}

pub fn save_tabs(tabs: &[Box<str>]) {
  let mut s = tabs.join("\n");
  s.push('\n');
  if let Err(e) = write("tabs", &s) {
    eprintln!("save tabs: {e}");
  }
}
