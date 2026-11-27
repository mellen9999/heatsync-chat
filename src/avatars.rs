// the picture beside a chatter's name (ext avatars / `.hs-mc-avatar`). a login maps to an image url,
// asked once per login and kept in a 500-entry cache (the ext's cap). the net layer answers; every
// url is checked against the image host allowlist before it is kept
use crate::model::ident;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::images::url::allowed;
use crate::model::message::Platform;
use crate::ui::theme;

const MAX: usize = 500;
// a miss (no picture, outage, rate limit) is asked again no sooner than this
const RETRY: Duration = Duration::from_secs(5 * 60);
// a login's ask is in the air this long before it may be repeated
const INFLIGHT: Duration = Duration::from_secs(30);

struct Entry {
  url: Option<Box<str>>,
  at: Instant,
  // asked, not answered
  pending: bool,
}

#[derive(Default)]
pub struct Avatars {
  // twitch, kick, youtube: one map each so a read borrows the login instead of building a key
  maps: [HashMap<Box<str>, Entry>; 3],
  wanted: Vec<(Platform, Box<str>)>,
}

fn slot(p: Platform) -> Option<usize> {
  match p {
    Platform::Twitch => Some(0),
    Platform::Kick => Some(1),
    // youtube hands over the picture with the line: nothing is ever asked for a youtube name
    Platform::Youtube => Some(2),
  }
}

// a login we will put in a request: lowercase-able ascii names only
pub fn valid(login: &str) -> bool {
  ident::chatter_login(login)
}

impl Avatars {
  // the picture url for a chatter. an unknown login (or one past its retry time) queues one ask
  pub fn get(&mut self, p: Platform, login: &str) -> Option<&str> {
    let i = slot(p)?;
    if p == Platform::Youtube {
      return self.maps[i].get(login).and_then(|e| e.url.as_deref());
    }
    if !valid(login) {
      return None;
    }
    let now = Instant::now();
    if !self.maps[i].contains_key(login) {
      if self.maps.iter().map(HashMap::len).sum::<usize>() >= MAX {
        self.evict();
      }
      self.maps[i].insert(login.into(), Entry { url: None, at: now, pending: true });
      self.wanted.push((p, login.into()));
      return None;
    }
    let e = self.maps[i].get_mut(login)?;
    let due = if e.pending { INFLIGHT } else { RETRY };
    if e.url.is_none() && now - e.at > due {
      e.at = now;
      e.pending = true;
      self.wanted.push((p, login.into()));
    }
    e.url.as_deref()
  }

  pub fn take_wanted(&mut self) -> Vec<(Platform, Box<str>)> {
    std::mem::take(&mut self.wanted)
  }

  // an answer. true when the picture a line shows may have changed
  pub fn put(&mut self, p: Platform, login: &str, url: Option<Box<str>>) -> bool {
    if p == Platform::Youtube && !login.is_empty() {
      // a youtube line brings its own picture: keep it for the name's next lines (the ext's 500-entry cache)
      let known = self.maps[2].contains_key(login);
      if !known && url.as_deref().is_some_and(allowed) {
        if self.maps.iter().map(HashMap::len).sum::<usize>() >= MAX {
          self.evict();
        }
        self.maps[2].insert(login.into(), Entry { url: None, at: Instant::now(), pending: false });
      }
    }
    let Some(e) = slot(p).and_then(|i| self.maps[i].get_mut(login)) else { return false };
    // hostile like everything off the wire: only an allowlisted https image is kept
    let url = url.filter(|u| allowed(u));
    e.at = Instant::now();
    e.pending = false;
    let changed = e.url != url && url.is_some();
    if url.is_some() {
      e.url = url;
    }
    changed
  }

  #[cfg(test)]
  #[cfg(test)]
  pub fn len(&self) -> usize {
    self.maps.iter().map(HashMap::len).sum()
  }

  fn evict(&mut self) {
    let oldest = self
      .maps
      .iter()
      .enumerate()
      .flat_map(|(i, m)| m.iter().map(move |(k, e)| (i, k, e.at)))
      .min_by_key(|x| x.2)
      .map(|(i, k, _)| (i, k.clone()));
    if let Some((i, k)) = oldest {
      self.maps[i].remove(&k);
    }
  }
}

// the stand-in while the picture loads or when there is none: the first letter, and a color picked
// from the login (the ext's six-color hash, in the ansi palette)
pub fn tile(login: &str, nick: &str) -> (char, u32) {
  const INKS: [u32; 6] = [theme::WHITE, theme::BLUE, theme::GREEN, theme::YELLOW, theme::RED, theme::MAGENTA];
  let h = login.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
  let first = nick.chars().next().map_or('?', |c| c.to_uppercase().next().unwrap_or(c));
  (first, INKS[h as usize % INKS.len()])
}

#[cfg(test)]
mod tests {
  use super::*;

  const OK: &str = "https://static-cdn.jtvnw.net/jtv_user_pictures/a-70x70.png";

  #[test]
  fn one_ask_per_login_until_answered() {
    let mut a = Avatars::default();
    assert!(a.get(Platform::Twitch, "bob").is_none());
    assert!(a.get(Platform::Twitch, "bob").is_none());
    assert!(a.get(Platform::Kick, "bob").is_none());
    assert_eq!(a.take_wanted().len(), 2, "twitch bob and kick bob, once each");
    assert!(a.put(Platform::Twitch, "bob", Some(OK.into())));
    assert_eq!(a.get(Platform::Twitch, "bob"), Some(OK));
    assert!(a.take_wanted().is_empty());
  }

  #[test]
  fn hostile_urls_and_names_are_refused() {
    let mut a = Avatars::default();
    a.get(Platform::Twitch, "bob");
    for bad in [
      "http://static-cdn.jtvnw.net/a.png",
      "https://evil.example/a.png",
      "https://kick.com:@evil.com/a.png",
      "https://static-cdn.jtvnw.net/a b.png",
      "javascript:alert(1)",
    ] {
      assert!(!a.put(Platform::Twitch, "bob", Some(bad.into())), "{bad}");
      assert!(a.get(Platform::Twitch, "bob").is_none());
    }
    let long = "https://kick.com/".to_owned() + &"a".repeat(3000);
    assert!(!a.put(Platform::Twitch, "bob", Some(long.into())));
    for name in ["", "a b", "a/b", "../x", "x\"y", &"a".repeat(51)] {
      assert!(a.get(Platform::Twitch, name).is_none());
    }
    assert_eq!(a.len(), 1, "bad names make no entry");
    assert!(a.take_wanted().len() == 1);
    // an answer nobody asked for is dropped
    assert!(!a.put(Platform::Twitch, "carol", Some(OK.into())));
    assert_eq!(a.len(), 1);
  }

  #[test]
  fn youtube_is_never_asked() {
    let mut a = Avatars::default();
    assert!(a.get(Platform::Youtube, "bob").is_none());
    assert!(a.take_wanted().is_empty());
  }

  // a youtube line brings its chatter's picture (names redacted): it is kept for the name's other lines, never asked
  // for, and only an allowlisted image is kept
  #[test]
  fn a_youtube_picture_comes_with_the_line_and_is_kept_for_the_name() {
    let yt = "https://yt4.ggpht.com/aaaa=s64-c-k-c0x00ffffff-no-rj";
    let mut a = Avatars::default();
    assert!(a.get(Platform::Youtube, "@alpha_one").is_none(), "no picture yet: the stand-in");
    assert!(a.put(Platform::Youtube, "@alpha_one", Some(yt.into())), "older lines of the name redraw");
    assert_eq!(a.get(Platform::Youtube, "@alpha_one"), Some(yt));
    assert!(!a.put(Platform::Youtube, "@alpha_one", Some(yt.into())), "the same picture again changes nothing");
    assert!(a.take_wanted().is_empty(), "youtube is never asked");
    for bad in [Some("https://evil.example/a.png"), Some("http://yt4.ggpht.com/a"), None] {
      assert!(!a.put(Platform::Youtube, "@beta", bad.map(Into::into)));
    }
    assert!(!a.put(Platform::Youtube, "", Some(yt.into())));
    assert_eq!(a.len(), 1, "only the good picture made an entry");
  }

  #[test]
  fn youtube_names_share_the_five_hundred() {
    let yt = "https://yt4.ggpht.com/aaaa=s64-c-k";
    let mut a = Avatars::default();
    for i in 0..MAX + 20 {
      a.put(Platform::Youtube, &format!("@n{i}"), Some(yt.into()));
    }
    assert_eq!(a.len(), MAX);
    assert_eq!(a.get(Platform::Youtube, &format!("@n{}", MAX + 19)), Some(yt), "the newest stay");
  }

  #[test]
  fn the_cache_stops_at_five_hundred() {
    let mut a = Avatars::default();
    for i in 0..MAX + 50 {
      a.get(Platform::Twitch, &format!("u{i}"));
    }
    assert_eq!(a.len(), MAX);
    // the newest are kept, the oldest went
    assert!(a.maps[0].contains_key("u549") && !a.maps[0].contains_key("u0"));
  }

  #[test]
  fn a_miss_waits_before_asking_again() {
    let mut a = Avatars::default();
    a.get(Platform::Twitch, "bob");
    a.take_wanted();
    a.put(Platform::Twitch, "bob", None);
    assert!(a.get(Platform::Twitch, "bob").is_none());
    assert!(a.take_wanted().is_empty());
    a.maps[0].get_mut("bob").unwrap().at -= RETRY + Duration::from_secs(1);
    a.get(Platform::Twitch, "bob");
    assert_eq!(a.take_wanted().len(), 1);
  }

  #[test]
  fn the_stand_in_is_an_ansi_initial() {
    let (c, ink) = tile("bob", "bob");
    assert_eq!(c, 'B');
    assert!([theme::WHITE, theme::BLUE, theme::GREEN, theme::YELLOW, theme::RED, theme::MAGENTA].contains(&ink));
    assert_eq!(tile("x", "").0, '?');
    assert_eq!(tile("bob", "bob"), tile("bob", "Bob"));
  }
}
