// the first-run tour (the ext's six cards, adapted): one idea a card, keyboard-driven, shown once per install,
// brought back by /tour. the last card holds the switches that start off. no drawing here
use crate::store;

// the marker file: its being there is the whole record. written when the tour first opens, so a crash or a quit
// halfway never has it greeting you twice
const SEEN: &str = "tour";

pub struct Card {
  // locale keys (`t` them where shown)
  pub title: &'static str,
  // short lines (the card is ~44 characters wide)
  pub lines: &'static [&'static str],
}

pub const CARDS: [Card; 6] = [
  Card {
    title: "settings.tour.right_click.title",
    lines: &["settings.tour.right_click.line1", "settings.tour.right_click.line2"],
  },
  Card { title: "settings.tour.slash.title", lines: &["settings.tour.slash.line1", "settings.tour.slash.line2"] },
  Card { title: "settings.tour.find.title", lines: &["settings.tour.find.line1", "settings.tour.find.line2"] },
  Card { title: "settings.tour.channel.title", lines: &["settings.tour.channel.line1", "settings.tour.channel.line2"] },
  Card {
    title: "settings.tour.settings.title",
    lines: &["settings.tour.settings.line1", "settings.tour.settings.line2", "settings.tour.settings.line3"],
  },
  Card { title: "settings.tour.off.title", lines: &["settings.tour.off.line1"] },
];

pub const LAST: usize = CARDS.len() - 1;

// the switches on the last card: settings rows by id. the same rows settings shows, so the two can't disagree
pub const SWITCHES: [&str; 3] = ["timestamps", "vi", "hover_reply"];

#[derive(Default)]
pub struct Tour {
  pub at: usize,
  // the focused switch on the last card
  pub row: usize,
}

// what a key asked of the tour
#[derive(PartialEq, Eq, Debug)]
pub enum Go {
  Stay,
  // the last card was finished, or the tour was skipped
  Done,
}

impl Tour {
  pub fn last(&self) -> bool {
    self.at == LAST
  }

  // enter / right: the next card; after the last, done
  pub fn next(&mut self) -> Go {
    if self.last() {
      return Go::Done;
    }
    self.at += 1;
    Go::Stay
  }

  pub fn back(&mut self) {
    self.at = self.at.saturating_sub(1);
  }

  // j / k on the switches: stops at the ends
  pub fn step(&mut self, by: i32) {
    if self.last() {
      self.row = self.row.saturating_add_signed(by as isize).min(SWITCHES.len() - 1);
    }
  }
}

pub fn seen() -> bool {
  store::read(SEEN).is_some()
}

pub fn mark_seen() {
  // a failed write is said once by the store; the tour just may come again
  let _ = store::write(SEEN, "1\n");
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::i18n::t;

  #[test]
  fn next_walks_the_six_cards_then_is_done() {
    let mut t = Tour::default();
    for i in 1..CARDS.len() {
      assert_eq!(t.next(), Go::Stay);
      assert_eq!(t.at, i);
    }
    assert!(t.last());
    assert_eq!(t.next(), Go::Done);
    assert_eq!(t.at, LAST, "done stays on the last card");
  }

  #[test]
  fn back_stops_at_the_first_card() {
    let mut t = Tour { at: 2, row: 0 };
    t.back();
    t.back();
    t.back();
    assert_eq!(t.at, 0);
  }

  #[test]
  fn the_switch_cursor_lives_on_the_last_card_and_stops_at_the_ends() {
    let mut t = Tour::default();
    t.step(1);
    assert_eq!(t.row, 0, "only the last card has switches");
    t.at = LAST;
    t.step(-1);
    assert_eq!(t.row, 0);
    for _ in 0..9 {
      t.step(1);
    }
    assert_eq!(t.row, SWITCHES.len() - 1);
  }

  #[test]
  fn every_card_is_lowercase_short_and_every_switch_is_a_real_bool_row() {
    for c in &CARDS {
      assert_eq!(t(c.title), t(c.title).to_lowercase());
      assert!(!c.lines.is_empty() && c.lines.len() <= 3);
      for l in c.lines {
        assert!(t(l).chars().count() <= 44, "{l}");
        assert_eq!(t(l), t(l).to_lowercase());
      }
    }
    let set = crate::model::settings::Settings::default();
    for id in SWITCHES {
      let d = crate::model::schema::def(id).unwrap_or_else(|| panic!("{id}"));
      assert!(d.on(&set).is_some(), "{id} is a switch");
      assert_eq!(d.on(&set), Some(false), "{id} starts off");
    }
  }

  #[test]
  fn seen_is_remembered_across_runs() {
    store::remove(SEEN);
    assert!(!seen());
    mark_seen();
    assert!(seen());
    store::remove(SEEN);
  }
}
