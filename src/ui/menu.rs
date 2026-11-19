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
  // a menu taller than its room scrolls its rows: px hidden above, the most that can hide (set by the last
  // draw), and whether a key moved the pick since (only a key brings its row into view: a pointer resting
  // on a cut row must not scroll it, or the next cut row slides under it and the menu runs by itself)
  scroll: i32,
  max_scroll: i32,
  reveal: bool,
}

impl<T: Copy> Menu<T> {
  pub fn new<L: Into<String>>(items: Vec<(T, L)>, anchor: (i32, i32)) -> Self {
    let items = items.into_iter().map(|(t, l)| (t, l.into())).collect();
    Self { items, at: 0, head: None, anchor, rect: Rect::new(0, 0, 0, 0), scroll: 0, max_scroll: 0, reveal: false }
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
      self.go((self.at as i32 + by).rem_euclid(n) as usize);
    }
  }

  // a key picks this row: it is brought into view at the next draw
  pub fn go(&mut self, i: usize) {
    self.at = i;
    self.reveal = true;
  }

  // the wheel over the menu: px > 0 moves toward the first row. true when the menu took it (it can scroll)
  pub fn wheel(&mut self, x: i32, y: i32, px: i32) -> bool {
    if self.max_scroll == 0 || !self.rect.contains(x, y) {
      return false;
    }
    self.scroll = (self.scroll - px).clamp(0, self.max_scroll);
    true
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
    let i = usize::try_from((y - top + self.scroll).div_euclid(line.max(1))).ok()?;
    (y >= top && y < self.rect.bottom() - 1 && i < self.items.len()).then_some(i)
  }

  pub fn draw(&mut self, c: &mut Canvas, text: &mut Text, bounds: Rect) -> Rect {
    let line = text.line_px as i32;
    let labels = self.items.iter().map(|i| i.1.as_str()).chain(self.head.as_deref());
    let w =
      labels.map(|l| text.layout(l, &[], &[], 1000.0).full_width().ceil() as i32).max().unwrap_or(0) + 2 * PAD + 2;
    let head_h = if self.head.is_some() { line } else { 0 };
    let rows_h = line * self.items.len() as i32;
    let h = (rows_h + head_h + 2).min(bounds.h.max(0));
    let view_h = (h - head_h - 2).max(0);
    self.max_scroll = (rows_h - view_h).max(0);
    if self.reveal {
      self.reveal = false;
      let (top, bottom) = (self.at as i32 * line, (self.at as i32 + 1) * line);
      if top < self.scroll {
        self.scroll = top;
      } else if bottom > self.scroll + view_h {
        self.scroll = bottom - view_h;
      }
    }
    self.scroll = self.scroll.clamp(0, self.max_scroll);
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
    let view = Rect::new(x + 1, y + 1 + head_h, w - 2, view_h);
    for (i, (_, label)) in self.items.iter().enumerate() {
      let full = Rect::new(x + 1, view.y + i as i32 * line - self.scroll, w - 2, line);
      let row = full.intersect(view);
      if row.is_empty() {
        continue;
      }
      let on = i == self.at;
      if on {
        c.fill(row, theme::WHITE);
      }
      let ink = if on { theme::BLACK } else { theme::WHITE };
      let l = text.layout(label, &[Span { range: 0..label.len(), ink: Ink(ink), bold: false }], &[], 1000.0);
      text.draw(&l, c, (full.x + PAD) as f32, full.y as f32, row, |_, _| {});
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

  // a menu of 10 rows in a room that shows 4: capped, scrolls by wheel, keys keep their row in view, the
  // pointer never moves it
  #[test]
  fn a_menu_taller_than_its_room_scrolls() {
    use crate::ui::text::Text;
    let mut text = Text::new(1.0);
    let line = text.line_px as i32;
    let rows: Vec<_> = (0..10).map(|i| (i, format!("row {i}"))).collect();
    let mut m = Menu::new(rows, (10, 10));
    let mut c = Canvas::new(300, 400);
    let room = Rect::new(0, 0, 300, line * 4 + 2);
    let r = m.draw(&mut c, &mut text, room);
    assert_eq!(r.h, room.h, "capped to the room");
    assert!(m.wheel(r.x + 5, r.y + 5, -line * 3), "the wheel scrolls it");
    m.draw(&mut c, &mut text, room);
    assert_eq!(m.row_at(r.x + 5, r.y + 1, line), Some(3), "the first row on view is the fourth");
    assert!(!m.wheel(2000, 2000, -line), "off the menu is not its wheel");
    // a pointer's pick never scrolls it
    m.at = 9;
    m.draw(&mut c, &mut text, room);
    assert_eq!(m.row_at(r.x + 5, r.y + 1, line), Some(3));
    // a key's pick does
    m.step(0);
    m.draw(&mut c, &mut text, room);
    assert_eq!(m.row_at(r.x + 5, r.y + r.h - 3, line), Some(9), "the keyed row is the last on view");
    m.step(1);
    m.draw(&mut c, &mut text, room);
    assert_eq!(m.row_at(r.x + 5, r.y + 1, line), Some(0), "wrapping to row 0 scrolls back to the top");
    // a menu that fits never takes the wheel
    let mut small = Menu::new(vec![(1, "a")], (10, 10));
    let r = small.draw(&mut c, &mut text, Rect::new(0, 0, 300, 300));
    assert!(!small.wheel(r.x + 2, r.y + 2, line));
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
