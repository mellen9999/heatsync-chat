// where the app's own words go on windows. the release exe has no console window (it is a "windows" program,
// see main.rs), so out of the box stdout and stderr go nowhere. three small fixes: --help from a terminal
// still prints, diagnostics land in a log file, and the few things a double-clicker must hear are a message box
use super::win::wide;
use std::os::windows::io::IntoRawHandle;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{
  ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};

// past this many bytes the log starts over: it is for the last run's trouble, not a history
const LOG_MAX: u64 = 256 << 10;

fn missing(h: HANDLE) -> bool {
  h.is_null() || h == INVALID_HANDLE_VALUE
}

// --help and friends print to the terminal that started us (if one did); a redirect (`> file`) is kept
pub fn attach_terminal() {
  // SAFETY: attaches to the parent's console when it has one, then points the std handles that have nowhere to go at it
  unsafe {
    if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
      return;
    }
    for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
      if !missing(GetStdHandle(which)) {
        continue;
      }
      if let Ok(f) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
        SetStdHandle(which, f.into_raw_handle());
      }
    }
  }
}

// diagnostics go to `heatsync-chat.log` beside the cache, unless stderr already goes somewhere
pub fn log_stderr() {
  // SAFETY: reading the process's std handle
  if !missing(unsafe { GetStdHandle(STD_ERROR_HANDLE) }) {
    return;
  }
  let Some(dir) = super::dirs::cache() else { return };
  let path = dir.join("heatsync-chat.log");
  let big = std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX);
  let file = super::owner::dir(&dir)
    .and_then(|()| std::fs::OpenOptions::new().create(true).append(!big).write(true).truncate(big).open(&path));
  if let Ok(f) = file {
    // SAFETY: the file stays open for the life of the process (its handle is not closed)
    unsafe { SetStdHandle(STD_ERROR_HANDLE, f.into_raw_handle()) };
  }
}

// a message box: for someone who double-clicked and would otherwise see nothing happen
pub fn alert(title: &str, text: &str) {
  let (t, b) = (wide(title), wide(text));
  // SAFETY: null-terminated strings that outlive the call, no parent window
  unsafe { MessageBoxW(null_mut(), b.as_ptr(), t.as_ptr(), MB_OK | MB_ICONINFORMATION) };
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Write;

  // a windows-subsystem exe starts with no stderr: what it says must still land somewhere readable
  #[test]
  fn stderr_with_nowhere_to_go_lands_in_the_log() {
    let d = std::env::temp_dir().join(format!("hsc-stdio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    // SAFETY: the one test that sets this variable; the std handle is put back empty, as a gui exe starts
    unsafe {
      std::env::set_var("LOCALAPPDATA", &d);
      SetStdHandle(STD_ERROR_HANDLE, null_mut());
    }
    log_stderr();
    std::io::stderr().write_all(b"hello from the log test\n").unwrap();
    let log = std::fs::read_to_string(d.join("heatsync-chat").join("heatsync-chat.log")).unwrap();
    assert!(log.contains("hello from the log test"), "{log}");
    let _ = std::fs::remove_dir_all(&d);
  }
}
