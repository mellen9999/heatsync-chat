// opening things on unix: programs on PATH are found by their exec bit, links go to xdg-open
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// the executable of that name on PATH (looked up each time: installing mpv works without a restart)
pub fn which(bin: &str) -> Option<PathBuf> {
  use std::os::unix::fs::PermissionsExt;
  let path = std::env::var_os("PATH")?;
  std::env::split_paths(&path)
    .map(|d| Path::new(&d).join(bin))
    .find(|p| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

// nothing to hide on unix: a program started here has no console of its own to flash
pub fn quiet(_: &mut Command) {}

// the default browser (the link is already checked)
pub fn browse(url: &str) -> Result<(), String> {
  let mut child = Command::new("xdg-open")
    .arg(url)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| {
      eprintln!("xdg-open: {e}");
      "can't open links yet, install xdg-utils (paru -S xdg-utils)".to_string()
    })?;
  // reap it so it never lingers as a zombie
  std::thread::spawn(move || _ = child.wait());
  Ok(())
}
