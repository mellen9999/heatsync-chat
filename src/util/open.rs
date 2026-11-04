// open a link from chat in the default browser, or play a video or a stream in mpv. the text is hostile:
// only http(s), no spaces or control characters, bounded, and never through a shell (the url is one
// argument, after mpv's `--`). mpv gets only https, and none of the user's mpv config or scripts
use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::video;

fn check(url: &str) -> Result<(), String> {
  let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
  let ok = (lower.starts_with("https://") || lower.starts_with("http://"))
    && url.len() <= 2048
    && !url.chars().any(|c| c.is_control() || c.is_whitespace());
  if ok { Ok(()) } else { Err("not a link i can open".into()) }
}

// an executable of that name on PATH (looked up each time: installing mpv works without a restart)
pub fn installed(bin: &str) -> bool {
  use std::os::unix::fs::PermissionsExt;
  std::env::var_os("PATH").is_some_and(|p| {
    std::env::split_paths(&p)
      .any(|d| Path::new(&d).join(bin).metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
  })
}

// what mpv is started with. `--no-config` and `--load-scripts=no`: a link from chat never runs with the
// user's mpv.conf options or scripts. `--` ends the options, so the url (which `check` made an http(s)
// address) is the thing to play and never a flag
fn mpv_args(url: &str) -> [&str; 6] {
  ["--no-config", "--load-scripts=no", "--force-window=immediate", "--no-terminal", "--", url]
}

fn https(url: &str) -> bool {
  url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))
}

// mpv plays it when it can: a page (youtube, twitch) needs yt-dlp too, a video file only mpv. else the
// browser. if mpv gives up (a stream gone offline, a page it can't read) the browser gets it after all,
// and `status` says so
pub fn play(url: &str, status: impl FnOnce(String) + Send + 'static) -> Result<(), String> {
  check(url)?;
  let ytdl = installed("yt-dlp") || installed("youtube-dl");
  // plain http stays in the browser: nothing unencrypted is fed to a player
  if !https(url) || !installed("mpv") || !(ytdl || video::file(url)) {
    return open_url(url);
  }
  let mut child = Command::new("mpv")
    .args(mpv_args(url))
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| format!("couldn't start mpv ({e})"))?;
  let url = url.to_owned();
  std::thread::spawn(move || {
    // 1: mpv failed, 2: it couldn't play the link. a quit or a closed window is 0
    if child.wait().is_ok_and(|s| matches!(s.code(), Some(1 | 2))) {
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
  let mut child = Command::new("xdg-open")
    .arg(url)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| format!("couldn't open the link (xdg-open: {e})"))?;
  // reap it so it never lingers as a zombie
  std::thread::spawn(move || _ = child.wait());
  Ok(())
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
}
