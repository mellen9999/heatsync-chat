// virtualized chat rows: heights for every row, layouts only for rows on screen
use std::collections::{HashSet, VecDeque};

use parley::{InlineBox, InlineBoxKind, VerticalAlign};

use super::canvas::{Canvas, Rect};
use super::res::{BADGE_PX, EMOTE_PX, Res};
use super::text::{Ink, Span, TextLayout};
use super::theme;
use crate::images::{Images, ImgId};
use crate::model::message::{Kind, Message, Platform, flag};
use crate::util::time::hhmm;

// one emote slot in a line; zero-width overlays stack on the base image
struct EmoteBox {
  img: ImgId,
  zw: Vec<ImgId>,
  // tile color behind the image (ffz badges)
  bg: Option<u32>,
}

struct Laid {
  layout: TextLayout,
  boxes: Vec<EmoteBox>,
}

struct Row {
  // parity drives zebra; backfill counts down from 0 so stripes stay alternating
  seq: i64,
  msg: Message,
  height: i32,
  laid: Option<Box<Laid>>,
}

// an animated emote currently on screen: redrawn alone when its frame flips
struct Anim {
  rect: Rect,
  bg: u32,
  imgs: Vec<ImgId>,
}

pub struct ChatView {
  pub channel: Box<str>,
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
  anims: Vec<Anim>,
  // ms until the soonest on-screen animation flips
  pub next_anim: Option<u32>,
  pub zebra: bool,
  pub timestamps: bool,
  // [T]/[K]/[Y] before each line once a tab merges platforms
  pub tags: bool,
}

pub enum Paint {
  Nothing,
  // rows appended while pinned: blit up by `shift`, draw only the freed strip
  Append { shift: i32 },
  Full,
}

const MARK_W: i32 = 2;

fn layout_row(res: &mut Res, chan: &str, m: &Message, width: i32, ts: bool, tags: bool) -> Laid {
  let mut s = String::with_capacity(m.nick.len() + m.body.len() + 16);
  let mut spans = Vec::with_capacity(4);
  let dim = |s: &mut String, spans: &mut Vec<Span>, t: &str| {
    let a = s.len();
    s.push_str(t);
    spans.push(Span { range: a..s.len(), ink: Ink(theme::DIM), bold: false });
  };
  if let Some(sys) = &m.system {
    dim(&mut s, &mut spans, sys);
    s.push('\n');
  }
  if let Some(parent) = &m.reply_to {
    dim(&mut s, &mut spans, &format!("↳ @{parent}\n"));
  }
  if ts {
    dim(&mut s, &mut spans, &hhmm(m.ts));
    s.push(' ');
  }
  if tags {
    let (t, c) = match m.platform {
      Platform::Twitch => ("[T] ", theme::TWITCH),
      Platform::Kick => ("[K] ", theme::GREEN),
      Platform::Youtube => ("[Y] ", theme::RED),
    };
    let a = s.len();
    s.push_str(t);
    spans.push(Span { range: a..a + 3, ink: Ink(c), bold: true });
  }
  let mut ib = Vec::new();
  let mut boxes = Vec::new();
  if m.kind == Kind::System {
    dim(&mut s, &mut spans, &m.body);
  } else {
    badges(res, chan, m, &mut s, &mut ib, &mut boxes);
    let n0 = s.len();
    s.push_str(&m.nick);
    spans.push(Span { range: n0..s.len(), ink: Ink(m.color), bold: true });
    if m.kind == Kind::Chat {
      s.push(':');
    }
    s.push(' ');
    let b0 = s.len();
    body(res, chan, m, &mut s, &mut ib, &mut boxes);
    let ink = match (m.has(flag::CLEARED), m.kind) {
      (true, _) => Some(theme::DIM),
      (_, Kind::Action) => Some(m.color),
      _ => None,
    };
    if let Some(c) = ink {
      spans.push(Span { range: b0..s.len(), ink: Ink(c), bold: false });
    }
  }
  let layout = res.text.layout(&s, &spans, &ib, (width - MARK_W) as f32 - 2.0 * theme::PAD_X);
  Laid { layout, boxes }
}

fn inline(id: usize, index: usize, w: u32, h: u32) -> InlineBox {
  InlineBox {
    id: id as u64,
    kind: InlineBoxKind::InFlow,
    index,
    width: w as f32,
    height: h as f32,
    baseline: None,
    vertical_align: VerticalAlign::MIDDLE,
  }
}

fn badges(res: &mut Res, chan: &str, m: &Message, s: &mut String, ib: &mut Vec<InlineBox>, boxes: &mut Vec<EmoteBox>) {
  let found: Vec<(Box<str>, Option<u32>)> =
    res.badges.for_message(chan, m).into_iter().map(|b| (b.url.clone(), b.bg)).collect();
  let h = res.images.px(BADGE_PX);
  for (url, bg) in found {
    let img = res.images.id(&url, BADGE_PX);
    let w = res.images.width(img, (18, 18));
    ib.push(inline(boxes.len(), s.len(), w, h));
    boxes.push(EmoteBox { img, zw: Vec::new(), bg });
    // a thin space keeps badges from touching
    s.push('\u{2009}');
  }
}

// words become text or emote boxes. twitch's own emotes come by position, the rest by name
fn body(res: &mut Res, chan: &str, m: &Message, s: &mut String, ib: &mut Vec<InlineBox>, boxes: &mut Vec<EmoteBox>) {
  let h = res.images.px(EMOTE_PX);
  let mut prev_emote = false;
  let mut first = true;
  let mut at = 0usize;
  let sender = (m.platform == Platform::Twitch).then(|| m.user_id.parse::<u64>().ok()).flatten();
  for word in m.body.split(' ') {
    let (a, b) = (at, at + word.len());
    at = b + 1;
    let native = m.emotes.iter().find(|(r, _)| r.start as usize == a && r.end as usize == b);
    let found = match native {
      Some((_, url)) => Some((url.clone(), (0, 0), false)),
      None if word.is_empty() => None,
      None => resolve(res, chan, m, sender, word),
    };
    match found {
      Some((url, _, true)) if prev_emote => {
        let img = res.images.id(&url, EMOTE_PX);
        if let Some(last) = boxes.last_mut() {
          last.zw.push(img);
        }
        continue;
      }
      Some((url, dims, _)) => {
        if !first {
          s.push(' ');
        }
        let img = res.images.id(&url, EMOTE_PX);
        let w = res.images.width(img, dims);
        ib.push(inline(boxes.len(), s.len(), w, h));
        boxes.push(EmoteBox { img, zw: Vec::new(), bg: None });
        prev_emote = true;
      }
      None => {
        if !first {
          s.push(' ');
        }
        s.push_str(word);
        prev_emote = word.is_empty() && prev_emote;
      }
    }
    first = false;
  }
}

// ext order: the line's own server-verified refs, the channel catalog, the sender's heatsync
// inventory, then globals. the sender's set is the only heatsync source (privacy invariant)
fn resolve(res: &mut Res, chan: &str, m: &Message, sender: Option<u64>, word: &str) -> Option<Found> {
  let e = |e: &crate::emotes::Emote| (e.url.clone(), (e.w, e.h), e.zero_width);
  if let Some((_, u, zw)) = m.hs.iter().find(|(w, _, _)| **w == *word) {
    return Some((u.clone(), (0, 0), *zw));
  }
  if let Some(x) = res.emotes.channel(chan, word) {
    return Some(e(x));
  }
  if let Some(x) = sender.and_then(|id| res.senders.lookup(id, word, m.ts)) {
    return Some((x.url.clone(), (x.w, x.h), x.zero_width));
  }
  res.emotes.global(word).map(e)
}

type Found = (Box<str>, (u16, u16), bool);

fn blit_emote(c: &mut Canvas, images: &mut Images, imgs: &[ImgId], rect: Rect, clip: Rect, next: &mut Option<u32>) {
  for &id in imgs {
    let Some((f, w, h, flip)) = images.frame(id) else {
      continue;
    };
    // zero-width overlays center on the base
    let x = rect.x + (rect.w - w as i32) / 2;
    let y = rect.y + (rect.h - h as i32) / 2;
    c.rgba(x, y, w as i32, h as i32, &f.rgba, clip);
    if let Some(ms) = flip {
      *next = Some(next.map_or(ms, |n| n.min(ms)));
    }
  }
}

impl ChatView {
  pub fn new(cap: usize, channel: Box<str>) -> Self {
    Self {
      channel,
      rows: VecDeque::with_capacity(cap.min(1024)),
      cap,
      seq_back: 0,
      seq_front: 0,
      width: 0,
      scroll: 0,
      pending: 0,
      stale: true,
      anims: Vec::new(),
      next_anim: None,
      zebra: true,
      timestamps: true,
      tags: false,
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
    self.rows.push_back(Row { seq: self.seq_back, msg, height: 0, laid: None });
    self.pending += 1;
  }

  // history arrives after live lines may already be showing: slot it in front, skip dupes
  pub fn prepend(&mut self, history: Vec<Message>) {
    let have: HashSet<&str> = self.rows.iter().map(|r| &*r.msg.id).filter(|id| !id.is_empty()).collect();
    let room = self.cap.saturating_sub(self.rows.len());
    let fresh: Vec<Message> = history.into_iter().filter(|m| m.id.is_empty() || !have.contains(&*m.id)).collect();
    let skip = fresh.len().saturating_sub(room);
    for msg in fresh.into_iter().skip(skip).rev() {
      self.seq_front -= 1;
      self.rows.push_front(Row { seq: self.seq_front, msg, height: 0, laid: None });
    }
    self.stale = true;
  }

  fn touch(r: &mut Row) {
    r.laid = None;
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
    for r in self.rows.iter_mut().filter(|r| &*r.msg.login == login && r.msg.kind != Kind::System) {
      r.msg.flags |= flag::CLEARED;
      Self::touch(r);
      self.stale = true;
    }
  }

  // a twitch sender's heatsync inventory arrived or changed: re-resolve only their lines
  pub fn touch_sender(&mut self, uid: &str) {
    for r in self.rows.iter_mut().filter(|r| r.msg.platform == Platform::Twitch && &*r.msg.user_id == uid) {
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

  pub fn scroll_by(&mut self, px: i32, res: &mut Res) {
    let max = self.content_height(res);
    self.scroll = (self.scroll + px).clamp(0, max);
    self.stale = true;
  }

  pub fn scroll_to_bottom(&mut self) {
    self.scroll = 0;
    self.stale = true;
  }

  pub fn scroll_to_top(&mut self, res: &mut Res) {
    self.scroll = self.content_height(res);
    self.stale = true;
  }

  fn content_height(&mut self, res: &mut Res) -> i32 {
    let (w, ts, tags) = (self.width, self.timestamps, self.tags);
    let mut total = 0;
    for r in &mut self.rows {
      if r.height == 0 {
        r.height = layout_row(res, &self.channel, &r.msg, w, ts, tags).layout.height().ceil() as i32;
      }
      total += r.height;
    }
    total
  }

  // what the next frame needs, given what changed since the last draw
  pub fn take_paint(&mut self, res: &mut Res, view_h: i32) -> Paint {
    let n = std::mem::take(&mut self.pending);
    let mut shift = 0;
    let (w, ts, tags) = (self.width, self.timestamps, self.tags);
    for r in self.rows.iter_mut().rev().take(n) {
      let laid = layout_row(res, &self.channel, &r.msg, w, ts, tags);
      r.height = laid.layout.height().ceil() as i32;
      shift += r.height;
      r.laid = Some(Box::new(laid));
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

  // the pixels in `area` moved up by `dy`: keep tracked animations on their emotes
  pub fn scrolled(&mut self, area: Rect, dy: i32) {
    self.anims.retain_mut(|a| {
      a.rect.y -= dy;
      a.rect.bottom() > area.y
    });
  }

  // draw rows bottom-up into `area`, touching only pixels inside `clip`
  pub fn draw(&mut self, c: &mut Canvas, res: &mut Res, area: Rect, clip: Rect) {
    let clip = clip.intersect(area);
    c.fill(clip, theme::BLACK);
    self.anims.retain(|a| a.rect.intersect(clip).is_empty());
    let (w, ts, tags, zebra) = (self.width, self.timestamps, self.tags, self.zebra);
    let mut bottom = area.bottom() + self.scroll;
    let mut drawn_lo = self.rows.len();
    for (i, r) in self.rows.iter_mut().enumerate().rev() {
      if bottom <= area.y {
        break;
      }
      if r.height == 0 || (r.laid.is_none() && bottom - r.height < clip.bottom()) {
        let laid = layout_row(res, &self.channel, &r.msg, w, ts, tags);
        r.height = laid.layout.height().ceil() as i32;
        r.laid = Some(Box::new(laid));
      }
      let top = bottom - r.height;
      if top < clip.bottom() && bottom > clip.y {
        let row = Rect::new(area.x, top, area.w, r.height);
        let bg = if zebra && r.seq % 2 == 0 { theme::ZEBRA } else { theme::BLACK };
        if bg != theme::BLACK {
          c.fill(row.intersect(clip), bg);
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
        if let Some(laid) = &r.laid {
          let Res { text, images, .. } = res;
          let anims = &mut self.anims;
          let x = (area.x + MARK_W) as f32 + theme::PAD_X;
          text.draw(&laid.layout, c, x, top as f32, clip, |c, b| {
            let Some(eb) = laid.boxes.get(b.id as usize) else {
              return;
            };
            let rect =
              Rect::new(b.x.round() as i32, b.y.round() as i32, b.width.round() as i32, b.height.round() as i32);
            let mut imgs = Vec::with_capacity(1 + eb.zw.len());
            imgs.push(eb.img);
            imgs.extend_from_slice(&eb.zw);
            if let Some(bg) = eb.bg {
              c.fill(rect.intersect(clip), bg);
            }
            let mut flip = None;
            blit_emote(c, images, &imgs, rect, clip, &mut flip);
            if flip.is_some() {
              anims.push(Anim { rect: rect.intersect(area), bg, imgs });
            }
          });
        }
      }
      drawn_lo = i;
      bottom = top;
    }
    // layouts that scrolled out of view are dropped; heights stay
    for r in self.rows.iter_mut().take(drawn_lo) {
      r.laid = None;
    }
    self.next_anim = self.soonest(res);
  }

  fn soonest(&self, res: &mut Res) -> Option<u32> {
    let mut next = None;
    for a in &self.anims {
      for &id in &a.imgs {
        if let Some((_, _, _, Some(ms))) = res.images.frame(id) {
          next = Some(next.map_or(ms, |n: u32| n.min(ms)));
        }
      }
    }
    next
  }

  // repaint only on-screen animated emotes; returns the damaged region
  pub fn animate(&mut self, c: &mut Canvas, res: &mut Res, area: Rect) -> Option<Rect> {
    let mut dirty: Option<Rect> = None;
    let mut next = None;
    for a in &self.anims {
      let r = a.rect.intersect(area);
      if r.is_empty() {
        continue;
      }
      c.fill(r, a.bg);
      blit_emote(c, &mut res.images, &a.imgs, a.rect, r, &mut next);
      dirty = Some(dirty.map_or(r, |d| super::union(d, r)));
    }
    self.next_anim = next;
    dirty
  }
}
