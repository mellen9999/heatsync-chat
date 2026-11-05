// small keyboard-and-mouse menu drawn over the chat. square, 1px white border on black;
// the active row is reversed (white bg, black text). no motion
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;

const PAD: i32 = 6;

pub struct Menu<T> {
  pub items: Vec<(T, String)>,
  pub at: usize,
  // a dim line above the rows that says what they act on; never selectable
  head: Option<String>,
  // where it was asked for (window px), clamped into view when drawn
  anchor: (i32, i32),
  rect: Rect,
}

impl<T: Copy> Menu<T> {
  pub fn new<L: Into<String>>(items: Vec<(T, L)>, anchor: (i32, i32)) -> Self {
    let items = items.into_iter().map(|(t, l)| (t, l.into())).collect();
    Self { items, at: 0, head: None, anchor, rect: Rect::new(0, 0, 0, 0) }
  }

  pub fn anchor_to(&mut self, at: (i32, i32)) {
    self.anchor = at;
  }

  #[cfg(test)]
  pub fn head(&self) -> Option<&str> {
    self.head.as_deref()
  }

  pub fn titled(mut self, head: impl Into<String>) -> Self {
    self.head = Some(head.into());
    self
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
    let top = self.rect.y + 1 + if self.head.is_some() { line } else { 0 };
    let i = usize::try_from((y - top).div_euclid(line.max(1))).ok()?;
    (y >= top && i < self.items.len()).then_some(i)
  }

  pub fn draw(&mut self, c: &mut Canvas, text: &mut Text, bounds: Rect) -> Rect {
    let line = text.line_px as i32;
    let labels = self.items.iter().map(|i| i.1.as_str()).chain(self.head.as_deref());
    let w =
      labels.map(|l| text.layout(l, &[], &[], 1000.0).full_width().ceil() as i32).max().unwrap_or(0) + 2 * PAD + 2;
    let head_h = if self.head.is_some() { line } else { 0 };
    let h = line * self.items.len() as i32 + head_h + 2;
    let x = self.anchor.0.min(bounds.right() - w).max(bounds.x);
    let y = self.anchor.1.min(bounds.bottom() - h).max(bounds.y);
    let r = Rect::new(x, y, w, h);
    self.rect = r;
    c.fill(r, theme::WHITE);
    c.fill(Rect::new(x + 1, y + 1, w - 2, h - 2), theme::BLACK);
    if let Some(head) = &self.head {
      let l = text.layout(head, &[Span { range: 0..head.len(), ink: Ink(theme::DIM), bold: false }], &[], 1000.0);
      let row = Rect::new(x + 1, y + 1, w - 2, line);
      text.draw(&l, c, (row.x + PAD) as f32, row.y as f32, row, |_, _| {});
    }
    for (i, (_, label)) in self.items.iter().enumerate() {
      let row = Rect::new(x + 1, y + 1 + head_h + i as i32 * line, w - 2, line);
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

  #[test]
  fn a_head_line_is_not_a_row() {
    let mut m = Menu::new(vec![(1, "a"), (2, "b")], (0, 0)).titled("who");
    m.rect = Rect::new(10, 10, 50, 62);
    assert_eq!(m.row_at(20, 15, 20), None);
    assert_eq!(m.row_at(20, 31, 20), Some(0));
    assert_eq!(m.row_at(20, 51, 20), Some(1));
  }
}
