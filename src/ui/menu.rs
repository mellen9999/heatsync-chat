// small keyboard-and-mouse menu drawn over the chat. square, 1px white border on black;
// the active row is reversed (white bg, black text). no motion
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;

const PAD: i32 = 6;

pub struct Menu<T> {
  pub items: Vec<(T, &'static str)>,
  pub at: usize,
  // where it was asked for (window px), clamped into view when drawn
  anchor: (i32, i32),
  rect: Rect,
}

impl<T: Copy> Menu<T> {
  pub fn new(items: Vec<(T, &'static str)>, anchor: (i32, i32)) -> Self {
    Self { items, at: 0, anchor, rect: Rect::new(0, 0, 0, 0) }
  }

  pub fn step(&mut self, by: i32) {
    let n = self.items.len() as i32;
    if n > 0 {
      self.at = (self.at as i32 + by).rem_euclid(n) as usize;
    }
  }

  pub fn chosen(&self) -> Option<T> {
    self.items.get(self.at).map(|i| i.0)
  }

  // the row under a pointer position, if any
  pub fn row_at(&self, x: i32, y: i32, line: i32) -> Option<usize> {
    if !self.rect.contains(x, y) {
      return None;
    }
    let i = ((y - self.rect.y - 1) / line.max(1)) as usize;
    (i < self.items.len()).then_some(i)
  }

  pub fn draw(&mut self, c: &mut Canvas, text: &mut Text, bounds: Rect) -> Rect {
    let line = text.line_px as i32;
    let w = self.items.iter().map(|i| text.layout(i.1, &[], &[], 1000.0).full_width().ceil() as i32).max().unwrap_or(0)
      + 2 * PAD
      + 2;
    let h = line * self.items.len() as i32 + 2;
    let x = self.anchor.0.min(bounds.right() - w).max(bounds.x);
    let y = self.anchor.1.min(bounds.bottom() - h).max(bounds.y);
    let r = Rect::new(x, y, w, h);
    self.rect = r;
    c.fill(r, theme::WHITE);
    c.fill(Rect::new(x + 1, y + 1, w - 2, h - 2), theme::BLACK);
    for (i, (_, label)) in self.items.iter().enumerate() {
      let row = Rect::new(x + 1, y + 1 + i as i32 * line, w - 2, line);
      let on = i == self.at;
      if on {
        c.fill(row, theme::WHITE);
      }
      let ink = if on { theme::BLACK } else { theme::WHITE };
      let l = text.layout(label, &[Span { range: 0..label.len(), ink: Ink(ink), bold: false }], &[], 1000.0);
      text.draw(&l, c, (row.x + PAD) as f32, row.y as f32, row, |_, _| {});
    }
    r
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn steps_wrap_and_rows_map() {
    let mut m = Menu::new(vec![(1, "a"), (2, "b"), (3, "c")], (0, 0));
    assert_eq!(m.chosen(), Some(1));
    m.step(-1);
    assert_eq!(m.chosen(), Some(3));
    m.step(1);
    assert_eq!(m.chosen(), Some(1));
    m.rect = Rect::new(10, 10, 50, 62);
    assert_eq!(m.row_at(20, 11, 20), Some(0));
    assert_eq!(m.row_at(20, 51, 20), Some(2));
    assert_eq!(m.row_at(5, 11, 20), None);
  }
}
