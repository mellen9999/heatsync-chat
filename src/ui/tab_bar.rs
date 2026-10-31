// one row of tabs: active reversed (white bg, black text), unread bold, status dim at the end.
// a `+` after the last tab adds one; hover reverses it like the active tab
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;

pub struct TabLabel<'a> {
  pub name: &'a str,
  pub active: bool,
  pub unread: bool,
}

// returns each tab's rect, then the `+` rect last (for clicks)
pub fn draw(
  c: &mut Canvas,
  text: &mut Text,
  area: Rect,
  tabs: &[TabLabel],
  status: &str,
  plus_hover: bool,
) -> Vec<Rect> {
  c.fill(area, theme::BLACK);
  let mut x = area.x;
  let mut hits = Vec::with_capacity(tabs.len() + 1);
  let cell = |c: &mut Canvas, text: &mut Text, x: &mut i32, label: &str, ink: u32, bold: bool, on: bool| {
    let l = text.layout(
      label,
      &[Span { range: 0..label.len(), ink: Ink(if on { theme::BLACK } else { ink }), bold }],
      &[],
      f32::MAX,
    );
    let w = l.width().ceil() as i32;
    if on {
      c.fill(Rect::new(*x, area.y, w, area.h), theme::WHITE);
    }
    text.draw(&l, c, *x as f32, area.y as f32, area, |_, _| {});
    let r = Rect::new(*x, area.y, w, area.h);
    *x += w;
    r
  };
  for t in tabs {
    let ink = if t.unread { theme::WHITE } else { theme::DIM };
    hits.push(cell(c, text, &mut x, &format!(" {} ", t.name), ink, t.unread || t.active, t.active));
  }
  hits.push(cell(c, text, &mut x, " + ", theme::WHITE, true, plus_hover));
  if !status.is_empty() {
    let s = format!("  {status}");
    let l = text.layout(&s, &[Span { range: 0..s.len(), ink: Ink(theme::DIM), bold: false }], &[], f32::MAX);
    text.draw(&l, c, x as f32, area.y as f32, area, |_, _| {});
  }
  hits
}
