// is obs running, on windows: a snapshot of the process list with an `obs64.exe` (or `obs32.exe`) in it
use std::path::{Path, PathBuf};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
  CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

use crate::util::win::Handle;

// processes looked at per check: a runaway list never turns a check into a stall
const SCAN_MAX: usize = 20_000;

// windows lists processes through the api, not a folder: there is no root to point anywhere
pub fn proc_root() -> PathBuf {
  PathBuf::new()
}

// an exe file name (utf-16, null-ended inside a fixed array) is obs, whatever the case
fn is_obs(name: &[u16]) -> bool {
  let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
  let n = String::from_utf16_lossy(&name[..end]).to_ascii_lowercase();
  n == "obs64.exe" || n == "obs32.exe"
}

pub fn obs_running(_: &Path) -> bool {
  // SAFETY: a snapshot handle we close; the entry is plain data with its size set first, as the api asks
  unsafe {
    let Some(snap) = Handle::ok(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)) else { return false };
    let mut e: PROCESSENTRY32W = std::mem::zeroed();
    e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut ok = Process32FirstW(snap.0, &mut e);
    let mut seen = 0;
    while ok != 0 && seen < SCAN_MAX {
      if is_obs(&e.szExeFile) {
        return true;
      }
      seen += 1;
      ok = Process32NextW(snap.0, &mut e);
    }
  }
  false
}

#[cfg(test)]
mod tests {
  use super::*;

  fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0, 0x41, 0x42]).collect()
  }

  #[test]
  fn obs_is_found_by_its_exe_name() {
    for yes in ["obs64.exe", "OBS64.EXE", "obs32.exe"] {
      assert!(is_obs(&w(yes)), "{yes}");
    }
    for no in ["obs.exe", "obs64.exe.bak", "xobs64.exe", "obs-ndi.exe", "", "Streamlabs OBS.exe"] {
      assert!(!is_obs(&w(no)), "{no}");
    }
    assert!(!is_obs(&[]), "no name is not obs");
  }

  #[test]
  fn the_real_list_is_readable() {
    // this test process is not obs, and the walk ends
    assert!(!obs_running(Path::new("")));
  }
}
