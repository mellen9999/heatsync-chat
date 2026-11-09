// the built-in tabs, in the ext's order (tab-registry.js): feed, whispers, mentions, mod log, then your channels.
// the names are the reserved ids a channel may never take

use crate::i18n::t;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
  Chat,
  Feed,
  Whispers,
  Mentions,
  // every mod action seen in your channels (ext modlog)
  ModLog,
}

pub const BUILTIN: [Kind; 4] = [Kind::Feed, Kind::Whispers, Kind::Mentions, Kind::ModLog];

impl Kind {
  pub fn name(self) -> &'static str {
    match self {
      Kind::Chat => "",
      Kind::Feed => "feed",
      Kind::Whispers => "whispers",
      Kind::Mentions => "mentions",
      Kind::ModLog => "modlog",
    }
  }

  // what a tab of this kind is called on screen (`name` is the id that is typed and saved)
  pub fn label(self) -> &'static str {
    match self {
      Kind::Chat => "",
      Kind::Feed => t("chat2.tabs.feed"),
      Kind::Whispers => t("chat2.tabs.whispers"),
      Kind::Mentions => t("chat2.tabs.mentions"),
      Kind::ModLog => t("chat2.tabs.modlog"),
    }
  }

  pub fn parse(s: &str) -> Option<Kind> {
    BUILTIN.into_iter().find(|k| k.name() == s.trim().to_ascii_lowercase())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_round_trip_and_stay_reserved() {
    for k in BUILTIN {
      assert_eq!(Kind::parse(k.name()), Some(k));
      assert!(crate::model::chan::RESERVED.contains(&k.name()));
    }
    assert_eq!(Kind::parse(" Feed "), Some(Kind::Feed));
    assert_eq!(Kind::parse("live"), None);
    assert_eq!(Kind::parse(""), None);
  }
}
