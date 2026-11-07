// a desktop notification on windows: none yet. the in-app status line and the sound still say it. a tray
// balloon (Shell_NotifyIconW, added when it is needed and removed a few seconds after the last one) is the
// planned popup; until it lands this says so once, on the log, and shows nothing
use std::sync::atomic::{AtomicBool, Ordering};

pub fn show(what: &str, _title: &str, _body: &str) {
  static SAID: AtomicBool = AtomicBool::new(false);
  if !SAID.swap(true, Ordering::Relaxed) {
    eprintln!("{what} notifications don't pop up on windows yet");
  }
}
