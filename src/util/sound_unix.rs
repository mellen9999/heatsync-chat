// playing a wav on unix: the first of pw-play, paplay, aplay on PATH, fed the wav on stdin. best effort: no
// player is said once on stderr
use crate::util::open::installed;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

const PLAYERS: [(&str, &[&str]); 3] = [("pw-play", &["-"]), ("paplay", &[]), ("aplay", &["-q", "-"])];

// `bytes` makes the wav, off the ui thread: the same thread feeds the player and reaps it, so a stuck
// player never stalls a frame or leaves a zombie
pub fn play(bytes: impl FnOnce() -> Vec<u8> + Send + 'static) {
  static MISSING: AtomicBool = AtomicBool::new(false);
  let Some((bin, args)) = PLAYERS.iter().find(|(b, _)| installed(b)) else {
    if !MISSING.swap(true, Ordering::Relaxed) {
      eprintln!("the mention sound needs pw-play, paplay or aplay");
    }
    return;
  };
  let spawned = Command::new(bin).args(*args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
  let Ok(mut child) = spawned else { return };
  std::thread::spawn(move || {
    if let Some(mut stdin) = child.stdin.take() {
      let _ = stdin.write_all(&bytes());
    }
    let _ = child.wait();
  });
}
