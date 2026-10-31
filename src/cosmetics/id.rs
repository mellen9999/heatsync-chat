// whose cosmetics: heatsync's paint ids live in per-platform namespaces (ext paintIdFor / ID-SPACE
// SAFETY). a kick numeric id can equal an unrelated twitch id, so the platform is part of the key
// everywhere (cache, art, queues) and a bare number never crosses over. wire form = what
// `/api/paints` and `/api/cosmetics/batch` take: `123`, `kick_123`, `yt_UC…`, or a native users.id
use crate::model::message::{Message, Platform};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum PaintId {
  Twitch(u64),
  Kick(u64),
  Youtube(Box<str>),
  // a heatsync-native account: its own users.id, never looked up at 7tv
  Native(Box<str>),
}

fn digits(s: &str) -> Option<u64> {
  (!s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()))
    .then(|| s.parse().ok())
    .flatten()
    .filter(|n| *n > 0)
}

fn uc(s: &str) -> bool {
  s.len() == 24 && s.starts_with("UC") && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

impl PaintId {
  // an id as the server writes it (users.id): bare digits are twitch, `kick_`/`yt_` carry their own namespace
  pub fn from_wire(s: &str) -> Option<PaintId> {
    if let Some(n) = digits(s) {
      return Some(PaintId::Twitch(n));
    }
    if let Some(n) = s.strip_prefix("kick_").and_then(digits) {
      return Some(PaintId::Kick(n));
    }
    s.strip_prefix("yt_").filter(|c| uc(c)).map(|c| PaintId::Youtube(c.into()))
  }

  // a raw platform id plus the platform it came from, the ext's isPaintLookupSafeId: an id that
  // already names its namespace is kept; a bare one is read in its own platform's space only
  pub fn of(platform: &str, id: &str) -> Option<PaintId> {
    if id.starts_with("kick_") || id.starts_with("yt_") {
      return Self::from_wire(id);
    }
    match platform {
      "twitch" => digits(id).map(PaintId::Twitch),
      "kick" => digits(id).map(PaintId::Kick),
      "youtube" => uc(id).then(|| PaintId::Youtube(id.into())),
      "heatsync" => {
        (id.len() <= 64 && !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')))
          .then(|| PaintId::Native(id.into()))
      }
      _ => None,
    }
  }

  // a chat line's sender: only the platforms that send an id of their own
  pub fn of_message(m: &Message) -> Option<PaintId> {
    if let Some(p) = m.x.as_deref().and_then(|x| x.paint.as_ref()) {
      return Some(p.clone());
    }
    let platform = match m.platform {
      Platform::Twitch => "twitch",
      Platform::Kick => "kick",
      Platform::Youtube => "youtube",
    };
    Self::of(platform, &m.user_id)
  }

  pub fn wire(&self) -> String {
    match self {
      PaintId::Twitch(n) => n.to_string(),
      PaintId::Kick(n) => format!("kick_{n}"),
      PaintId::Youtube(c) => format!("yt_{c}"),
      PaintId::Native(i) => i.to_string(),
    }
  }

  // 7tv keys its cosmetics by twitch and kick ids alone
  pub fn seven_tv(&self) -> bool {
    matches!(self, PaintId::Twitch(_) | PaintId::Kick(_))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::HashSet;

  #[test]
  fn platforms_never_share_a_key() {
    // the same number on three platforms is three different people
    let ids = [
      PaintId::of("twitch", "5").unwrap(),
      PaintId::of("kick", "5").unwrap(),
      PaintId::of("heatsync", "5").unwrap(),
      PaintId::from_wire("kick_5").unwrap(),
    ];
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 3, "kick 5 and kick_5 are one person");
    assert_eq!(ids[1], ids[3]);
    assert_ne!(ids[0], ids[2]);
    assert_eq!(ids[1].wire(), "kick_5");
    // a bare number is read in its own platform's space, never another's
    assert_eq!(PaintId::of("kick", "5").unwrap().wire(), "kick_5");
    assert_eq!(PaintId::of("twitch", "5").unwrap().wire(), "5");
    // an id already carrying its namespace is not mapped twice (no kick_kick_5)
    assert_eq!(PaintId::of("kick", "kick_5").unwrap().wire(), "kick_5");
    // ...and one carrying another platform's namespace stays in it
    assert_eq!(PaintId::of("twitch", "kick_5"), PaintId::from_wire("kick_5"));
  }

  #[test]
  fn only_well_formed_ids_get_a_key() {
    let yt = "UC0123456789abcdefghij_-";
    assert_eq!(PaintId::of("youtube", yt).unwrap().wire(), format!("yt_{yt}"));
    assert_eq!(PaintId::from_wire(&format!("yt_{yt}")), PaintId::of("youtube", yt));
    for (p, id) in [
      ("youtube", "UCshort"),
      ("youtube", "123"),
      ("twitch", "0"),
      ("twitch", "12x"),
      ("twitch", ""),
      ("twitch", "99999999999999999999999"),
      ("kick", "-1"),
      ("heatsync", "a b"),
      ("heatsync", ""),
      ("tiktok", "5"),
    ] {
      assert!(PaintId::of(p, id).is_none(), "{p} {id}");
    }
    assert!(PaintId::from_wire("kick_").is_none() && PaintId::from_wire("yt_x").is_none());
    assert!(PaintId::of("heatsync", "abc-9_Z").is_some());
  }

  #[test]
  fn only_twitch_and_kick_go_to_7tv() {
    assert!(PaintId::Twitch(1).seven_tv() && PaintId::Kick(1).seven_tv());
    assert!(!PaintId::Native("1".into()).seven_tv() && !PaintId::Youtube("x".into()).seven_tv());
  }
}
