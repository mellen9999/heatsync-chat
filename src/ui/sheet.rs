// a full-width scrolling list over the chat, one row per line, one row picked: the settings screen
// and the key map. the picked row is reversed (white bg, black text); a switch shows as [on] / [off]
// with [on] reversed. square, no motion. the caller owns the lines and the state; this only draws
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;

const PAD: i32 = 6;

#[derive(Clone)]
pub enum L {
  Head(String),
  // a: the label (or the keys), b: the value (or what they do). pill: a switch and its state
  Row { a: String, b: String, pill: Option<bool> },
}

#[derive(Default)]
pub struct State {
  // the picked line (always a Row once settled), the first line shown, and where rows were drawn
  pub at: usize,
  pub scroll: usize,
  hits: Vec<(Rect, usize)>,
}

fn row(lines: &[L], i: usize) -> bool {
  matches!(lines.get(i), Some(L::Row { .. }))
}

impl State {
  // after the lines changed: land on a row, the nearest at or after `at`, else before it
  pub fn settle(&mut self, lines: &[L]) {
    self.at = self.at.min(lines.len().saturating_sub(1));
    if row(lines, self.at) {
      return;
    }
    let after = (self.at..lines.len()).find(|&i| row(lines, i));
    let before = (0..self.at).rev().find(|&i| row(lines, i));
    self.at = after.or(before).unwrap_or(0);
  }

  // move `by` rows, wrapping
  pub fn step(&mut self, lines: &[L], by: i32) {
    let rows: Vec<usize> = (0..lines.len()).filter(|&i| row(lines, i)).collect();
    if rows.is_empty() {
      return;
    }
    let cur = rows.iter().position(|&i| i == self.at).unwrap_or(0) as i32;
    self.at = rows[(cur + by).rem_euclid(rows.len() as i32) as usize];
  }

  pub fn first(&mut self, lines: &[L]) {
    self.at = 0;
    self.settle(lines);
  }

  pub fn last(&mut self, lines: &[L]) {
    self.at = lines.len().saturating_sub(1);
    self.settle(lines);
  }

  // a row the pointer is over
  pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
    self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|h| h.1)
  }

  // move the pick `by` rows without wrapping (pages, wheel)
  pub fn shift(&mut self, lines: &[L], by: i32) {
    let rows: Vec<usize> = (0..lines.len()).filter(|&i| row(lines, i)).collect();
    if rows.is_empty() {
      return;
    }
    let cur = rows.iter().position(|&i| i == self.at).unwrap_or(0) as i32;
    self.at = rows[(cur + by).clamp(0, rows.len() as i32 - 1) as usize];
  }
}

pub struct View<'a> {
  pub title: &'a str,
  // the search box: typed text, whether it has the keys, what it says when empty, and the mark before
  // typed text (`/ `). None = no box
  pub search: Option<(&'a str, bool, &'a str, &'a str)>,
  pub lines: &'a [L],
  // dim lines under the list: what the picked row does, and the keys
  pub tip: &'a str,
  pub keys: &'a str,
  // keys screens: where `b` starts, in columns. None = right-aligned values
  pub col: Option<usize>,
}

struct Ctx<'a> {
  c: &'a mut Canvas,
  text: &'a mut Text,
}

impl Ctx<'_> {
  fn put(&mut self, s: &str, x: i32, row: Rect, ink: u32, bold: bool) -> i32 {
    let l = self.text.layout(s, &[Span { range: 0..s.len(), ink: Ink(ink), bold }], &[], f32::MAX);
    self.text.draw(&l, self.c, x as f32, row.y as f32, row, |_, _| {});
    l.full_width().ceil() as i32
  }

  fn width(&mut self, s: &str, bold: bool) -> i32 {
    let l = self.text.layout(s, &[Span { range: 0..s.len(), ink: Ink(theme::WHITE), bold }], &[], f32::MAX);
    l.full_width().ceil() as i32
  }
}

pub fn draw(c: &mut Canvas, text: &mut Text, area: Rect, v: &View, st: &mut State) {
  c.fill(area, theme::BLACK);
  let line = text.line_px as i32;
  let em = text.layout("0", &[], &[], f32::MAX).full_width().ceil() as i32;
  let foot = 3 * line;
  let mut cx = Ctx { c, text };
  // top row: title, then the search box
  let top = Rect::new(area.x, area.y, area.w, line);
  let mut x = area.x + PAD;
  x += cx.put(v.title, x, top, theme::WHITE, true) + 2 * em;
  if let Some((q, typing, hint, mark)) = v.search {
    if q.is_empty() && !typing {
      cx.put(hint, x, top, theme::DIM, false);
    } else {
      x += cx.put(mark, x, top, theme::DIM, false);
      x += cx.put(q, x, top, theme::WHITE, false);
      if typing {
        cx.put("█", x, top, theme::WHITE, false);
      }
    }
  }
  // the list
  let body = Rect::new(area.x, area.y + line, area.w, (area.h - line - foot).max(line));
  let fit = ((body.h / line).max(1)) as usize;
  let n = v.lines.len();
  // keep the pick on screen, with its heading when it is the first row of a section
  let lead = if st.at > 0 && matches!(v.lines[st.at - 1], L::Head(_)) { st.at - 1 } else { st.at };
  if lead < st.scroll {
    st.scroll = lead;
  } else if st.at >= st.scroll + fit {
    st.scroll = st.at + 1 - fit;
  }
  st.scroll = st.scroll.min(n.saturating_sub(fit));
  st.hits.clear();
  for (k, l) in v.lines.iter().enumerate().skip(st.scroll).take(fit) {
    let r = Rect::new(body.x, body.y + (k - st.scroll) as i32 * line, body.w, line);
    match l {
      L::Head(h) => {
        cx.put(h, r.x + PAD, r, theme::WHITE, true);
      }
      L::Row { a, b, pill } => {
        let sel = k == st.at;
        if sel {
          cx.c.fill(r, theme::WHITE);
        }
        st.hits.push((r, k));
        let (fg, bg) = if sel { (theme::BLACK, theme::WHITE) } else { (theme::WHITE, theme::BLACK) };
        cx.put(a, r.x + PAD + em, r, fg, false);
        match (pill, v.col) {
          (Some(on), _) => {
            let s = if *on { "[on]" } else { "[off]" };
            let w = cx.width(s, false);
            let pr = Rect::new(r.right() - PAD - w, r.y, w, line);
            // on = reverse video; the picked row is already reversed, so there it flips back
            let (pfg, pbg) = match (*on, sel) {
              (true, false) => (theme::BLACK, theme::WHITE),
              (true, true) => (theme::WHITE, theme::BLACK),
              (false, false) => (theme::DIM, bg),
              (false, true) => (theme::BLACK, bg),
            };
            if pbg != bg {
              cx.c.fill(pr, pbg);
            }
            cx.put(s, pr.x, pr, pfg, false);
          }
          (None, Some(col)) => {
            cx.put(b, r.x + PAD + em + col as i32 * em, r, if sel { theme::BLACK } else { theme::DIM }, false);
          }
          (None, None) => {
            let w = cx.width(b, false);
            cx.put(b, r.right() - PAD - w, r, if sel { theme::BLACK } else { theme::DIM }, false);
          }
        }
      }
    }
  }
  // footer: the picked row's tip (two lines, wrapped), then the keys
  let tip_r = Rect::new(area.x, area.bottom() - foot, area.w, 2 * line);
  let l = cx.text.layout(
    v.tip,
    &[Span { range: 0..v.tip.len(), ink: Ink(theme::DIM), bold: false }],
    &[],
    (area.w - 2 * PAD) as f32,
  );
  cx.text.draw(&l, cx.c, (tip_r.x + PAD) as f32, tip_r.y as f32, tip_r, |_, _| {});
  let keys_r = Rect::new(area.x, area.bottom() - line, area.w, line);
  cx.put(v.keys, keys_r.x + PAD, keys_r, theme::DIM, false);
}

#[cfg(test)]
mod tests {
  use super::*;

  fn lines() -> Vec<L> {
    let r = |a: &str, p| L::Row { a: a.into(), b: String::new(), pill: p };
    vec![L::Head("one".into()), r("a", Some(true)), r("b", Some(false)), L::Head("two".into()), r("c", None)]
  }

  #[test]
  fn the_pick_only_lands_on_rows_and_wraps() {
    let l = lines();
    let mut s = State::default();
    s.settle(&l);
    assert_eq!(s.at, 1);
    s.step(&l, 1);
    assert_eq!(s.at, 2);
    s.step(&l, 1);
    assert_eq!(s.at, 4);
    s.step(&l, 1);
    assert_eq!(s.at, 1);
    s.step(&l, -1);
    assert_eq!(s.at, 4);
    s.at = 3;
    s.settle(&l);
    assert_eq!(s.at, 4);
    s.first(&l);
    assert_eq!(s.at, 1);
    s.last(&l);
    assert_eq!(s.at, 4);
    s.shift(&l, -9);
    assert_eq!(s.at, 1);
    s.shift(&l, 2);
    assert_eq!(s.at, 4);
    s.shift(&l, 5);
    assert_eq!(s.at, 4);
    // nothing to pick is fine
    let mut e = State::default();
    e.step(&[], 1);
    e.settle(&[]);
    assert_eq!(e.at, 0);
  }

  #[test]
  fn it_draws_rows_square_and_answers_clicks() {
    let mut text = Text::new(1.0);
    let mut c = Canvas::new(400, 300);
    let area = Rect::new(0, 20, 400, 280);
    let l = lines();
    let mut s = State::default();
    s.settle(&l);
    let v = View {
      title: "settings",
      search: Some(("", false, "/ search", "/ ")),
      lines: &l,
      tip: "a tip",
      keys: "esc closes",
      col: None,
    };
    draw(&mut c, &mut text, area, &v, &mut s);
    let line = text.line_px as i32;
    // the picked row is white-on-black reversed: its left edge is white
    assert_eq!(c.px[((area.y + line + line / 2 + line) * 400 + 1) as usize], theme::WHITE);
    assert_eq!(s.hit(5, area.y + 2 * line + line / 2), Some(1));
    assert_eq!(s.hit(5, area.y + line + 2), None);
    assert_eq!(s.hit(5, area.y + 3 * line + 2), Some(2));
  }
}
