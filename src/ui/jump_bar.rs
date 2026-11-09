// the jump-to-bottom bar (ext #hs-mc-new-msgs): a small square chip at the bottom middle of the chat
// while you are scrolled up. `▼ 3 new` counts the lines that came in below, `▼ resume` when none did.
// mention yellow on black with a yellow edge, bold; the pointer on it turns it white with black text
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;
use crate::i18n::{f, t};

// the ext's offsets: 12px above the bottom, 4px / 12px of padding
const LIFT: i32 = 12;
const PAD_Y: i32 = 4;
const PAD_X: i32 = 12;

pub fn label(unread: usize) -> String {
  if unread == 0 { t("ui.jump.resume").into() } else { f("ui.jump.new", &[("n", &unread.to_string())]) }
}

// where the bar sits in the chat `area`; empty when the area is too small to hold it
pub fn rect(text: &mut Text, area: Rect, unread: usize) -> Rect {
  let l = text.layout(&label(unread), &[], &[], f32::MAX);
  let (w, h) = (l.width().ceil() as i32 + 2 * PAD_X, text.line_px as i32 + 2 * PAD_Y);
  if w > area.w || h + LIFT > area.h {
    return Rect::default();
  }
  Rect::new(area.x + (area.w - w) / 2, area.bottom() - LIFT - h, w, h)
}

pub fn draw(c: &mut Canvas, text: &mut Text, r: Rect, unread: usize, hover: bool) {
  let (bg, ink) = if hover { (theme::WHITE, theme::BLACK) } else { (theme::BLACK, theme::YELLOW) };
  c.fill(r, bg);
  c.outline(r, r, if hover { theme::WHITE } else { theme::YELLOW });
  let s = label(unread);
  let l = text.layout(&s, &[Span { range: 0..s.len(), ink: Ink(ink), bold: true }], &[], f32::MAX);
  text.draw(&l, c, (r.x + PAD_X) as f32, (r.y + PAD_Y) as f32, r, |_, _| {});
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn words_and_place() {
    assert_eq!(label(0), "▼ resume");
    assert_eq!(label(12), "▼ 12 new");
    let mut t = Text::new(1.0);
    let area = Rect::new(10, 50, 400, 300);
    let r = rect(&mut t, area, 7);
    assert!(!r.is_empty());
    assert_eq!(r.bottom(), area.bottom() - LIFT);
    assert!((r.x - area.x - (area.right() - r.right())).abs() <= 1, "centered");
    assert!(rect(&mut t, Rect::new(0, 0, 20, 300), 7).is_empty(), "a sliver holds none");
    assert!(rect(&mut t, Rect::new(0, 0, 400, 20), 7).is_empty());
  }

  #[test]
  fn yellow_on_black_and_white_on_hover() {
    let mut t = Text::new(1.0);
    let mut c = Canvas::new(300, 120);
    let r = rect(&mut t, Rect::new(0, 0, 300, 120), 3);
    draw(&mut c, &mut t, r, 3, false);
    assert!(c.inked(theme::YELLOW) > 0 && c.inked(theme::WHITE) == 0);
    draw(&mut c, &mut t, r, 3, true);
    assert!(c.inked(theme::WHITE) > (r.w * r.h) as usize / 2 && c.inked(theme::YELLOW) == 0, "white chip, no yellow");
  }
}
