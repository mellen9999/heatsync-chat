// virtualized chat rows: heights for every row, layouts only for rows on screen
use std::collections::{HashSet, VecDeque};

use super::canvas::{Canvas, Rect};
use super::text::{Ink, Span, Text, TextLayout};
use super::theme;
use crate::model::message::{Kind, Message, flag};
use crate::util::time::hhmm;

struct Row {
  // parity drives zebra; backfill counts down from 0 so stripes stay alternating
  seq: i64,
  msg: Message,
  height: i32,
  layout: Option<Box<TextLayout>>,
}

pub struct ChatView {
  rows: VecDeque<Row>,
  cap: usize,
  seq_back: i64,
  seq_front: i64,
  width: i32,
  // px scrolled up from the bottom; 0 = pinned to newest
  scroll: i32,
  // rows appended since the last draw, for the scroll-blit fast path
  pending: usize,
  // something above the newest rows changed: next frame repaints everything
  stale: bool,
  pub zebra: bool,
  pub timestamps: bool,
}

pub enum Paint {
  Nothing,
  // rows appended while pinned: blit up by `shift`, draw only the freed strip
  Append { shift: i32 },
  Full,
}

const MARK_W: i32 = 2;

impl ChatView {
  pub fn new(cap: usize) -> Self {
    Self {
      rows: VecDeque::with_capacity(cap.min(1024)),
      cap,
      seq_back: 0,
      seq_front: 0,
      width: 0,
      scroll: 0,
      pending: 0,
      stale: true,
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
    self.seq_back += 1;
    self.rows.push_back(Row {
      seq: self.seq_back,
      msg,
      height: 0,
      layout: None,
    });
    self.pending += 1;
  }

  // history arrives after live lines may already be showing: slot it in front, skip dupes
  pub fn prepend(&mut self, history: Vec<Message>) {
    let have: HashSet<&str> = self
      .rows
      .iter()
      .map(|r| &*r.msg.id)
      .filter(|id| !id.is_empty())
      .collect();
    let room = self.cap.saturating_sub(self.rows.len());
    let fresh: Vec<Message> = history
      .into_iter()
      .filter(|m| m.id.is_empty() || !have.contains(&*m.id))
      .collect();
    let skip = fresh.len().saturating_sub(room);
    for msg in fresh.into_iter().skip(skip).rev() {
      self.seq_front -= 1;
      self.rows.push_front(Row {
        seq: self.seq_front,
        msg,
        height: 0,
        layout: None,
      });
    }
    self.stale = true;
  }

  fn touch(r: &mut Row) {
    r.layout = None;
    r.height = 0;
  }

  pub fn clear_msg(&mut self, id: &str) {
    if let Some(r) = self.rows.iter_mut().rev().find(|r| &*r.msg.id == id) {
      r.msg.flags |= flag::CLEARED;
      Self::touch(r);
      self.stale = true;
    }
  }

  pub fn clear_user(&mut self, login: &str) {
    for r in self
      .rows
      .iter_mut()
      .filter(|r| &*r.msg.login == login && r.msg.kind != Kind::System)
    {
      r.msg.flags |= flag::CLEARED;
      Self::touch(r);
      self.stale = true;
    }
  }

  pub fn resize(&mut self, width: i32) {
    if width != self.width {
      self.width = width;
      self.relayout();
    }
  }

  pub fn relayout(&mut self) {
    self.rows.iter_mut().for_each(Self::touch);
    self.stale = true;
  }

  pub fn invalidate(&mut self) {
    self.stale = true;
  }

  pub fn pinned(&self) -> bool {
    self.scroll == 0
  }

  pub fn scroll_by(&mut self, px: i32, text: &mut Text) {
    let max = self.content_height(text);
    self.scroll = (self.scroll + px).clamp(0, max);
    self.stale = true;
  }

  pub fn scroll_to_bottom(&mut self) {
    self.scroll = 0;
    self.stale = true;
  }

  pub fn scroll_to_top(&mut self, text: &mut Text) {
    self.scroll = self.content_height(text);
    self.stale = true;
  }

  fn content_height(&mut self, text: &mut Text) -> i32 {
    let (w, ts) = (self.width, self.timestamps);
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
    let mut s = String::with_capacity(m.nick.len() + m.body.len() + 16);
    let mut spans = Vec::with_capacity(4);
    let mut dim = |s: &mut String, t: &str| {
      let a = s.len();
      s.push_str(t);
      spans.push(Span {
        range: a..s.len(),
        ink: Ink(theme::DIM),
        bold: false,
      });
    };
    if let Some(sys) = &m.system {
      dim(&mut s, sys);
      s.push('\n');
    }
    if let Some(parent) = &m.reply_to {
      dim(&mut s, &format!("↳ @{parent}\n"));
    }
    if ts {
      dim(&mut s, &hhmm(m.ts));
      s.push(' ');
    }
    let cleared = m.has(flag::CLEARED);
    if m.kind == Kind::System {
      dim(&mut s, &m.body);
    } else {
      let n0 = s.len();
      s.push_str(&m.nick);
      spans.push(Span {
        range: n0..s.len(),
        ink: Ink(m.color),
        bold: true,
      });
      if m.kind == Kind::Chat {
        s.push(':');
      }
      s.push(' ');
      let b0 = s.len();
      s.push_str(&m.body);
      let ink = match (cleared, m.kind) {
        (true, _) => Some(theme::DIM),
        (_, Kind::Action) => Some(m.color),
        _ => None,
      };
      if let Some(c) = ink {
        spans.push(Span {
          range: b0..s.len(),
          ink: Ink(c),
          bold: false,
        });
      }
    }
    text.layout(&s, &spans, (width - MARK_W) as f32 - 2.0 * theme::PAD_X)
  }

  // what the next frame needs, given what changed since the last draw
  pub fn take_paint(&mut self, text: &mut Text, view_h: i32) -> Paint {
    let n = std::mem::take(&mut self.pending);
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
    }
    if std::mem::take(&mut self.stale) {
      return Paint::Full;
    }
    match () {
      _ if n == 0 || !self.pinned() => Paint::Nothing,
      _ if shift >= view_h => Paint::Full,
      _ => Paint::Append { shift },
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
        let row = Rect::new(area.x, top, area.w, r.height);
        if zebra && r.seq % 2 == 0 {
          c.fill(row.intersect(clip), theme::ZEBRA);
        }
        let mark = if r.msg.has(flag::FIRST) {
          Some(theme::MAGENTA)
        } else if r.msg.has(flag::HIGHLIGHTED) {
          Some(theme::CYAN)
        } else {
          None
        };
        if let Some(m) = mark {
          c.fill(Rect::new(area.x, top, MARK_W, r.height).intersect(clip), m);
        }
        if let Some(l) = &r.layout {
          text.draw(l, c, (area.x + MARK_W) as f32 + theme::PAD_X, top as f32, clip);
        }
      }
      drawn_lo = i;
      bottom = top;
    }
    // layouts that scrolled out of view are dropped; heights stay
    for r in self.rows.iter_mut().take(drawn_lo) {
      r.layout = None;
    }
  }
}
