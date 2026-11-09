// the add/edit channel form, drawn over the chat. square, 1px white borders on black; buttons
// reverse on hover. emphasis is bold/reverse only
use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text};
use super::theme;
use crate::i18n::t as t_;
use crate::model::chan::Form;

const LABELS: [&str; 3] = ["twitch", "kick", "youtube"];
// the greyed hint in an empty row
fn hold(i: usize) -> &'static str {
  t_(if i == 2 { "ui.form.hold_youtube" } else { "ui.form.hold_name" })
}
const PAD: i32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
  Field(usize),
  Act(usize),
  Ok,
  Cancel,
}

pub struct Geo {
  pub panel: Rect,
  pub title: (i32, i32),
  pub fields: [Rect; 3],
  pub labels: [(i32, i32); 3],
  pub err: (i32, i32),
  pub acts: Vec<Rect>,
  pub ok: Rect,
  pub cancel: Rect,
  pub hint: (i32, i32),
}

pub fn geo(area: Rect, line: i32, em: i32, n_acts: usize) -> Geo {
  let row = line + 2 * PAD;
  let w = (em * 46).min(area.w - 2 * PAD).max(em * 20);
  let (abw, per) = (em * 14, ((w - 2 * PAD) / (em * 14 + PAD)).max(1) as usize);
  let act_rows = n_acts.div_ceil(per) as i32;
  // title, 3 rows, error line, action rows, buttons, hint
  let h = PAD + line + 2 * PAD + 3 * (row + PAD) + line + PAD + act_rows * (row + PAD) + row + PAD + line + PAD;
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
  let acts = (0..n_acts)
    .map(|i| Rect::new(x + PAD + (i % per) as i32 * (abw + PAD), cy + (i / per) as i32 * (row + PAD), abw, row))
    .collect();
  cy += act_rows * (row + PAD);
  let bw = em * 10;
  let ok = Rect::new(x + PAD, cy, bw, row);
  let cancel = Rect::new(x + PAD + bw + PAD, cy, bw, row);
  cy += row + PAD;
  Geo { panel, title, fields, labels, err, acts, ok, cancel, hint: (x + PAD, cy) }
}

pub fn hit(g: &Geo, x: i32, y: i32) -> Option<Hit> {
  if g.ok.contains(x, y) {
    return Some(Hit::Ok);
  }
  if g.cancel.contains(x, y) {
    return Some(Hit::Cancel);
  }
  if let Some(i) = g.acts.iter().position(|r| r.contains(x, y)) {
    return Some(Hit::Act(i));
  }
  g.fields.iter().position(|f| f.contains(x, y)).map(Hit::Field)
}

pub(super) fn put(c: &mut Canvas, t: &mut Text, s: &str, at: (i32, i32), ink: u32, bold: bool, clip: Rect) {
  if s.is_empty() {
    return;
  }
  let l = t.layout(s, &[Span { range: 0..s.len(), ink: Ink(ink), bold }], &[], f32::MAX);
  t.draw(&l, c, at.0 as f32, at.1 as f32, clip, |_, _| {});
}

pub(super) fn boxed(c: &mut Canvas, r: Rect, bg: u32) {
  c.fill(r, theme::WHITE);
  c.fill(Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2), bg);
}

pub(super) fn button(c: &mut Canvas, t: &mut Text, r: Rect, label: &str, on: bool, line: i32) {
  let (bg, fg) = if on { (theme::WHITE, theme::BLACK) } else { (theme::BLACK, theme::WHITE) };
  boxed(c, r, bg);
  // a translation can outgrow the box: elide it rather than spill over the edge
  let label = &t.elide(label, r.w - 4);
  let w = t.layout(label, &[], &[], f32::MAX).full_width() as i32;
  put(c, t, label, (r.x + (r.w - w) / 2, r.y + (r.h - line) / 2), fg, true, r);
}

// returns the caret rect (for the input method) and the panel
pub fn draw(c: &mut Canvas, t: &mut Text, area: Rect, f: &Form, title: &str, hover: Option<Hit>) -> (Rect, Rect) {
  let line = t.line_px as i32;
  let em = t.layout("0", &[], &[], f32::MAX).full_width().ceil() as i32;
  let g = geo(area, line, em, f.acts.len());
  c.fill(area, theme::BLACK);
  boxed(c, g.panel, theme::BLACK);
  let title = t.elide(title, g.panel.w - 2 * PAD);
  put(c, t, &title, g.title, theme::WHITE, true, g.panel);
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
      let h = t.elide(hold(i), inner.w - 2 * PAD);
      put(c, t, &h, (r.x + PAD, at_y), theme::DIM, false, inner);
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
  // one line each in a panel of fixed width: elide what runs past it
  let room = g.panel.w - 2 * PAD;
  match (f.err, f.welcome) {
    (Some(e), _) => {
      let e = t.elide(e, room);
      put(c, t, &e, g.err, theme::RED, false, g.panel)
    }
    (None, true) => {
      let w = t.elide(t_("ui.form.welcome"), room);
      put(c, t, &w, g.err, theme::DIM, false, g.panel)
    }
    _ => {}
  }
  for (i, (_, label)) in f.acts.iter().enumerate() {
    button(c, t, g.acts[i], label, f.focus == 3 + i || hover == Some(Hit::Act(i)), line);
  }
  let go = t_(if f.edit.is_some() { "ui.form.save" } else { "ui.form.add" });
  button(c, t, g.ok, go, hover == Some(Hit::Ok), line);
  button(c, t, g.cancel, t_("ui.form.cancel"), hover == Some(Hit::Cancel), line);
  let hint = t.elide(t_(if f.acts.is_empty() { "ui.form.hint_fields" } else { "ui.form.hint_buttons" }), room);
  put(c, t, &hint, g.hint, theme::DIM, false, g.panel);
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
    let g = geo(area, t.line_px as i32, em, 0);
    assert_eq!(hit(&g, g.ok.x + 2, g.ok.y + 2), Some(Hit::Ok));
    assert_eq!(hit(&g, g.cancel.x + 2, g.cancel.y + 2), Some(Hit::Cancel));
    assert_eq!(hit(&g, g.fields[1].x + 2, g.fields[1].y + 2), Some(Hit::Field(1)));
    assert_eq!(hit(&g, 0, 0), None);
  }
}
