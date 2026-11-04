// badges as text (ext textBadges): the ext's BADGE_SHORT words for its BADGE_STYLES roles, and nothing
// else. a role with no entry draws nothing (a channel's own badge names are ids, not words). colored
// text on black, never a tile: the ext's tile colors map to the palette's nearest ansi color
use super::theme;

pub fn chip(role: &str) -> Option<(&'static str, u32)> {
  Some(match role {
    "broadcaster" => ("bc", theme::RED),
    "moderator" => ("mod", theme::MOD_BADGE),
    "vip" => ("vip", theme::MAGENTA),
    "subscriber" => ("sub", theme::MAGENTA),
    "predictions" => ("pred", theme::BLUE),
    "premium" => ("prime", theme::BLUE),
    "admin" => ("admin", theme::YELLOW),
    "staff" => ("staff", theme::YELLOW),
    "global_mod" => ("gmod", theme::WHITE),
    "partner" => ("ptnr", theme::TWITCH),
    "bits-leader" => ("bits", theme::YELLOW),
    "sub-gifter" | "sub_gifter" => ("gift", theme::MAGENTA),
    "artist" => ("art", theme::YELLOW),
    "turbo" => ("turbo", theme::MAGENTA),
    "founder" => ("fdr", theme::MAGENTA),
    "og" => ("og", theme::GREEN),
    "verified" => ("vrfd", theme::GREEN),
    _ => return None,
  })
}

// youtube's chips (ext ytBadgeStyles): a star for the owner, crossed swords for a moderator. members and
// verified draw nothing there (the server sends no picture for them)
pub fn youtube_chip(role: &str) -> Option<(&'static str, u32)> {
  match role {
    "broadcaster" | "owner" => Some(("\u{2606}", theme::YELLOW)),
    "moderator" => Some(("\u{2694}", theme::BLUE)),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_ext_words_and_only_the_styled_roles() {
    assert_eq!(chip("moderator"), Some(("mod", theme::MOD_BADGE)));
    assert_eq!(chip("subscriber").map(|c| c.0), Some("sub"));
    assert_eq!(chip("vip").map(|c| c.0), Some("vip"));
    assert_eq!(chip("broadcaster").map(|c| c.0), Some("bc"));
    // BADGE_SHORT has it but BADGE_STYLES doesn't: the ext draws nothing
    assert_eq!(chip("first-msg"), None);
    assert_eq!(chip("enchantedbigboiboxers"), None);
  }

  #[test]
  fn youtube_draws_owner_and_moderator_only() {
    assert_eq!(youtube_chip("broadcaster"), Some(("\u{2606}", theme::YELLOW)));
    assert_eq!(youtube_chip("owner"), youtube_chip("broadcaster"));
    assert_eq!(youtube_chip("moderator"), Some(("\u{2694}", theme::BLUE)));
    assert_eq!((youtube_chip("member"), youtube_chip("verified")), (None, None));
  }

  #[test]
  fn every_word_is_lowercase_and_short() {
    for r in ["broadcaster", "moderator", "vip", "subscriber", "predictions", "premium", "admin", "staff"] {
      let (w, _) = chip(r).unwrap();
      assert!(w == w.to_lowercase() && (1..=5).contains(&w.len()), "{r}");
    }
  }
}
