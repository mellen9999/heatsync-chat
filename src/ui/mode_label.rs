// vim's mode line for the vi keys: which of the three modes you are in, plus the half-typed command.
// shown at the left of the input bar, or in the tab bar when auto-hide has folded the bar away.
// plain (non-vi) mode shows nothing
use super::text::{Ink, Span};
use super::theme;
use crate::model::vi::Mode;

// (text, bold)
pub type Seg = (&'static str, bool);

// the widest short label: the bar's left gutter is sized to it so typed text never shifts
pub const WIDEST: &str = "-- normal --";

// short: the bar already holds typed text, so keep to the gutter width
pub fn segs(vi: bool, mode: Mode, selecting: bool, short: bool) -> Vec<Seg> {
  if !vi {
    return Vec::new();
  }
  match mode {
    Mode::Insert => vec![("-- insert --", false)],
    // vi normal inside the input box: "normal" is the chat mode the user lives in
    Mode::Normal => vec![("-- edit --", false)],
    Mode::Chat if selecting && short => vec![("-- select --", false)],
    Mode::Chat if selecting => vec![
      ("-- select: ", false),
      ("r", true),
      (" reply · ", false),
      ("m", true),
      (" menu · ", false),
      ("c", true),
      (" card · ", false),
      ("esc", true),
      (" --", false),
    ],
    Mode::Chat => vec![(WIDEST, false)],
  }
}

pub fn text(s: &[Seg]) -> String {
  s.iter().map(|x| x.0).collect()
}

// dim, with the key names bold
pub fn spans(s: &[Seg]) -> Vec<Span> {
  let mut at = 0;
  s.iter()
    .map(|&(t, bold)| {
      let r = at..at + t.len();
      at = r.end;
      Span { range: r, ink: Ink(theme::DIM), bold }
    })
    .collect()
}

// vim's showcmd: the count, operator and waiting char typed so far, last few only
pub fn showcmd(count: usize, op: Option<(usize, char)>, pending: Option<char>) -> String {
  let mut s = String::new();
  if let Some((n, op)) = op {
    if n > 1 {
      s.push_str(&n.to_string());
    }
    s.push(op);
  }
  if count > 0 {
    s.push_str(&count.to_string());
  }
  s.extend(pending);
  let n = s.chars().count();
  s.chars().skip(n.saturating_sub(5)).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn plain_shows_nothing() {
    for m in [Mode::Chat, Mode::Insert, Mode::Normal] {
      assert!(segs(false, m, true, false).is_empty());
    }
  }

  #[test]
  fn per_mode() {
    assert_eq!(text(&segs(true, Mode::Insert, false, false)), "-- insert --");
    assert_eq!(text(&segs(true, Mode::Normal, false, false)), "-- edit --");
    assert_eq!(text(&segs(true, Mode::Chat, false, false)), "-- normal --");
    assert_eq!(text(&segs(true, Mode::Chat, true, false)), "-- select: r reply · m menu · c card · esc --");
    assert_eq!(text(&segs(true, Mode::Chat, true, true)), "-- select --");
  }

  #[test]
  fn select_keys_bold() {
    let bold: Vec<_> = segs(true, Mode::Chat, true, false).into_iter().filter(|s| s.1).map(|s| s.0).collect();
    assert_eq!(bold, ["r", "m", "c", "esc"]);
    let s = segs(true, Mode::Chat, true, false);
    let sp = spans(&s);
    assert_eq!(sp.len(), s.len());
    assert_eq!(sp.last().map(|x| x.range.end), Some(text(&s).len()));
  }

  #[test]
  fn short_labels_fit_the_gutter() {
    for m in [Mode::Chat, Mode::Insert, Mode::Normal] {
      assert!(text(&segs(true, m, true, true)).chars().count() <= WIDEST.chars().count());
    }
  }

  #[test]
  fn pending() {
    assert_eq!(showcmd(0, None, None), "");
    assert_eq!(showcmd(0, Some((1, 'd')), None), "d");
    assert_eq!(showcmd(2, None, None), "2");
    assert_eq!(showcmd(0, Some((3, 'd')), None), "3d");
    assert_eq!(showcmd(2, Some((1, 'd')), None), "d2");
    assert_eq!(showcmd(0, None, Some('f')), "f");
    assert_eq!(showcmd(3, None, Some('r')), "3r");
    assert_eq!(showcmd(9999, Some((9999, 'd')), Some('f')), "9d9999f".chars().skip(2).collect::<String>());
  }
}
