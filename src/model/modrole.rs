// do we moderate a kick room? kick has no way to ask (the ext reads kick.com with your browser
// session, which a standalone app does not have), so we read it off our own lines: kick puts a
// moderator or broadcaster badge on them. youtube has no server-side moderation at all (heatsync's
// capabilities list for it is empty; the ext drives youtube's own page), so it has no role here

// kick badge names that mean "can moderate"
pub fn kick_badges_mod(badges: &[(Box<str>, Box<str>)]) -> bool {
  badges.iter().any(|b| matches!(&*b.0, "moderator" | "broadcaster"))
}

// kick folds a name into its channel slug: lowercase, `_` as `-`
pub fn kick_slug(name: &str) -> String {
  name.trim().trim_start_matches('@').to_ascii_lowercase().replace('_', "-")
}

// is this kick sender us (our linked kick name)?
pub fn kick_is_me(sender: &str, mine: Option<&str>) -> bool {
  mine.is_some_and(|m| !sender.is_empty() && kick_slug(m) == kick_slug(sender))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn b(names: &[&str]) -> Vec<(Box<str>, Box<str>)> {
    names.iter().map(|n| ((*n).into(), "1".into())).collect()
  }

  #[test]
  fn mod_and_broadcaster_badges_count() {
    assert!(kick_badges_mod(&b(&["subscriber", "moderator"])));
    assert!(kick_badges_mod(&b(&["broadcaster"])));
    assert!(!kick_badges_mod(&b(&["subscriber", "vip", "og"])));
    assert!(!kick_badges_mod(&[]));
  }

  #[test]
  fn names_match_as_slugs() {
    assert!(kick_is_me("Some_One", Some("some-one")));
    assert!(kick_is_me("bob", Some("@Bob")));
    assert!(!kick_is_me("bob", Some("bobby")));
    assert!(!kick_is_me("bob", None));
    assert!(!kick_is_me("", Some("")));
  }
}
