// streamer mode's plain parts: what hidden text says, and whether obs is running. the app side
// (what flips, what is held back) is src/app/streamer.rs
#[cfg_attr(unix, path = "obs_unix.rs")]
#[cfg_attr(windows, path = "obs_windows.rs")]
mod obs;
#[cfg(all(test, unix))]
pub use obs::fake_proc;
pub use obs::{obs_running, proc_root};

// what a hidden whisper row says in place of its words
#[cfg(test)]
pub const HIDDEN: &str = "hidden: press space s or click to show";

pub fn hidden() -> &'static str {
  crate::i18n::t("chat2.streamer.hidden")
}

// how often auto looks for obs, only while auto is the setting
pub const CHECK_SECS: u64 = 10;
