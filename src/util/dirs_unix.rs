// where the app keeps things on unix: the xdg base folders (the variable when it is an absolute path,
// else the home fallback), each with a `heatsync-chat` folder of its own
// tests keep their own scratch folders (store, images::cache), so the real ones go unused there
#![cfg_attr(test, allow(dead_code))]

use std::path::{Path, PathBuf};

fn xdg(var: &str, home_rel: &str) -> Option<PathBuf> {
  std::env::var_os(var)
    .map(PathBuf::from)
    .filter(|p| p.is_absolute())
    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(home_rel)))
}

// settings, tabs, the login: $XDG_CONFIG_HOME/heatsync-chat
pub fn config() -> Option<PathBuf> {
  Some(xdg("XDG_CONFIG_HOME", ".config")?.join("heatsync-chat"))
}

// chat logs (data, not config: it grows and is yours to keep): $XDG_DATA_HOME/heatsync-chat/logs
pub fn logs() -> Option<PathBuf> {
  Some(xdg("XDG_DATA_HOME", ".local/share")?.join("heatsync-chat").join("logs"))
}

// the picture cache's parent: $XDG_CACHE_HOME/heatsync-chat
pub fn cache() -> Option<PathBuf> {
  Some(xdg("XDG_CACHE_HOME", ".cache")?.join("heatsync-chat"))
}

// the home folder, for `~/` in paths typed by the owner
pub fn home() -> Option<PathBuf> {
  std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute())
}

// a folder as a status line shows it: the home folder as `~`
pub fn show(dir: &Path) -> String {
  match home().as_deref().and_then(|h| dir.strip_prefix(h).ok()) {
    Some(rest) => format!("~/{}", rest.display()),
    None => dir.display().to_string(),
  }
}

// the folders as the --help text names them
pub const HELP_FOLDERS: &str = "your settings and tabs are in ~/.config/heatsync-chat, the picture cache in
~/.cache/heatsync-chat. both are private to you.";
