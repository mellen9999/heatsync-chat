// whose paint you chose not to see (ext hiddenPaintUsers): their name wears its plain color instead.
// keyed by paint id (each platform in its own namespace), one wire id per line in a small file
use std::collections::BTreeSet;

use super::id::PaintId;

pub const MAX: usize = 2000;

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Hidden {
  // wire form: sorted, so the file is stable
  set: BTreeSet<String>,
}

impl Hidden {
  pub fn has(&self, id: &PaintId) -> bool {
    !self.set.is_empty() && self.set.contains(&id.wire())
  }

  // flip it; true = hidden now. a full list refuses a new hide (false, unchanged)
  pub fn toggle(&mut self, id: &PaintId) -> bool {
    let w = id.wire();
    if self.set.remove(&w) {
      return false;
    }
    if self.set.len() >= MAX {
      return false;
    }
    self.set.insert(w)
  }

  pub fn render(&self) -> String {
    self.set.iter().map(|w| format!("{w}\n")).collect()
  }

  // tolerant: a line that is not a paint id is dropped
  pub fn parse(s: &str) -> Self {
    let set = s
      .lines()
      .map(str::trim)
      .filter_map(|l| PaintId::from_wire(l).or_else(|| PaintId::of("heatsync", l)))
      .map(|id| id.wire())
      .take(MAX)
      .collect();
    Hidden { set }
  }

  // the account's hiddenPaintUsers: a json array of paint ids, as the ext keeps it
  pub fn to_json(&self) -> String {
    serde_json::to_string(&self.set).unwrap_or_else(|_| "[]".into())
  }

  // the account's list in. None = not a list of at most MAX; an entry that is not a paint id is dropped
  pub fn from_json(text: &str) -> Option<Self> {
    let serde_json::Value::Array(a) = serde_json::from_str(text).ok()? else { return None };
    if a.len() > MAX {
      return None;
    }
    let set = a
      .iter()
      .filter_map(serde_json::Value::as_str)
      .filter_map(|l| PaintId::from_wire(l).or_else(|| PaintId::of("heatsync", l)))
      .map(|id| id.wire())
      .collect();
    Some(Hidden { set })
  }

  pub fn load() -> Self {
    crate::store::read("hidden_paints").map(|s| Self::parse(&s)).unwrap_or_default()
  }

  pub fn save(&self) {
    if let Err(e) = crate::store::write("hidden_paints", &self.render()) {
      eprintln!("save hidden paints: {e}");
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn toggles_and_namespaces_stay_apart() {
    let mut h = Hidden::default();
    assert!(!h.has(&PaintId::Twitch(7)));
    assert!(h.toggle(&PaintId::Twitch(7)));
    assert!(h.has(&PaintId::Twitch(7)));
    assert!(!h.has(&PaintId::Kick(7)), "a kick id equal to a twitch id is someone else");
    assert!(!h.toggle(&PaintId::Twitch(7)));
    assert!(!h.has(&PaintId::Twitch(7)));
  }

  #[test]
  fn round_trips_and_survives_garbage() {
    let mut h = Hidden::default();
    for id in [PaintId::Twitch(7), PaintId::Kick(9), PaintId::Native("abc_1".into())] {
      h.toggle(&id);
    }
    let back = Hidden::parse(&h.render());
    assert!(back.has(&PaintId::Kick(9)) && back.has(&PaintId::Twitch(7)) && back.has(&PaintId::Native("abc_1".into())));
    let g = Hidden::parse("\n  \nnot an id!\n12\n\u{0}\nkick_x\n");
    assert!(g.has(&PaintId::Twitch(12)));
    assert_eq!(g.set.len(), 1);
  }

  #[test]
  fn the_account_list_round_trips_and_is_bounded() {
    let mut h = Hidden::default();
    for id in [PaintId::Twitch(7), PaintId::Kick(9), PaintId::Native("abc_1".into())] {
      h.toggle(&id);
    }
    assert_eq!(Hidden::from_json(&h.to_json()), Some(Hidden::parse(&h.render())));
    assert_eq!(Hidden::default().to_json(), "[]");
    // junk entries drop, the rest stay; not a list, or too long a list, is refused whole
    let g = Hidden::from_json(r#"["12", 5, null, "not an id!", "kick_3", ["x"]]"#).unwrap();
    assert!(g.has(&PaintId::Twitch(12)) && g.has(&PaintId::Kick(3)) && g.set.len() == 2);
    for bad in ["{}", "\"12\"", "[", "", "null"] {
      assert!(Hidden::from_json(bad).is_none(), "{bad}");
    }
    let long = format!("[{}]", vec!["\"1\""; MAX + 1].join(","));
    assert!(Hidden::from_json(&long).is_none());
  }

  #[test]
  fn a_full_list_refuses_more() {
    let mut h = Hidden::default();
    for i in 1..=MAX as u64 {
      assert!(h.toggle(&PaintId::Twitch(i)));
    }
    assert!(!h.toggle(&PaintId::Twitch(99_999)));
    assert!(!h.has(&PaintId::Twitch(99_999)));
  }
}
