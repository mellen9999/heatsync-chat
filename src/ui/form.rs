// the add/edit channel form, drawn over the chat. square, 1px white borders on black; buttons
// reverse on hover. emphasis is bold/reverse only
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;
use crate::model::chan::Form;

const LABELS: [&str; 3] = ["twitch", "kick", "youtube"];
const HOLD: [&str; 3] = ["username", "username", "username or url"];
const PAD: i32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
  Field(usize),
  Ok,
  Cancel,
}

pub struct Geo {
  pub panel: Rect,
  pub title: (i32, i32),
  pub fields: [Rect; 3],
  pub labels: [(i32, i32); 3],
  pub err: (i32, i32),
  pub ok: Rect,
  pub cancel: Rect,
  pub hint: (i32, i32),
}

pub fn geo(area: Rect, line: i32, em: i32) -> Geo {
  let row = line + 2 * PAD;
  let w = (em * 46).min(area.w - 2 * PAD).max(em * 20);
  // title, 3 rows, error line, buttons, hint
  let h = PAD + line + 2 * PAD + 3 * (row + PAD) + line + PAD + row + PAD + line + PAD;
  let x = area.x + ((area.w - w) / 2).max(0);
  let y = area.y + ((area.h - h) / 3).max(0);
  let panel = Rect::new(x, y, w, h);
  let lw = em * 9;
  let mut cy = y + PAD;
  let title = (x + PAD, cy);
  cy += line + 2 * PAD;
  let mut fields = [Rect::new(0, 0, 0, 0); 3];
  let mut labels = [(0, 0); 3];
  for i in 0..3 {
    labels[i] = (x + PAD, cy + PAD);
    fields[i] = Rect::new(x + PAD + lw, cy, w - 2 * PAD - lw, row);
    cy += row + PAD;
  }
  let err = (x + PAD, cy);
  cy += line + PAD;
  let bw = em * 10;
  let ok = Rect::new(x + PAD, cy, bw, row);
  let cancel = Rect::new(x + PAD + bw + PAD, cy, bw, row);
  cy += row + PAD;
  Geo { panel, title, fields, labels, err, ok, cancel, hint: (x + PAD, cy) }
}

pub fn hit(g: &Geo, x: i32, y: i32) -> Option<Hit> {
  if g.ok.contains(x, y) {
    return Some(Hit::Ok);
  }
  if g.cancel.contains(x, y) {
    return Some(Hit::Cancel);
  }
  g.fields.iter().position(|f| f.contains(x, y)).map(Hit::Field)
}

fn put(c: &mut Canvas, t: &mut Text, s: &str, at: (i32, i32), ink: u32, bold: bool, clip: Rect) {
  if s.is_empty() {
    return;
  }
  let l = t.layout(s, &[Span { range: 0..s.len(), ink: Ink(ink), bold }], &[], f32::MAX);
  t.draw(&l, c, at.0 as f32, at.1 as f32, clip, |_, _| {});
}

fn boxed(c: &mut Canvas, r: Rect, bg: u32) {
  c.fill(r, theme::WHITE);
  c.fill(Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2), bg);
}

fn button(c: &mut Canvas, t: &mut Text, r: Rect, label: &str, on: bool, line: i32) {
  let (bg, fg) = if on { (theme::WHITE, theme::BLACK) } else { (theme::BLACK, theme::WHITE) };
  boxed(c, r, bg);
  let w = t.layout(label, &[], &[], f32::MAX).full_width() as i32;
  put(c, t, label, (r.x + (r.w - w) / 2, r.y + (r.h - line) / 2), fg, true, r);
}

// returns the caret rect (for the input method) and the panel
pub fn draw(c: &mut Canvas, t: &mut Text, area: Rect, f: &Form, title: &str, hover: Option<Hit>) -> (Rect, Rect) {
  let line = t.line_px as i32;
  let em = t.layout("0", &[], &[], f32::MAX).full_width().ceil() as i32;
  let g = geo(area, line, em);
  c.fill(area, theme::BLACK);
  boxed(c, g.panel, theme::BLACK);
  put(c, t, title, g.title, theme::WHITE, true, g.panel);
  let mut caret = Rect::new(g.fields[0].x, g.fields[0].y, 1, line);
  for i in 0..3 {
    let on = f.focus == i;
    put(c, t, LABELS[i], g.labels[i], theme::WHITE, on, g.panel);
    let r = g.fields[i];
    boxed(c, r, theme::BLACK);
    let inner = Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2);
    let ln = &f.fields[i];
    let at_y = r.y + (r.h - line) / 2;
    if ln.text.is_empty() {
      put(c, t, HOLD[i], (r.x + PAD, at_y), theme::DIM, false, inner);
    }
    // scroll the text so the caret stays inside the box
    let cx = t.layout(&ln.text[..ln.cursor], &[], &[], f32::MAX).full_width().ceil() as i32;
    let shift = (cx - (inner.w - 2 * PAD)).max(0);
    put(c, t, &ln.text, (r.x + PAD - shift, at_y), theme::WHITE, false, inner);
    if on {
      caret = Rect::new(r.x + PAD - shift + cx, at_y, 2, line);
      c.fill(caret.intersect(inner), theme::WHITE);
    }
  }
  if let Some(e) = f.err {
    put(c, t, e, g.err, theme::RED, false, g.panel);
  }
  let go = if f.edit.is_some() { "save" } else { "add" };
  button(c, t, g.ok, go, hover == Some(Hit::Ok), line);
  button(c, t, g.cancel, "cancel", hover == Some(Hit::Cancel), line);
  put(c, t, "tab next field · enter confirms · esc cancels", g.hint, theme::DIM, false, g.panel);
  (caret, g.panel)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::chan::Spec;

  #[test]
  fn draws_inside_the_area_and_hits_map() {
    let mut c = Canvas::new(900, 600);
    let mut t = Text::new(1.0);
    let area = Rect::new(0, 20, 900, 500);
    let f = Form::new(None, &Spec::default());
    let (_, panel) = draw(&mut c, &mut t, area, &f, "add channel", Some(Hit::Ok));
    assert!(panel.x >= 0 && panel.right() <= 900 && panel.y >= area.y && panel.bottom() <= area.bottom());
    let em = t.layout("0", &[], &[], f32::MAX).full_width().ceil() as i32;
    let g = geo(area, t.line_px as i32, em);
    assert_eq!(hit(&g, g.ok.x + 2, g.ok.y + 2), Some(Hit::Ok));
    assert_eq!(hit(&g, g.cancel.x + 2, g.cancel.y + 2), Some(Hit::Cancel));
    assert_eq!(hit(&g, g.fields[1].x + 2, g.fields[1].y + 2), Some(Hit::Field(1)));
    assert_eq!(hit(&g, 0, 0), None);
  }
}
