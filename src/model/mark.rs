// the one platform mark: [T] [K] [Y] [H], or [?] when a platform name is one we do not know. a row's mark is a
// pure function of its own data: [H] when the line is known to have gone through heatsync, else the platform
// it was said on. every surface asks here; none spells a letter or a colour itself
use std::sync::atomic::{AtomicBool, Ordering};

use super::message::{Message, Platform, flag};
use crate::ui::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
  Twitch,
  Kick,
  Youtube,
  Heatsync,
  // an ingest bug: a platform name nothing here knows
  Unknown,
}

impl From<Platform> for Mark {
  fn from(p: Platform) -> Self {
    match p {
      Platform::Twitch => Mark::Twitch,
      Platform::Kick => Mark::Kick,
      Platform::Youtube => Mark::Youtube,
    }
  }
}

static WARNED: AtomicBool = AtomicBool::new(false);

impl Mark {
  // a row's mark: [H] once it is known to be a heatsync send, else where it was said
  pub fn of(m: &Message) -> Mark {
    if m.x.as_ref().is_some_and(|x| x.origin) { Mark::Heatsync } else { m.platform.into() }
  }

  // the mark a row wears, if it wears one. a status line (`Message::status`, the app's own words about what you
  // just did) is no chat row: no platform said it and heatsync did not carry it, so it wears none (the site and the
  // extension show the same words as a toast, which has no mark). every surface that draws a row's mark asks here
  pub fn worn(m: &Message) -> Option<Mark> {
    Some(Mark::of(m)).filter(|_| !m.has(flag::STATUS))
  }

  // a platform name off the wire. canonical names only (ingress normalizes aliases); anything else is
  // `Unknown`, reported once
  pub fn named(s: &str) -> Mark {
    match s {
      "twitch" => Mark::Twitch,
      "kick" => Mark::Kick,
      "youtube" => Mark::Youtube,
      "heatsync" => Mark::Heatsync,
      other => {
        if !WARNED.swap(true, Ordering::Relaxed) {
          eprintln!("mark: unknown platform {:?}", other.chars().take(32).collect::<String>());
        }
        Mark::Unknown
      }
    }
  }

  pub const fn letter(self) -> char {
    match self {
      Mark::Twitch => 'T',
      Mark::Kick => 'K',
      Mark::Youtube => 'Y',
      Mark::Heatsync => 'H',
      Mark::Unknown => '?',
    }
  }

  // `[T]`
  pub const fn tag(self) -> &'static str {
    match self {
      Mark::Twitch => "[T]",
      Mark::Kick => "[K]",
      Mark::Youtube => "[Y]",
      Mark::Heatsync => "[H]",
      Mark::Unknown => "[?]",
    }
  }

  // `[T] `, for a segment that sits before words
  pub const fn tag_sp(self) -> &'static str {
    match self {
      Mark::Twitch => "[T] ",
      Mark::Kick => "[K] ",
      Mark::Youtube => "[Y] ",
      Mark::Heatsync => "[H] ",
      Mark::Unknown => "[?] ",
    }
  }

  pub const fn ink(self) -> u32 {
    match self {
      Mark::Twitch => theme::TWITCH,
      Mark::Kick => theme::GREEN,
      Mark::Youtube => theme::RED,
      Mark::Heatsync => theme::HEATSYNC,
      Mark::Unknown => theme::YELLOW,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_mark_is_its_rows_own_data() {
    let mut m = Message::system(1, Platform::Kick, "");
    assert_eq!(Mark::of(&m), Mark::Kick);
    m.extra().origin = true;
    assert_eq!(Mark::of(&m), Mark::Heatsync, "origin upgrades it");
    assert_eq!(Mark::named("heatsync"), Mark::Heatsync);
    assert_eq!(Mark::named("yt"), Mark::Unknown, "an alias is not a platform here");
    assert_eq!(Mark::Unknown.tag(), "[?]");
    assert_eq!(Mark::Youtube.tag_sp(), "[Y] ");
  }

  // a status line is the app's own voice: no platform said it, heatsync did not carry it, so no mark
  #[test]
  fn a_status_line_is_no_platforms_row() {
    let m = Message::status(1, "added MeN to slot 12");
    assert_eq!(Mark::worn(&m), None);
    let mut said = Message::system(1, Platform::Twitch, "added MeN to slot 12");
    assert_eq!(Mark::worn(&said), Some(Mark::Twitch), "the same words from a platform keep its mark");
    said.extra().origin = true;
    assert_eq!(Mark::worn(&said), Some(Mark::Heatsync));
  }
}
