// the self-update's platform half on unix: what counts as a package manager's copy, the
// swap (one rename over the running file) and the restart (exec in place)
use super::exe;
use std::path::Path;

// the folder belongs to a package manager (/usr/bin), not to us
pub fn managed(dir: &Path) -> bool {
  dir.starts_with("/usr")
}

// nothing is left behind by a swap here
pub fn tidy() {}

// `exe` becomes the new bytes or stays as it was: written whole and flushed under a side name, one
// rename, then the folder flushed too, so a power cut can't leave a torn or missing app
pub fn replace(exe: &Path, bin: &[u8]) -> std::io::Result<()> {
  use std::io::Write;
  use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
  let tmp = exe.with_file_name(".heatsync-chat.new");
  // a leftover from a crashed update goes first, then create_new refuses to follow anything planted there
  let _ = std::fs::remove_file(&tmp);
  let put = || -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o755).open(&tmp)?;
    f.write_all(bin)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o755))?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, exe)
  };
  put().inspect_err(|_| {
    let _ = std::fs::remove_file(&tmp);
  })?;
  // the swap is done; this only makes it survive a power cut, so a folder that won't flush isn't an error
  if let Some(Ok(d)) = exe.parent().map(std::fs::File::open) {
    let _ = d.sync_all();
  }
  Ok(())
}

// start the new copy in this one's place, same arguments
pub fn restart() -> std::io::Error {
  use std::os::unix::process::CommandExt;
  match exe() {
    Some(p) => std::process::Command::new(p).args(std::env::args_os().skip(1)).exec(),
    None => std::io::Error::other("no path"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn replace_swaps_the_file_whole_and_executable() {
    use std::os::unix::fs::PermissionsExt;
    let d = std::env::temp_dir().join(format!("hsc-upd-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let exe = d.join("heatsync-chat");
    std::fs::write(&exe, b"old").unwrap();
    // a stale half-written leftover from a crash is cleared, not tripped over
    std::fs::write(d.join(".heatsync-chat.new"), b"torn").unwrap();
    replace(&exe, b"new bytes").unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"new bytes");
    assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755);
    assert!(!d.join(".heatsync-chat.new").exists(), "no side file left behind");
    // a folder we can't write leaves the old app alone and says so
    let gone = d.join("nope").join("heatsync-chat");
    assert!(replace(&gone, b"x").is_err());
    let _ = std::fs::remove_dir_all(&d);
  }
}
