// open a link from chat in the default browser, or play a video or a stream in mpv. the text is hostile:
// only http(s), no spaces or control characters, bounded, and never through a shell (the url is one
// argument, after mpv's `--`). mpv gets only https, and none of the user's mpv config or scripts
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::i18n::t;
use crate::model::video;

// finding programs, hiding their console, and the browser itself differ by platform
#[cfg_attr(unix, path = "open_unix.rs")]
#[cfg_attr(windows, path = "open_windows.rs")]
mod os;
pub use os::browse;
use os::{quiet, which};

fn check(url: &str) -> Result<(), String> {
  let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
  let ok = (lower.starts_with("https://") || lower.starts_with("http://"))
    && url.len() <= 2048
    && !url.chars().any(|c| c.is_control() || c.is_whitespace());
  if ok { Ok(()) } else { Err(t("misc.open.bad_link").into()) }
}

// is a program of that name on PATH (looked up each time: installing mpv works without a restart)
pub fn installed(bin: &str) -> bool {
  which(bin).is_some()
}

// what mpv is started with. `--no-config` and `--load-scripts=no`: a link from chat never runs with the
// user's mpv.conf options or scripts. `--` ends the options, so the url (which `check` made an http(s)
// address) is the thing to play and never a flag
fn mpv_args(url: &str) -> [&str; 6] {
  ["--no-config", "--load-scripts=no", "--force-window=immediate", "--no-terminal", "--", url]
}

// a link just asked for is not asked for again: a still that opens a player takes seconds (mpv, then
// yt-dlp), and every impatient click used to start another player. one link, one open, until its mpv
// has ended or `SETTLE` has passed (the browser has no process to watch)
const SETTLE: Duration = Duration::from_secs(3);
static OPENED: Mutex<Vec<(Box<str>, Instant, bool)>> = Mutex::new(Vec::new());

fn claim(url: &str, now: Instant) -> bool {
  let Ok(mut o) = OPENED.lock() else { return true };
  o.retain(|(_, at, running)| *running || now.duration_since(*at) < SETTLE);
  if o.iter().any(|(u, _, _)| &**u == url) {
    return false;
  }
  o.push((url.into(), now, false));
  true
}

fn running(url: &str, on: bool) {
  if let Ok(mut o) = OPENED.lock() {
    if on {
      o.iter_mut().filter(|e| &*e.0 == url).for_each(|e| e.2 = true);
    } else {
      o.retain(|e| &*e.0 != url);
    }
  }
}

fn https(url: &str) -> bool {
  url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))
}

// mpv plays it when it can: a page (youtube, twitch) needs yt-dlp too, a video file only mpv. else the
// browser. if mpv gives up (a stream gone offline, a page it can't read) the browser gets it after all,
// and `status` says so
pub fn play(url: &str, status: impl FnOnce(String) + Send + 'static) -> Result<(), String> {
  check(url)?;
  if !claim(url, Instant::now()) {
    return Ok(());
  }
  let ytdl = installed("yt-dlp") || installed("youtube-dl");
  // plain http stays in the browser: nothing unencrypted is fed to a player
  let mpv = which("mpv").filter(|_| https(url) && (ytdl || video::file(url)));
  let Some(mpv) = mpv else { return open_url(url) };
  let mut cmd = Command::new(mpv);
  quiet(&mut cmd);
  let mut child =
    cmd.args(mpv_args(url)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| {
      eprintln!("mpv: {e}");
      running(url, false);
      "couldn't start mpv".to_string()
    })?;
  running(url, true);
  let url = url.to_owned();
  std::thread::spawn(move || {
    // 1: mpv failed, 2: it couldn't play the link. a quit or a closed window is 0
    let failed = child.wait().is_ok_and(|s| matches!(s.code(), Some(1 | 2)));
    running(&url, false);
    if failed {
      status(match open_url(&url) {
        Ok(()) => "mpv couldn't play that, so it opened in the browser".into(),
        Err(e) => e,
      });
    }
  });
  Ok(())
}

pub fn open_url(url: &str) -> Result<(), String> {
  check(url)?;
  browse(url)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mpv_never_reads_the_link_as_an_option_or_loads_the_users_config() {
    let args = mpv_args("https://youtu.be/abc");
    let end = args.iter().position(|a| *a == "--").expect("options end before the url");
    assert_eq!(args[end + 1..], ["https://youtu.be/abc"], "the link is the one thing after `--`");
    assert_eq!(args.len(), end + 2);
    for want in ["--no-config", "--load-scripts=no"] {
      assert!(args[..end].contains(&want), "{want} comes before the link");
    }
    // however hostile the link, it stays a single argument after the `--`
    let evil = "https://x.com/--input-ipc-server=/tmp/s";
    assert_eq!(mpv_args(evil)[end + 1], evil);
    assert_eq!(mpv_args(evil).len(), args.len());
    // only https is handed to mpv; anything else (even if it passed `check`) goes to the browser
    assert!(https("https://a.com/x") && https("HTTPS://a.com/x"));
    assert!(!https("http://a.com/x") && !https("file:///x") && !https("https:/") && !https(""));
  }

  #[test]
  fn only_plain_web_links_pass() {
    for bad in ["file:///etc/passwd", "javascript:alert(1)", "https://a b", "https://a\n", "ftp://x", "-h", ""] {
      assert!(open_url(bad).is_err(), "{bad}");
    }
    assert!(open_url(&format!("https://a.com/{}", "x".repeat(3000))).is_err());
    for bad in ["--input-ipc-server=/tmp/x", "file:///etc/passwd", "https://a b"] {
      assert!(play(bad, |_| {}).is_err(), "{bad}");
    }
  }

  #[test]
  fn a_link_asked_for_again_while_it_opens_is_not_opened_twice() {
    let (u, t) = ("https://claim.test/a", Instant::now());
    assert!(claim(u, t), "the first click opens it");
    assert!(!claim(u, t + Duration::from_millis(900)), "an impatient second click does nothing");
    assert!(claim("https://claim.test/b", t), "another link is its own");
    assert!(claim(u, t + SETTLE), "after the settle time it can be asked again");
    // a player that is still running keeps the link taken however long it takes
    running(u, true);
    assert!(!claim(u, t + SETTLE * 20));
    running(u, false);
    assert!(claim(u, t + SETTLE * 20), "and it is free when the player ends");
  }
}
