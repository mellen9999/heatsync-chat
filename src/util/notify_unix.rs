// a desktop notification on unix: notify-send with an argument list, never a shell
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

// `body` is already cleaned and capped
pub fn show(what: &str, title: &str, body: &str) {
  static MISSING: AtomicBool = AtomicBool::new(false);
  let spawned = Command::new("notify-send")
    .args(["-a", "heatsync", "-t", "8000", "--"])
    .arg(title)
    .arg(body)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn();
  match spawned {
    // reap it so a long session leaves no zombies
    Ok(mut child) => {
      std::thread::spawn(move || {
        let _ = child.wait();
      });
    }
    Err(e) => {
      if !MISSING.swap(true, Ordering::Relaxed) {
        eprintln!("{what} notifications need notify-send: {e}");
      }
    }
  }
}
