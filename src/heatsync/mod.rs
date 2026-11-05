// heatsync's own data and the rules for when it renders
pub mod content;
pub mod flair;
pub mod heat;
pub mod late_refs;
pub mod marks;
pub mod senders;

pub const MAX_NAME: usize = 100;

// an emote name is one token: no whitespace/control bytes, not a command (`/me`) or bot prefix (`.`)
pub fn valid_name(w: &str) -> bool {
  !w.is_empty()
    && w.len() <= MAX_NAME
    && !w.starts_with(['/', '.'])
    && !w.chars().any(|c| c.is_whitespace() || c.is_control())
}

// an image only a heatsync inventory could have put on a line: heatsync's own `/e/hs/…`, an upload, its
// cdn. a provider's mirror (`/e/7tv/…`) is the channel's emote, not a person's. such an image is never
// trusted from a server's word for a line: it draws only through the sender's own set (emotes::resolve)
pub fn inventory_url(url: &str) -> bool {
  const MIRRORS: [&str; 5] = ["/e/7tv/", "/e/bttv/", "/e/ffz/", "/e/twitch/", "/e/kick/"];
  let Some(host) = crate::images::url::host(url) else { return false };
  let ours = host.eq_ignore_ascii_case("heatsync.org") || host.to_ascii_lowercase().ends_with(".heatsync.org");
  let path = &url["https://".len() + host.len()..];
  ours && !MIRRORS.iter().any(|m| path.starts_with(m))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn inventory_urls() {
    for yes in [
      "https://heatsync.org/e/hs/abc",
      "https://HeatSync.org/uploads/a.webp",
      "https://cdn.heatsync.org/uploads/a.webp",
      "https://heatsync.org/api/img?url=x",
    ] {
      assert!(inventory_url(yes), "{yes}");
    }
    for no in [
      "https://heatsync.org/e/7tv/01ABC",
      "https://heatsync.org/e/bttv/5",
      "https://heatsync.org/e/ffz/5",
      "https://heatsync.org/e/twitch/25",
      "https://heatsync.org/e/kick/5",
      "https://cdn.7tv.app/emote/x/1x.webp",
      "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0",
      "https://files.kick.com/emotes/5/fullsize",
      "https://evilheatsync.org/e/hs/a",
      "https://heatsync.org.evil.example/e/hs/a",
      "http://heatsync.org/e/hs/a",
      "",
    ] {
      assert!(!inventory_url(no), "{no}");
    }
  }

  #[test]
  fn names() {
    assert!(valid_name("PogU"));
    for bad in ["", "a b", "/me", ".w", "x\u{7}", &"x".repeat(101)] {
      assert!(!valid_name(bad), "{bad:?}");
    }
  }
}
