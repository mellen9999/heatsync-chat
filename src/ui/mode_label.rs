// vim's mode line for the vi keys: normal or insert (or the picked line's keys), plus the half-typed command.
// shown at the left of the input bar, or in the tab bar when auto-hide has folded the bar away.
// plain (non-vi) mode shows nothing
use super::text::{Ink, Span};
use super::theme;
use crate::i18n::t;
use crate::model::vi::Mode;

// (text, bold)
pub type Seg = (&'static str, bool);

// the widest short label: the bar's left gutter is sized to it so typed text never shifts
pub fn widest() -> &'static str {
  t("ui.mode.normal")
}

// short: the bar already holds typed text, so keep to the gutter width
pub fn segs(vi: bool, mode: Mode, selecting: bool, short: bool) -> Vec<Seg> {
  if !vi {
    return Vec::new();
  }
  match mode {
    Mode::Insert => vec![(t("ui.mode.insert"), false)],
    Mode::Normal if selecting && short => vec![(t("ui.mode.select"), false)],
    // the keys sit between `*` marks, bold; any wording around them is the translator's
    Mode::Normal if selecting => t("ui.mode.select_keys")
      .split('*')
      .enumerate()
      .filter(|(_, s)| !s.is_empty())
      .map(|(i, s)| (s, i % 2 == 1))
      .collect(),
    Mode::Normal => vec![(widest(), false)],
  }
}

// the widest a short label can be in this language: the gutter is sized to it, so a longer translation of
// `insert` or `select` can't push the typed text around
pub fn widest_px(text: &mut super::text::Text) -> f32 {
  [t("ui.mode.normal"), t("ui.mode.insert"), t("ui.mode.select")]
    .into_iter()
    .map(|s| text.layout(s, &[], &[], f32::MAX).full_width())
    .fold(0.0, f32::max)
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
    for m in [Mode::Insert, Mode::Normal] {
      assert!(segs(false, m, true, false).is_empty());
    }
  }

  #[test]
  fn per_mode() {
    assert_eq!(text(&segs(true, Mode::Insert, false, false)), "-- insert --");
    assert_eq!(text(&segs(true, Mode::Normal, false, false)), "-- normal --");
    assert_eq!(text(&segs(true, Mode::Normal, true, false)), "-- select: r reply · m menu · c card · esc --");
    assert_eq!(text(&segs(true, Mode::Normal, true, true)), "-- select --");
    assert_eq!(text(&segs(true, Mode::Insert, true, false)), "-- insert --");
  }

  #[test]
  fn select_keys_bold() {
    let bold: Vec<_> = segs(true, Mode::Normal, true, false).into_iter().filter(|s| s.1).map(|s| s.0).collect();
    assert_eq!(bold, ["r", "m", "c", "esc"]);
    let s = segs(true, Mode::Normal, true, false);
    let sp = spans(&s);
    assert_eq!(sp.len(), s.len());
    assert_eq!(sp.last().map(|x| x.range.end), Some(text(&s).len()));
  }

  #[test]
  fn short_labels_fit_the_gutter() {
    for m in [Mode::Insert, Mode::Normal] {
      assert!(text(&segs(true, m, true, true)).chars().count() <= widest().chars().count());
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
