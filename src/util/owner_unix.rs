// owner-only files on unix: folders 0700, files 0600, and an older version's looser ones tightened
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Error, ErrorKind, Result};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

// the folder exists and is ours alone: made 0700 when missing, tightened when it was left open
pub fn dir(d: &Path) -> Result<()> {
  match std::fs::metadata(d) {
    Ok(m) if !m.is_dir() => Err(Error::new(ErrorKind::NotADirectory, "not a folder")),
    Ok(m) if m.permissions().mode() & 0o077 != 0 => std::fs::set_permissions(d, PermissionsExt::from_mode(0o700)),
    Ok(_) => Ok(()),
    Err(_) => std::fs::DirBuilder::new().recursive(true).mode(0o700).create(d),
  }
}

// a file an older version left readable by others goes owner-only (best effort)
pub fn tighten(p: &Path, m: &Metadata) {
  if m.is_file() && m.permissions().mode() & 0o077 != 0 {
    let _ = std::fs::set_permissions(p, PermissionsExt::from_mode(0o600));
  }
}

// a file made by these options is owner-only
pub fn file(o: &mut OpenOptions) -> &mut OpenOptions {
  o.mode(0o600)
}

// a chat log opened to append to, made 0600; a symlink planted at its name is not followed
pub fn log(p: &Path) -> Result<File> {
  OpenOptions::new().create(true).append(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(p)
}

#[cfg(test)]
pub fn mode(p: &Path) -> u32 {
  std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}
