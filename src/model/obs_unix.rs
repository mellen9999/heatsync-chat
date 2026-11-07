// is obs running, on unix: a process named `obs`, read from /proc
use std::fs;
use std::path::{Path, PathBuf};

// processes looked at per check: a runaway /proc never turns a check into a stall
const SCAN_MAX: usize = 20_000;

// where processes are listed (tests point it at a fake one)
pub fn proc_root() -> PathBuf {
  "/proc".into()
}

// is a process named `obs` running? reads `<root>/<pid>/comm` (the kernel's name for the process, 15
// chars: flatpak and native builds are both `obs`). `root` is /proc, or a fake one in tests
pub fn obs_running(root: &Path) -> bool {
  let Ok(dir) = fs::read_dir(root) else { return false };
  for e in dir.flatten().take(SCAN_MAX) {
    if !e.file_name().as_encoded_bytes().iter().all(u8::is_ascii_digit) {
      continue;
    }
    if let Ok(c) = fs::read_to_string(e.path().join("comm"))
      && c.trim_end() == "obs"
    {
      return true;
    }
  }
  false
}

#[cfg(test)]
pub fn fake_proc(tag: &str, procs: &[(&str, &str)]) -> PathBuf {
  let root = std::env::temp_dir().join(format!("hsc-proc-{tag}-{}", std::process::id()));
  let _ = fs::remove_dir_all(&root);
  fs::create_dir_all(&root).unwrap();
  for (pid, comm) in procs {
    fs::create_dir_all(root.join(pid)).unwrap();
    fs::write(root.join(pid).join("comm"), format!("{comm}\n")).unwrap();
  }
  root
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn obs_is_found_by_its_process_name() {
    let r = fake_proc("yes", &[("1", "systemd"), ("42", "obs"), ("77", "foot")]);
    assert!(obs_running(&r));
    let _ = fs::remove_dir_all(r);
  }

  #[test]
  fn other_names_and_non_pid_entries_do_not_count() {
    let r = fake_proc("no", &[("1", "systemd"), ("5", "obs-ndi"), ("6", "xobs"), ("self", "obs")]);
    assert!(!obs_running(&r), "only an exact `obs`, only under a pid");
    let _ = fs::remove_dir_all(r);
    assert!(!obs_running(Path::new("/nonexistent-proc-root")));
  }
}
