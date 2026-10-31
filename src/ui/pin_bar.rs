// the pinned line, one row under the tabs: `pin` label, name, text, and a × that hides it.
// click the row to open the whole text (it wraps), click again to fold it back
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text, TextLayout};
use super::theme;
use crate::model::pin::Pin;

const LABEL: &str = "pin ";
const HIDE: &str = " × ";

fn body(text: &mut Text, p: &Pin, wrap: f32) -> TextLayout {
  let s = format!("{LABEL}{}: {}", p.nick, p.text);
  let (a, b) = (LABEL.len(), LABEL.len() + p.nick.len());
  let spans =
    [Span { range: 0..a, ink: Ink(theme::DIM), bold: false }, Span { range: a..b, ink: Ink(p.color), bold: true }];
  text.layout(&s, &spans, &[], wrap)
}

fn hide_w(text: &mut Text) -> i32 {
  text.layout(HIDE, &[], &[], f32::MAX).width().ceil() as i32
}

// the × cell at the right end of the row
pub fn hide_rect(text: &mut Text, area: Rect) -> Rect {
  let w = hide_w(text);
  Rect::new(area.right() - w, area.y, w, text.line_px as i32)
}

pub fn height(text: &mut Text, p: &Pin, width: i32, open: bool) -> i32 {
  if !open {
    return text.line_px as i32;
  }
  let wrap = (width - hide_w(text)) as f32;
  body(text, p, wrap).height().ceil() as i32
}

pub fn draw(c: &mut Canvas, text: &mut Text, area: Rect, p: &Pin, open: bool) {
  c.fill(area, theme::BLACK);
  let x = hide_rect(text, area);
  // folded: one line, cut at the × instead of wrapping
  let wrap = if open { (area.w - x.w) as f32 } else { f32::MAX };
  let l = body(text, p, wrap);
  let clip = Rect::new(area.x, area.y, area.w - x.w, area.h);
  text.draw(&l, c, area.x as f32 + theme::PAD_X, area.y as f32, clip, |_, _| {});
  let hide = text.layout(HIDE, &[Span { range: 0..HIDE.len(), ink: Ink(theme::WHITE), bold: true }], &[], f32::MAX);
  text.draw(&hide, c, x.x as f32, x.y as f32, x, |_, _| {});
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn folded_is_one_line_open_wraps() {
    let mut t = Text::new(1.0);
    let p =
      Pin { id: "1".into(), nick: "bob".into(), color: theme::WHITE, text: "word ".repeat(80).trim().into(), at: 0 };
    let line = t.line_px as i32;
    assert_eq!(height(&mut t, &p, 300, false), line);
    assert!(height(&mut t, &p, 300, true) > line);
    // the x sits inside the row at its right edge
    let area = Rect::new(0, 20, 300, line);
    let h = hide_rect(&mut t, area);
    assert_eq!(h.right(), 300);
    assert!(h.w > 0);
  }
}
