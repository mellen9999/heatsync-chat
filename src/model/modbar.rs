// the ext's hover toolbar buttons (mod-toolbar.js MOD_BUTTON_CATALOG), in its order. the picked ones are a
// bitmask over this table; the settings file keeps their ids (the ext's own values), hostile or stale
// ids are dropped on the way in. bt and mt (the ext's blocked-terms and mod-tools shortcuts) are left out:
// /blocked and /modtools are their doors here
use crate::model::message::Platform;
use crate::model::modcmd::Verb;

pub struct Btn {
  pub id: &'static str,
  pub label: &'static str,
  pub verb: Verb,
  // acts on one line: needs that line's platform id
  pub needs_msg: bool,
  // kick's shortest timeout is a minute, so a purge there would only be a timeout
  pub not_kick: bool,
}

const fn b(id: &'static str, label: &'static str, verb: Verb, needs_msg: bool, not_kick: bool) -> Btn {
  Btn { id, label, verb, needs_msg, not_kick }
}

pub const CATALOG: [Btn; 9] = [
  b("delete_message", "x", Verb::Delete, true, false),
  b("timeout_1m", "1m", Verb::Timeout(60), false, false),
  b("timeout_10m", "10m", Verb::Timeout(600), false, false),
  b("timeout_1h", "1h", Verb::Timeout(3600), false, false),
  b("timeout_24h", "24h", Verb::Timeout(86400), false, false),
  b("timeout_7d", "7d", Verb::Timeout(604800), false, false),
  b("purge", "purge", Verb::Purge, false, true),
  b("ban", "\u{26d4}", Verb::Ban, false, false),
  b("unban", "\u{2713}", Verb::Unban, false, false),
];

// a list longer than this is not a list of ours
const MAX_TEXT: usize = 256;
const MAX_WORDS: usize = 32;

// the buttons named by ids, labels or both, split on spaces and commas; unknown words are counted, not kept
pub fn parse(text: &str) -> (u16, usize) {
  let (mut mask, mut bad) = (0u16, 0usize);
  let end = text.char_indices().nth(MAX_TEXT).map_or(text.len(), |(i, _)| i);
  for w in text[..end].split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).take(MAX_WORDS) {
    match CATALOG.iter().position(|c| c.id == w || c.label == w) {
      Some(i) => mask |= 1 << i,
      None => bad += 1,
    }
  }
  (mask, bad)
}

// the settings file's form: the ext's ids, comma apart
pub fn ids(mask: u16) -> String {
  let ids: Vec<&str> = CATALOG.iter().enumerate().filter(|(i, _)| mask >> i & 1 == 1).map(|(_, c)| c.id).collect();
  ids.join(",")
}

// the screen's form: the buttons as they read on the bar
pub fn labels(mask: u16) -> String {
  let ls: Vec<&str> = CATALOG.iter().enumerate().filter(|(i, _)| mask >> i & 1 == 1).map(|(_, c)| c.label).collect();
  ls.join(" ")
}

// the picked buttons that suit this line (ext gateModButtons)
pub fn visible(mask: u16, platform: Platform, has_msg: bool) -> u16 {
  let mut out = 0;
  for (i, c) in CATALOG.iter().enumerate() {
    let ok = mask >> i & 1 == 1 && (has_msg || !c.needs_msg) && !(c.not_kick && platform == Platform::Kick);
    out |= u16::from(ok) << i;
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  const ALL: u16 = (1 << CATALOG.len()) - 1;

  #[test]
  fn ids_and_labels_both_pick_and_strangers_are_dropped() {
    assert_eq!(parse("x 1m,ban"), (1 | 1 << 1 | 1 << 7, 0));
    assert_eq!(parse("timeout_7d purge \u{2713}"), (1 << 5 | 1 << 6 | 1 << 8, 0));
    // bt / mt are not here, and neither is anything made up
    assert_eq!(parse("blocked_terms mt ban nope"), (1 << 7, 3));
    assert_eq!(parse(""), (0, 0));
  }

  #[test]
  fn hostile_text_is_bounded() {
    let long = "ban ".repeat(10_000);
    assert_eq!(parse(&long), (1 << 7, 0));
    let junk = "zz ".repeat(10_000);
    assert!(parse(&junk).1 <= MAX_WORDS);
    // a cut inside a multibyte char must not panic
    let wide = "\u{26d4}".repeat(MAX_TEXT + 5);
    assert_eq!(parse(&wide).0 & !ALL, 0);
  }

  #[test]
  fn the_file_form_round_trips_in_catalog_order() {
    let m = parse("ban timeout_1m x").0;
    assert_eq!(ids(m), "delete_message,timeout_1m,ban");
    assert_eq!(parse(&ids(m)), (m, 0));
    assert_eq!(labels(m), "x 1m \u{26d4}");
  }

  #[test]
  fn gates_follow_the_ext() {
    let all = ALL;
    assert_eq!(visible(all, Platform::Twitch, true), all);
    // no line id: no delete. kick: no purge
    assert_eq!(visible(all, Platform::Twitch, false), all & !1);
    assert_eq!(visible(all, Platform::Kick, true), all & !(1 << 6));
    assert_eq!(visible(0, Platform::Twitch, true), 0);
  }
}
