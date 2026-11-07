// the few shapes every win32 call here shares: a utf-16 string with its end mark, and a handle that closes
// itself. windows only (declared with a cfg in util/mod.rs)
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

// set (for the child only) by the updater's restart: the old copy is still closing, so the new one waits for
// its lock instead of taking it for a second launch
pub const RESTART_VAR: &str = "HSC_RESTARTED";

// `s` as a null-terminated utf-16 string; an interior null ends it early, which no caller here passes
pub fn wide(s: &str) -> Vec<u16> {
  s.encode_utf16().chain(Some(0)).collect()
}

// a handle closed when dropped. null and INVALID_HANDLE_VALUE are both "none"
pub struct Handle(pub HANDLE);

// SAFETY: a win32 handle is a process-wide id, valid from any thread
unsafe impl Send for Handle {}

impl Handle {
  pub fn ok(h: HANDLE) -> Option<Handle> {
    (!h.is_null() && h != INVALID_HANDLE_VALUE).then_some(Handle(h))
  }
}

impl Drop for Handle {
  fn drop(&mut self) {
    // SAFETY: we own this handle and close it once
    unsafe { CloseHandle(self.0) };
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn strings_end_with_a_null() {
    assert_eq!(wide("ab"), [97, 98, 0]);
    assert_eq!(wide(""), [0]);
    assert_eq!(wide("é"), [0xe9, 0]);
  }
}
