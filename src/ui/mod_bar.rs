// the mod toolbar (ext .hs-mod-toolbar): a small square strip of buttons at the top right of the line under
// the pointer. black with a thin edge and thin dividers, white text; the button under the pointer turns
// white with black text. the picked buttons that suit the line, in the catalog's order. the reply button
// (hoverReply) is the strip's first cell when it is on: beside the mod buttons, never over them
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;
use crate::model::modbar::CATALOG;

const PAD_X: i32 = 5;
// the reply cell's place in `idx` (not a catalog button) and its glyph
pub const REPLY: u8 = u8::MAX;
const REPLY_GLYPH: &str = "\u{21aa}";
// the most cells a strip holds: every catalog button and the reply
const CELLS: usize = CATALOG.len() + 1;
// the ext's right gap: clear of the edge
const GAP: i32 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bar {
  pub rect: Rect,
  pub n: usize,
  // which catalog button each cell is (REPLY for the reply cell), and where
  pub idx: [u8; CELLS],
  pub cell: [Rect; CELLS],
}

impl Bar {
  // the words of cell `i`
  pub fn label(&self, i: usize) -> &'static str {
    match self.idx[i] {
      REPLY => REPLY_GLYPH,
      k => CATALOG[k as usize].label,
    }
  }

  // the cell under (x, y)
  pub fn at(&self, x: i32, y: i32) -> Option<usize> {
    (0..self.n).find(|&i| self.cell[i].contains(x, y))
  }
}

fn width(text: &mut Text, label: &str) -> i32 {
  text.layout(label, &[], &[], f32::MAX).width().ceil() as i32
}

// where the buttons of `mask` (and the reply, when `reply`) sit for a line whose top is `top`, in the chat
// `area`; None when nothing is picked or the area is too small to hold them
pub fn place(text: &mut Text, area: Rect, top: i32, mask: u16, reply: bool) -> Option<Bar> {
  let h = text.line_px as i32 - 2;
  let mut bar = Bar { rect: Rect::default(), n: 0, idx: [0; CELLS], cell: [Rect::default(); CELLS] };
  let mut ws = [0i32; CELLS];
  if reply {
    ws[0] = width(text, REPLY_GLYPH) + 2 * PAD_X;
    bar.idx[0] = REPLY;
    bar.n = 1;
  }
  for (i, c) in CATALOG.iter().enumerate().filter(|(i, _)| mask >> i & 1 == 1) {
    ws[bar.n] = width(text, c.label) + 2 * PAD_X;
    bar.idx[bar.n] = i as u8;
    bar.n += 1;
  }
  if bar.n == 0 {
    return None;
  }
  // cells share one divider between them, plus the edge round the strip
  let w = ws[..bar.n].iter().sum::<i32>() + bar.n as i32 + 1;
  let y = top.max(area.y) + 1;
  if w + GAP > area.w || h < 4 || y + h > area.bottom() {
    return None;
  }
  let x = area.right() - GAP - w;
  bar.rect = Rect::new(x, y, w, h);
  let mut cx = x + 1;
  for (cell, w) in bar.cell.iter_mut().zip(ws).take(bar.n) {
    *cell = Rect::new(cx, y + 1, w, h - 2);
    cx += w + 1;
  }
  Some(bar)
}

pub fn draw(c: &mut Canvas, text: &mut Text, bar: &Bar, hover: Option<usize>) {
  c.fill(bar.rect, theme::BLACK);
  c.outline(bar.rect, bar.rect, theme::DIM);
  for i in 0..bar.n {
    let r = bar.cell[i];
    if i + 1 < bar.n {
      c.fill(Rect::new(r.right(), r.y, 1, r.h), theme::DIM);
    }
    let on = hover == Some(i);
    if on {
      c.fill(r, theme::WHITE);
    }
    let label = bar.label(i);
    let ink = if on { theme::BLACK } else { theme::WHITE };
    let l = text.layout(label, &[Span { range: 0..label.len(), ink: Ink(ink), bold: false }], &[], f32::MAX);
    let ty = r.y + (r.h - l.height().ceil() as i32) / 2;
    text.draw(&l, c, (r.x + PAD_X) as f32, ty as f32, r, |_, _| {});
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn cells_sit_in_order_at_the_top_right_inside_the_row() {
    let mut t = Text::new(1.0);
    let area = Rect::new(0, 100, 400, 200);
    let bar = place(&mut t, area, 140, 0b1_0000_0011, false).expect("three buttons fit");
    assert_eq!((bar.n, &bar.idx[..3]), (3, &[0u8, 1, 8][..]));
    assert_eq!(bar.rect.right(), area.right() - GAP);
    assert_eq!(bar.rect.y, 141);
    assert!(bar.cell[0].right() < bar.cell[1].x && bar.cell[1].right() < bar.cell[2].x, "dividers between");
    assert!(bar.rect.bottom() <= 140 + t.line_px as i32, "inside a one line row");
    assert_eq!(bar.at(bar.cell[1].x + 1, bar.cell[1].y + 1), Some(1));
    assert_eq!(bar.at(bar.rect.x - 3, bar.rect.y), None);
    // a row scrolled half off the top keeps the strip on screen
    assert_eq!(place(&mut t, area, 90, 1, false).map(|b| b.rect.y), Some(101));
    assert!(
      place(&mut t, area, 140, 0, false).is_none() && place(&mut t, Rect::new(0, 0, 30, 60), 0, 0b11, false).is_none()
    );
  }

  #[test]
  fn the_reply_leads_the_strip_and_never_overlaps_a_mod_button() {
    let mut t = Text::new(1.0);
    let area = Rect::new(0, 100, 400, 200);
    let alone = place(&mut t, area, 140, 0, true).expect("the reply fits alone");
    assert_eq!((alone.n, alone.idx[0], alone.label(0)), (1, REPLY, "\u{21aa}"));
    assert_eq!(alone.rect.right(), area.right() - GAP);
    let with = place(&mut t, area, 140, 0b11, true).expect("and with two buttons");
    assert_eq!((with.n, with.idx[0], with.idx[1], with.idx[2]), (3, REPLY, 0, 1));
    assert!(with.cell[0].right() < with.cell[1].x && with.cell[1].right() < with.cell[2].x);
    // the mod buttons keep the place they have without it: the reply only grows the strip to the left
    let bare = place(&mut t, area, 140, 0b11, false).unwrap();
    assert_eq!((bare.cell[0].x, bare.cell[1].x), (with.cell[1].x, with.cell[2].x));
    assert_eq!(bare.rect.right(), with.rect.right());
    assert_eq!(with.at(with.cell[0].x + 1, with.cell[0].y + 1), Some(0));
    // every button and the reply at once still fit the array
    assert!(place(&mut t, area, 140, 0x1ff, true).is_some_and(|b| b.n == 10));
  }

  #[test]
  fn white_on_black_and_the_hovered_button_inverts() {
    let mut t = Text::new(1.0);
    let mut c = Canvas::new(300, 60);
    let bar = place(&mut t, Rect::new(0, 0, 300, 60), 0, 0b11, false).unwrap();
    draw(&mut c, &mut t, &bar, None);
    let plain = c.inked(theme::WHITE);
    assert!(plain > 0 && c.px.contains(&theme::DIM));
    draw(&mut c, &mut t, &bar, Some(0));
    let r = bar.cell[0];
    assert!(c.inked(theme::WHITE) >= (r.w * r.h) as usize / 2, "that cell is a white block");
  }
}
