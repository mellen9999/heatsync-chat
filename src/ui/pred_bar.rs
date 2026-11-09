// the prediction banner, one row under the tabs (ext chat banner): `prediction` label, title and each
// outcome's share, then the countdown (or locked) at the right. click it to open the whole view
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;
use crate::model::pred::Banner;

pub fn draw(c: &mut Canvas, text: &mut Text, area: Rect, b: &Banner) {
  c.fill(area, theme::BLACK);
  let tail = format!(" {} ", b.tail);
  // locked and closing are the warnings; a running clock is plain
  // a countdown starts with a digit; `locked` and `closing` (in any language) are the words
  let ink = if b.tail.starts_with(|c: char| c.is_ascii_digit()) { theme::WHITE } else { theme::YELLOW };
  let t = text.layout(&tail, &[Span { range: 0..tail.len(), ink: Ink(ink), bold: true }], &[], f32::MAX);
  let w = t.full_width().ceil() as i32;
  let x = Rect::new(area.right() - w, area.y, w, area.h);
  text.draw(&t, c, x.x as f32, x.y as f32, x, |_, _| {});
  let label = crate::i18n::t("ui.pred.label");
  let s = format!("{label}{}", b.body);
  let l = text.layout(&s, &[Span { range: 0..label.len(), ink: Ink(theme::DIM), bold: false }], &[], f32::MAX);
  let clip = Rect::new(area.x, area.y, (area.w - w).max(0), area.h);
  text.draw(&l, c, area.x as f32 + theme::PAD_X, area.y as f32, clip, |_, _| {});
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_banner_paints_its_row_and_keeps_the_clock_at_the_right() {
    let mut text = Text::new(1.0);
    let line = text.line_px as i32;
    let mut c = Canvas::new(300, 60);
    let area = Rect::new(0, 20, 300, line);
    let b = Banner { body: "a very long title ".repeat(30), tail: "locked".into() };
    draw(&mut c, &mut text, area, &b);
    let lit = |x0: i32, x1: i32| {
      (area.y..area.bottom()).any(|y| (x0..x1).any(|x| c.px[(y * 300 + x) as usize] != theme::BLACK))
    };
    assert!(lit(0, 100), "the label and title paint");
    assert!(lit(250, 300), "the clock paints at the right edge");
    // nothing leaks out of the row
    assert!((0..300).all(|x| c.px[((area.bottom() + 1) * 300 + x) as usize] == theme::BLACK));
    assert!((0..300).all(|x| c.px[((area.y - 1) * 300 + x) as usize] == theme::BLACK));
  }
}
