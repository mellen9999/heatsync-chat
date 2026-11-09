// the first-run tour card, drawn over the chat: a square white-bordered panel, the title bold, a few dim-free lines,
// on the last card the switches as `on`/`off` rows, then back / next / skip as buttons that reverse on hover
use super::canvas::{Canvas, Rect};
use super::form::{boxed, button, put};
use super::text::Text;
use super::theme;
use crate::model::tour::{CARDS, SWITCHES, Tour};

const PAD: i32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
  Back,
  Next,
  Skip,
  // a switch row
  Row(usize),
}

pub struct Geo {
  pub panel: Rect,
  pub title: (i32, i32),
  pub lines: (i32, i32),
  pub rows: [Rect; 3],
  pub back: Rect,
  pub next: Rect,
  pub skip: Rect,
  pub hint: (i32, i32),
}

pub fn geo(area: Rect, line: i32, em: i32, tour: &Tour) -> Geo {
  let row = line + 2 * PAD;
  let w = (em * 48).min(area.w - 2 * PAD).max(em * 24);
  let n = CARDS[tour.at].lines.len() as i32;
  let sw = if tour.last() { SWITCHES.len() as i32 * (row + PAD) } else { 0 };
  // counter + title, the lines, switches, buttons, hint
  let h = PAD + line + 2 * PAD + n * line + PAD + sw + row + PAD + line + PAD;
  let (x, y) = (area.x + ((area.w - w) / 2).max(0), area.y + ((area.h - h) / 3).max(0));
  let mut cy = y + PAD;
  let title = (x + PAD, cy);
  cy += line + 2 * PAD;
  let lines = (x + PAD, cy);
  cy += n * line + PAD;
  let mut rows = [Rect::new(0, 0, 0, 0); 3];
  if tour.last() {
    for r in rows.iter_mut() {
      *r = Rect::new(x + PAD, cy, w - 2 * PAD, row);
      cy += row + PAD;
    }
  }
  let bw = em * 9;
  let back = Rect::new(x + PAD, cy, bw, row);
  let next = Rect::new(x + PAD + bw + PAD, cy, bw, row);
  let skip = Rect::new(x + w - PAD - bw, cy, bw, row);
  cy += row + PAD;
  Geo { panel: Rect::new(x, y, w, h), title, lines, rows, back, next, skip, hint: (x + PAD, cy) }
}

pub fn hit(g: &Geo, tour: &Tour, x: i32, y: i32) -> Option<Hit> {
  if g.next.contains(x, y) {
    return Some(Hit::Next);
  }
  if g.back.contains(x, y) && tour.at > 0 {
    return Some(Hit::Back);
  }
  if g.skip.contains(x, y) {
    return Some(Hit::Skip);
  }
  if tour.last() {
    return g.rows.iter().position(|r| r.contains(x, y)).map(Hit::Row);
  }
  None
}

// `rows`: each switch's label and whether it is on. `pointer` reverses the button under it
pub fn draw(c: &mut Canvas, t: &mut Text, area: Rect, tour: &Tour, rows: &[(&str, bool)], pointer: (i32, i32)) {
  let line = t.line_px as i32;
  let em = t.layout("0", &[], &[], f32::MAX).full_width().ceil() as i32;
  let g = geo(area, line, em, tour);
  let under = hit(&g, tour, pointer.0, pointer.1);
  let card = &CARDS[tour.at];
  c.fill(area, theme::BLACK);
  boxed(c, g.panel, theme::BLACK);
  put(c, t, &format!("{}/{}  {}", tour.at + 1, CARDS.len(), card.title), g.title, theme::WHITE, true, g.panel);
  for (i, l) in card.lines.iter().enumerate() {
    put(c, t, l, (g.lines.0, g.lines.1 + i as i32 * line), theme::WHITE, false, g.panel);
  }
  if tour.last() {
    for (i, (label, on)) in rows.iter().enumerate() {
      let hot = tour.row == i || under == Some(Hit::Row(i));
      let (bg, fg) = if hot { (theme::WHITE, theme::BLACK) } else { (theme::BLACK, theme::WHITE) };
      let r = g.rows[i];
      boxed(c, r, bg);
      let at_y = r.y + (r.h - line) / 2;
      put(c, t, label, (r.x + PAD, at_y), fg, false, r);
      let state = if *on { "on" } else { "off" };
      let sw = t.layout(state, &[], &[], f32::MAX).full_width().ceil() as i32;
      put(c, t, state, (r.right() - PAD - sw, at_y), fg, true, r);
    }
  }
  let word = if tour.last() { "done" } else { "next" };
  if tour.at > 0 {
    button(c, t, g.back, "back", under == Some(Hit::Back), line);
  }
  button(c, t, g.next, word, under == Some(Hit::Next), line);
  button(c, t, g.skip, "skip", under == Some(Hit::Skip), line);
  let hint = if tour.last() {
    "space switch · enter done · ← back · esc skip"
  } else {
    "enter next · ← back · esc skip"
  };
  put(c, t, hint, g.hint, theme::DIM, false, g.panel);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_card_fits_a_phone_wide_window_and_its_buttons_are_reachable() {
    let area = Rect::new(0, 0, 420, 700);
    let mut tour = Tour::default();
    for at in 0..CARDS.len() {
      tour.at = at;
      let g = geo(area, 16, 8, &tour);
      assert!(g.panel.x >= 0 && g.panel.right() <= area.w && g.panel.bottom() <= area.h, "card {at}");
      for r in [g.back, g.next, g.skip] {
        assert!(r.x >= g.panel.x && r.right() <= g.panel.right() && r.bottom() <= g.panel.bottom());
      }
      assert_eq!(hit(&g, &tour, g.next.x + 1, g.next.y + 1), Some(Hit::Next));
      assert_eq!(hit(&g, &tour, g.skip.x + 1, g.skip.y + 1), Some(Hit::Skip));
      let back = hit(&g, &tour, g.back.x + 1, g.back.y + 1);
      assert_eq!(back, if at == 0 { None } else { Some(Hit::Back) }, "no back on the first card");
    }
  }

  #[test]
  fn switch_rows_exist_only_on_the_last_card() {
    let area = Rect::new(0, 0, 420, 700);
    let mut tour = Tour { at: CARDS.len() - 1, row: 0 };
    let g = geo(area, 16, 8, &tour);
    assert_eq!(hit(&g, &tour, g.rows[1].x + 2, g.rows[1].y + 2), Some(Hit::Row(1)));
    tour.at = 0;
    let g = geo(area, 16, 8, &tour);
    assert_eq!(hit(&g, &tour, g.rows[1].x + 2, g.rows[1].y + 2), None);
  }
}
