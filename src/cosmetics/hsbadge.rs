// the heatsync badge beside a name (ext lib/badge-catalogue.js, a copy of the site's list). the wire carries
// only the id of the ONE badge a person picked and still has; glyph and color are ours (the ext also words a hover; this window has none on a name mark), so an id this
// build does not know draws nothing. colors are ansi 0-7, glyphs plain text
use serde_json::Value;

use crate::ui::theme;

pub struct Def {
  pub id: &'static str,
  pub glyph: &'static str,
  pub ink: u32,
}

pub static CATALOGUE: [Def; 8] = [
  Def { id: "staff", glyph: "■", ink: theme::WHITE },
  Def { id: "plus_24m", glyph: "★", ink: theme::MAGENTA },
  Def { id: "plus_12m", glyph: "★", ink: theme::YELLOW },
  Def { id: "plus_6m", glyph: "◆", ink: theme::GREEN },
  Def { id: "plus_3m", glyph: "◆", ink: theme::CYAN },
  Def { id: "plus_1m", glyph: "◇", ink: theme::CYAN },
  Def { id: "ember", glyph: "▲", ink: theme::RED },
  Def { id: "early_supporter", glyph: "●", ink: theme::BLUE },
];

impl std::fmt::Debug for Def {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(self.id)
  }
}

// the wire value of one id: a plain `[a-z0-9_]{1,32}` string we know, else nothing
pub fn parse(v: Option<&Value>) -> Option<&'static Def> {
  let id = v?.as_str().filter(|s| (1..=32).contains(&s.len()))?;
  CATALOGUE.iter().find(|d| d.id == id)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn a_known_id_resolves_and_everything_else_draws_nothing() {
    assert_eq!(parse(Some(&json!("plus_12m"))).map(|d| d.glyph), Some("★"));
    for bad in [json!("nope"), json!(""), json!(5), json!(null), json!("x".repeat(40)), json!("PLUS_12M")] {
      assert!(parse(Some(&bad)).is_none(), "{bad}");
    }
    assert!(parse(None).is_none());
  }

  #[test]
  fn every_badge_is_one_glyph() {
    for d in &CATALOGUE {
      assert!(!d.glyph.is_empty() && d.glyph.chars().count() == 1, "{}", d.id);
    }
  }
}
