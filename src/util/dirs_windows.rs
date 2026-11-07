// where the app keeps things on windows: settings and tabs are small and follow the account (%APPDATA%,
// roaming); logs and the picture cache grow and stay on this machine (%LOCALAPPDATA%). a variable that is
// missing or not an absolute path is no folder at all: the callers fail loud ("no home directory")
// tests keep their own scratch folders (store, images::cache), so the real ones go unused there
#![cfg_attr(test, allow(dead_code))]

use std::path::{Path, PathBuf};

fn abs(var: &str) -> Option<PathBuf> {
  std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute())
}

pub fn config() -> Option<PathBuf> {
  Some(abs("APPDATA")?.join("heatsync-chat"))
}

pub fn logs() -> Option<PathBuf> {
  Some(abs("LOCALAPPDATA")?.join("heatsync-chat").join("logs"))
}

pub fn cache() -> Option<PathBuf> {
  Some(abs("LOCALAPPDATA")?.join("heatsync-chat"))
}

pub fn home() -> Option<PathBuf> {
  abs("USERPROFILE")
}

// there is no `~` on windows: a status line shows the whole folder
pub fn show(dir: &Path) -> String {
  dir.display().to_string()
}

pub const HELP_FOLDERS: &str = "your settings and tabs are in %APPDATA%\\heatsync-chat, the picture cache in
%LOCALAPPDATA%\\heatsync-chat. both are private to you.";

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_missing_or_relative_variable_is_no_folder() {
    // SAFETY: the one test that touches this variable
    unsafe {
      std::env::remove_var("APPDATA");
      assert_eq!(config(), None);
      std::env::set_var("APPDATA", "relative\\dir");
      assert_eq!(config(), None);
      std::env::set_var("APPDATA", "C:\\Users\\a\\AppData\\Roaming");
    }
    assert_eq!(config().unwrap(), PathBuf::from("C:\\Users\\a\\AppData\\Roaming\\heatsync-chat"));
  }
}
