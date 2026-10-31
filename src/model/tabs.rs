// the built-in tabs, in the ext's order (tab-registry.js): feed, whispers, mentions, then your channels.
// the names are the reserved ids a channel may never take
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
  Chat,
  Feed,
  Whispers,
  Mentions,
}

pub const BUILTIN: [Kind; 3] = [Kind::Feed, Kind::Whispers, Kind::Mentions];

impl Kind {
  pub fn name(self) -> &'static str {
    match self {
      Kind::Chat => "",
      Kind::Feed => "feed",
      Kind::Whispers => "whispers",
      Kind::Mentions => "mentions",
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
