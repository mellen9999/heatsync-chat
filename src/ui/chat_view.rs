// virtualized chat rows: heights for every row, layouts only for rows on screen
use std::collections::VecDeque;

use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text, TextLayout};
use super::theme;
use crate::model::message::Message;

struct Row {
  seq: u64,
  msg: Message,
  height: i32,
  layout: Option<Box<TextLayout>>,
}

pub struct ChatView {
  rows: VecDeque<Row>,
  cap: usize,
  seq: u64,
  width: i32,
  // px scrolled up from the bottom; 0 = pinned to newest
  scroll: i32,
  // rows appended since the last draw, for the scroll-blit fast path
  pending: usize,
  pub zebra: bool,
  pub timestamps: bool,
}

pub enum Paint {
  Nothing,
  // rows appended while pinned: blit up by `shift`, draw only the freed strip
  Append { shift: i32 },
  Full,
}

fn hhmm(ts: i64) -> String {
  let m = ts.div_euclid(60).rem_euclid(1440);
  format!("{:02}:{:02} ", m / 60, m % 60)
}

impl ChatView {
  pub fn new(cap: usize) -> Self {
    Self {
      rows: VecDeque::with_capacity(cap.min(1024)),
      cap,
      seq: 0,
      width: 0,
      scroll: 0,
      pending: 0,
      zebra: true,
      timestamps: true,
    }
  }

  pub fn len(&self) -> usize {
    self.rows.len()
  }

  pub fn push(&mut self, msg: Message) {
    if self.rows.len() == self.cap {
      self.rows.pop_front();
    }
    self.seq += 1;
    self.rows.push_back(Row {
      seq: self.seq,
      msg,
      height: 0,
      layout: None,
    });
    self.pending += 1;
  }

  pub fn resize(&mut self, width: i32) {
    if width != self.width {
      self.width = width;
      for r in &mut self.rows {
        r.height = 0;
        r.layout = None;
      }
    }
  }

  pub fn pinned(&self) -> bool {
    self.scroll == 0
  }

  pub fn scroll_by(&mut self, px: i32, text: &mut Text) {
    let max = self.content_height(text);
    self.scroll = (self.scroll + px).clamp(0, max);
  }

  pub fn scroll_to_bottom(&mut self) {
    self.scroll = 0;
  }

  // full scrollback height; lays out every row once per width, then cached as heights
  fn content_height(&mut self, text: &mut Text) -> i32 {
    let w = self.width;
    let ts = self.timestamps;
    let mut total = 0;
    for r in &mut self.rows {
      if r.height == 0 {
        r.height = Self::layout_row(text, &r.msg, w, ts).height().ceil() as i32;
      }
      total += r.height;
    }
    total
  }

  fn layout_row(text: &mut Text, m: &Message, width: i32, ts: bool) -> TextLayout {
    let mut s = String::with_capacity(m.nick.len() + m.body.len() + 9);
    let mut spans = Vec::with_capacity(2);
    if ts {
      s.push_str(&hhmm(m.ts));
      spans.push(Span {
        range: 0..s.len(),
        ink: Ink(theme::DIM),
        bold: false,
      });
    }
    let n0 = s.len();
    s.push_str(&m.nick);
    spans.push(Span {
      range: n0..s.len(),
      ink: Ink(m.color),
      bold: true,
    });
    s.push_str(": ");
    s.push_str(&m.body);
    text.layout(&s, &spans, width as f32 - 2.0 * theme::PAD_X)
  }

  // what the next frame needs, given rows appended since the last draw
  pub fn take_paint(&mut self, text: &mut Text, view_h: i32) -> Paint {
    let n = std::mem::take(&mut self.pending);
    if n == 0 {
      return Paint::Nothing;
    }
    let mut shift = 0;
    let (w, ts) = (self.width, self.timestamps);
    for r in self.rows.iter_mut().rev().take(n) {
      let l = Self::layout_row(text, &r.msg, w, ts);
      r.height = l.height().ceil() as i32;
      shift += r.height;
      r.layout = Some(Box::new(l));
    }
    if !self.pinned() {
      // keep the reader's place: content grew under them
      self.scroll += shift;
      return Paint::Nothing;
    }
    if shift >= view_h {
      Paint::Full
    } else {
      Paint::Append { shift }
    }
  }

  // draw rows bottom-up into `area`, touching only pixels inside `clip`
  pub fn draw(&mut self, c: &mut Canvas, text: &mut Text, area: Rect, clip: Rect) {
    let clip = clip.intersect(area);
    c.fill(clip, theme::BLACK);
    let (w, ts, zebra) = (self.width, self.timestamps, self.zebra);
    let mut bottom = area.bottom() + self.scroll;
    let mut drawn_lo = self.rows.len();
    for (i, r) in self.rows.iter_mut().enumerate().rev() {
      if bottom <= area.y {
        break;
      }
      if r.height == 0 || (r.layout.is_none() && bottom - r.height < clip.bottom()) {
        let l = Self::layout_row(text, &r.msg, w, ts);
        r.height = l.height().ceil() as i32;
        r.layout = Some(Box::new(l));
      }
      let top = bottom - r.height;
      if top < clip.bottom() && bottom > clip.y {
        if zebra && r.seq % 2 == 0 {
          c.fill(Rect::new(area.x, top, area.w, r.height).intersect(clip), theme::ZEBRA);
        }
        if let Some(l) = &r.layout {
          text.draw(l, c, (area.x as f32) + theme::PAD_X, top as f32, clip);
        }
      }
      drawn_lo = i;
      bottom = top;
    }
    // drop layouts that scrolled out of view; heights stay
    for r in self.rows.iter_mut().take(drawn_lo) {
      if r.layout.is_none() {
        continue;
      }
      r.layout = None;
    }
  }
}
