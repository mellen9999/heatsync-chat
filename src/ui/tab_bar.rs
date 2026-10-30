// one row of tabs: active reversed (white bg, black text), unread bold, status dim at the end
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;

pub struct TabLabel<'a> {
  pub name: &'a str,
  pub active: bool,
  pub unread: bool,
}

pub fn draw(c: &mut Canvas, text: &mut Text, area: Rect, tabs: &[TabLabel], status: &str) {
  c.fill(area, theme::BLACK);
  let mut x = area.x;
  for t in tabs {
    let label = format!(" {} ", t.name);
    let ink = if t.active {
      theme::BLACK
    } else if t.unread {
      theme::WHITE
    } else {
      theme::DIM
    };
    let l = text.layout(
      &label,
      &[Span {
        range: 0..label.len(),
        ink: Ink(ink),
        bold: t.unread || t.active,
      }],
      &[],
      f32::MAX,
    );
    let w = l.width().ceil() as i32;
    if t.active {
      c.fill(Rect::new(x, area.y, w, area.h), theme::WHITE);
    }
    text.draw(&l, c, x as f32, area.y as f32, area, |_, _| {});
    x += w;
  }
  if !status.is_empty() {
    let s = format!("  {status}");
    let l = text.layout(
      &s,
      &[Span {
        range: 0..s.len(),
        ink: Ink(theme::DIM),
        bold: false,
      }],
      &[],
      f32::MAX,
    );
    text.draw(&l, c, x as f32, area.y as f32, area, |_, _| {});
  }
}
