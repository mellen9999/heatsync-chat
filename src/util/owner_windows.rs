// owner-only files on windows: nothing to set. a folder under %APPDATA% / %LOCALAPPDATA% inherits the
// account's own access list from the profile (the owner, SYSTEM and the administrators, no other user),
// so every file made in it is private the same way; there is no mode to loosen or tighten. the one thing
// kept from unix is the chat log refusing a symlink at its name
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Error, ErrorKind, Result};
use std::path::Path;

pub fn dir(d: &Path) -> Result<()> {
  match std::fs::metadata(d) {
    Ok(m) if !m.is_dir() => Err(Error::new(ErrorKind::NotADirectory, "not a folder")),
    Ok(_) => Ok(()),
    Err(_) => std::fs::DirBuilder::new().recursive(true).create(d),
  }
}

pub fn tighten(_: &Path, _: &Metadata) {}

pub fn file(o: &mut OpenOptions) -> &mut OpenOptions {
  o
}

pub fn log(p: &Path) -> Result<File> {
  // symlinks need a privilege on windows, so the attack is narrow; refusing one keeps the intent
  if std::fs::symlink_metadata(p).is_ok_and(|m| m.is_symlink()) {
    return Err(Error::other("a link where the log should be"));
  }
  OpenOptions::new().create(true).append(true).open(p)
}
