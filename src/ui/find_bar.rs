// the chat filter bar, one row at the top of the chat (ext #hs-mc-search-bar): a square 1px box (white while
// it has the keys, dim otherwise), the query or a dim placeholder in it, and the count at the right
use super::canvas::{Canvas, Rect};
use super::input_bar::underline;
use super::text::{Ink, Span, Text};
use super::theme;

pub struct FindBar<'a> {
  pub text: &'a str,
  // caret, a byte in `text`
  pub cursor: usize,
  pub preedit: &'a str,
  pub focused: bool,
  pub hint: &'a str,
  // "3/10", "no match"
  pub note: &'a str,
}

// the text plus its 1px box above and below
pub fn height(line_px: i32) -> i32 {
  line_px + 2
}

// returns the caret rect (window px) for the input method
pub fn draw(c: &mut Canvas, text: &mut Text, area: Rect, b: &FindBar) -> Rect {
  c.fill(area, theme::BLACK);
  c.outline(area, area, if b.focused { theme::WHITE } else { theme::DIM });
  let inner = Rect::new(area.x + 1, area.y + 1, (area.w - 2).max(0), (area.h - 2).max(0));
  let dim = |s: &str| [Span { range: 0..s.len(), ink: Ink(theme::DIM), bold: false }];
  let nw = if b.note.is_empty() {
    0
  } else {
    let n = format!(" {} ", b.note);
    let l = text.layout(&n, &dim(&n), &[], f32::MAX);
    let w = l.full_width().ceil() as i32;
    text.draw(&l, c, (inner.right() - w) as f32, inner.y as f32, inner, |_, _| {});
    w
  };
  let clip = Rect::new(inner.x, inner.y, (inner.w - nw).max(0), inner.h);
  let ox = inner.x as f32 + theme::PAD_X;
  let at = b.cursor.min(b.text.len());
  let shown = format!("{}{}{}", &b.text[..at], b.preedit, &b.text[at..]);
  if shown.is_empty() {
    let l = text.layout(b.hint, &dim(b.hint), &[], f32::MAX);
    text.draw(&l, c, ox, inner.y as f32, clip, |_, _| {});
  } else {
    let l = text.layout(&shown, &[Span { range: 0..shown.len(), ink: Ink(theme::WHITE), bold: false }], &[], f32::MAX);
    text.draw(&l, c, ox, inner.y as f32, clip, |_, _| {});
    underline(c, &l, at..at + b.preedit.len(), ox, inner.y as f32, clip);
  }
  let x = if at + b.preedit.len() == 0 {
    0.0
  } else {
    let head = &shown[..at + b.preedit.len()];
    text.layout(head, &[], &[], f32::MAX).full_width()
  };
  let caret = Rect::new((ox + x) as i32, inner.y, 2, inner.h);
  if b.focused {
    c.fill(caret.intersect(clip), theme::WHITE);
  }
  caret
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_bar_is_a_square_box_with_a_hint_a_count_and_a_caret() {
    let mut text = Text::new(1.0);
    let line = text.line_px as i32;
    let area = Rect::new(0, 10, 300, height(line));
    let mut c = Canvas::new(300, 60);
    let bar = |text: &'static str, focused, note| FindBar {
      text,
      cursor: text.len(),
      preedit: "",
      focused,
      hint: "filter",
      note,
    };
    draw(&mut c, &mut text, area, &bar("", false, ""));
    // the box: dim corners and edges, nothing outside it
    assert_eq!(c.px[(10 * 300) as usize], theme::DIM);
    assert_eq!(c.px[(10 * 300 + 299) as usize], theme::DIM);
    assert!((0..300).all(|x| c.px[(9 * 300 + x) as usize] == theme::BLACK));
    assert!((0..300).all(|x| c.px[(area.bottom() * 300 + x) as usize] == theme::BLACK));
    // the placeholder is dim and the caret is not there while the keys are elsewhere
    assert!(c.px.contains(&theme::DIM) && !c.px.contains(&theme::WHITE));
    // focused: a white box and a caret
    let caret = draw(&mut c, &mut text, area, &bar("/fo+/", true, "2 found"));
    assert_eq!(c.px[(10 * 300) as usize], theme::WHITE);
    assert!(caret.x > theme::PAD_X as i32 && c.px[(caret.y * 300 + caret.x) as usize] == theme::WHITE);
    // the count sits at the right edge
    assert!((250..299).any(|x| (11..10 + area.h - 1).any(|y| c.px[(y * 300 + x) as usize] != theme::BLACK)));
  }
}
