// small persisted state under $XDG_CONFIG_HOME/heatsync-chat, written atomically (tmp + rename)
use std::path::PathBuf;

fn dir() -> Option<PathBuf> {
  let base = std::env::var_os("XDG_CONFIG_HOME")
    .map(PathBuf::from)
    .filter(|p| p.is_absolute())
    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
  Some(base.join("heatsync-chat"))
}

pub fn read(name: &str) -> Option<String> {
  std::fs::read_to_string(dir()?.join(name)).ok()
}

pub fn write(name: &str, data: &str) -> std::io::Result<()> {
  let d = dir().ok_or_else(|| std::io::Error::other("no home directory"))?;
  std::fs::create_dir_all(&d)?;
  let tmp = d.join(format!(".{name}.tmp"));
  std::fs::write(&tmp, data)?;
  std::fs::rename(tmp, d.join(name))
}

pub fn load_tabs() -> Vec<Box<str>> {
  read("tabs")
    .map(|s| {
      s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(Into::into)
        .collect()
    })
    .unwrap_or_default()
}

pub fn save_tabs(tabs: &[Box<str>]) {
  let mut s = tabs.join("\n");
  s.push('\n');
  if let Err(e) = write("tabs", &s) {
    eprintln!("save tabs: {e}");
  }
}
