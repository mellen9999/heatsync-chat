// what floats over the chat just above the message box: the tab cycle's tooltip ("2/14 PogU · channel ·
// 7tv users") and the `:` `@` `/` lists, boxed in a 1px white border on black
use super::canvas::{Canvas, Rect};
use super::chat_view::blit_layer;
use super::text::{Ink, Span, Text};
use super::theme;
use crate::images::{Images, ImgId};

pub fn tip(c: &mut Canvas, text: &mut Text, input: Rect, s: &str) -> Rect {
  let line = text.line_px as i32;
  let l = text.layout(s, &[], &[], (input.w as f32 - 2.0 * theme::PAD_X - 2.0).max(1.0));
  let w = ((l.full_width() + 2.0 * theme::PAD_X).ceil() as i32 + 2).min(input.w);
  let r = Rect::new(input.x, input.y - line - 2, w, line + 2);
  c.fill(r, theme::WHITE);
  let inner = Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2);
  c.fill(inner, theme::BLACK);
  text.draw(&l, c, inner.x as f32 + theme::PAD_X, inner.y as f32, inner, |_, _| {});
  r
}

// one row of a suggestion list: an emote's picture, or a person in their color, or a command with
// its arguments and what it does (dim, the description pushed right)
pub struct Row<'a> {
  pub img: Option<ImgId>,
  pub name: &'a str,
  pub ink: u32,
  pub bold: bool,
  pub args: &'a str,
  pub desc: &'a str,
}

// the ext's list caps at 280px tall
const MAX_H: u16 = 280;
const PREVIEW: u16 = 22;

// the list right above the box, as wide as it, never above `top`; the picked row is reversed and kept in
// view (the row under the pointer, if any, is the reversed one instead). returns the painted rect and each shown row's (index, rect) for clicks
pub fn list(
  c: &mut Canvas,
  text: &mut Text,
  images: &mut Images,
  input: Rect,
  top: i32,
  rows: &[Row],
  (sel, hot): (usize, Option<usize>),
) -> (Rect, Vec<(usize, Rect)>) {
  let rh = (text.line_px as i32).max(images.px(PREVIEW) as i32 + 2);
  let room = (input.y - top - 2).min(images.px(MAX_H) as i32);
  let fit = ((room / rh).max(1) as usize).min(rows.len());
  // a window over the rows that holds the picked one
  let first = sel.saturating_sub(fit - 1).min(rows.len() - fit);
  let h = fit as i32 * rh + 2;
  let r = Rect::new(input.x, input.y - h, input.w, h);
  c.fill(r, theme::WHITE);
  c.fill(Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2), theme::BLACK);
  let mut hits = Vec::with_capacity(fit);
  for (k, row) in rows.iter().enumerate().skip(first).take(fit) {
    let y = r.y + 1 + (k - first) as i32 * rh;
    let cell = Rect::new(r.x + 1, y, r.w - 2, rh);
    let on = k == hot.unwrap_or(sel);
    if on {
      c.fill(cell, theme::WHITE);
    }
    let fg = |ink| if on { theme::BLACK } else { ink };
    let mut x = cell.x as f32 + theme::PAD_X;
    if let Some(id) = row.img {
      let ph = rh - 2;
      let pw = ph * 2;
      if let Some((f, w, h, _)) = images.frame(id) {
        let dw = ((w as i32 * ph) / h.max(1) as i32).clamp(1, pw);
        let dh = ((h as i32 * dw) / w.max(1) as i32).clamp(1, ph);
        let slot = Rect::new(x as i32, y + 1, dw, ph);
        blit_layer(c, &f.rgba, (w, h), (dw, dh), slot, cell);
        x += dw as f32;
      } else {
        x += ph as f32;
      }
      x += theme::PAD_X;
    }
    let mut s = String::from(row.name);
    let mut spans = vec![Span { range: 0..s.len(), ink: Ink(fg(row.ink)), bold: row.bold }];
    if !row.args.is_empty() {
      let a = s.len() + 1;
      s.push(' ');
      s.push_str(row.args);
      spans.push(Span { range: a..s.len(), ink: Ink(fg(theme::DIM)), bold: false });
    }
    let ty = y as f32 + (rh as f32 - text.line_px) / 2.0;
    let l = text.layout(&s, &spans, &[], f32::MAX);
    text.draw(&l, c, x, ty, cell, |_, _| {});
    if !row.desc.is_empty() {
      let span = [Span { range: 0..row.desc.len(), ink: Ink(fg(theme::DIM)), bold: false }];
      let d = text.layout(row.desc, &span, &[], f32::MAX);
      let dx = (cell.right() as f32 - theme::PAD_X - d.full_width()).max(x + l.full_width() + theme::PAD_X * 2.0);
      text.draw(&d, c, dx, ty, cell, |_, _| {});
    }
    hits.push((k, cell));
  }
  (r, hits)
}
