// which fill a row wears (heatsync shared/brand/tokens.json; ext styles/21-brand-rows.css + fill-family.js).
// a notification row is a solid fill from the dim set, the dim twin of its meaning: olive stream events,
// purple new people, navy support, teal channel points + the broadcaster's voice, red for-you. plain chat
// never gets one. a filled row skips zebra, keeps its label's own colour and writes everything else white
use crate::model::filter;
use crate::model::message::{Kind, Message, Notice, flag};
use crate::ui::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Fill {
  pub bg: u32,
  // the row's label (system line, first-message words, chips): the fill's own colour
  pub label: u32,
}

const OLIVE: Fill = Fill { bg: theme::FILL_OLIVE, label: theme::LABEL_OLIVE };
const PURPLE: Fill = Fill { bg: theme::FILL_PURPLE, label: theme::LABEL_PURPLE };
const NAVY: Fill = Fill { bg: theme::FILL_NAVY, label: theme::LABEL_NAVY };
const TEAL: Fill = Fill { bg: theme::FILL_TEAL, label: theme::LABEL_TEAL };
const RED: Fill = Fill { bg: theme::MENTION_BG, label: theme::LABEL_RED };

// the ext's order (rowFill): for-you, a rule, new people, the broadcaster's voice, support, then a stream
// line (olive, its label yellow)
pub(super) fn of(m: &Message, look: u8) -> Option<Fill> {
  let x = m.x.as_deref();
  if m.has(flag::ALERT) || x.is_some_and(|x| x.held.is_some()) {
    return Some(RED);
  }
  if look & filter::MARKED != 0 {
    return Some(Fill { bg: theme::rule_fill(filter::mark_ink(look)), label: theme::LABEL_RED });
  }
  let n = m.notice();
  if m.has(flag::FIRST) || n == Notice::Raid || x.is_some_and(|x| x.intro) {
    return Some(PURPLE);
  }
  if m.has(flag::HIGHLIGHTED) || matches!(n, Notice::Announce | Notice::Redeem) || x.is_some_and(|x| !x.fx.is_empty()) {
    return Some(TEAL);
  }
  if n != Notice::Plain {
    return Some(NAVY);
  }
  (m.kind == Kind::Event).then_some(OLIVE)
}
