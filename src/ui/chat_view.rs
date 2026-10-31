// virtualized chat rows: heights for every row, layouts only for rows on screen
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use parley::{InlineBox, InlineBoxKind, PositionedLayoutItem, VerticalAlign};

use super::canvas::{Canvas, Rect};
use super::name_art::{self, ArtKey, PAINT_IMG_PX};
use super::res::{BADGE_PX, EMOTE_PX, Res};
use super::text::{Ink, Span, TextLayout};
use super::theme;
use crate::cosmetics::paint::{Fill, Paint as NamePaint};
use crate::emotes::block::Scope;
use crate::images::{Images, ImgId};
use crate::model::message::{Kind, Message, Platform, flag};
use crate::model::seen::Seen;
use crate::util::time::hhmm;

// one emote slot in a line; zero-width overlays stack on the base image
struct EmoteBox {
  img: ImgId,
  zw: Vec<ImgId>,
  // tile color behind the image (ffz badges)
  bg: Option<u32>,
  // byte range of the word in the message body; 0..0 for badges
  word: (u32, u32),
  // blocked: the image is never drawn or fetched, a dashed outline holds the slot
  blocked: bool,
}

// an emote under the pointer
pub struct EmoteHit {
  pub name: Box<str>,
  pub url: Box<str>,
  pub zero_width: bool,
  pub scope: Scope,
}

// a username wearing a 7tv paint: where the shaded bitmap goes (layout coords)
struct PaintedName {
  uid: u64,
  nick: Box<str>,
  color: u32,
  paint: Arc<NamePaint>,
  x: f32,
  w: f32,
  baseline: f32,
}

struct Laid {
  layout: TextLayout,
  boxes: Vec<EmoteBox>,
  name: Option<PaintedName>,
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
  // the line picked with `v` / a click (by seq), outlined; replies and actions act on it
  selected: Option<i64>,
  // yellow edge (magenta = twitch first message ever) on a person's first line this session (setting first_glow)
  pub glow: bool,
  seen: Seen,
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
  let mut painted = None;
  if m.kind == Kind::System {
    // stream lines carry their color (green live, magenta game switch); notices stay dim
    if m.color == 0 {
      dim(&mut s, &mut spans, &m.body);
    } else {
      let a = s.len();
      s.push_str(&m.body);
      spans.push(Span { range: a..s.len(), ink: Ink(m.color), bold: false });
    }
  } else {
    badges(res, chan, m, &mut s, &mut ib, &mut boxes);
    let n0 = s.len();
    s.push_str(&m.nick);
    // a painted name's glyphs are drawn from the shaded bitmap instead (paints: names only)
    painted = twitch_uid(m).and_then(|uid| Some((uid, res.cosmetics.get(uid)?.paint.clone()?)));
    let tag = if painted.is_some() { Ink::PAINTED } else { 0 };
    spans.push(Span { range: n0..s.len(), ink: Ink(m.color | tag), bold: true });
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
  let name = painted.and_then(|(uid, paint)| {
    let (x, w, baseline) = painted_run(&layout)?;
    Some(PaintedName { uid, nick: m.nick.clone(), color: m.color, paint, x, w, baseline })
  });
  Laid { layout, boxes, name }
}

fn twitch_uid(m: &Message) -> Option<u64> {
  (m.platform == Platform::Twitch).then(|| m.user_id.parse().ok()).flatten()
}

// (x, width, baseline) of the painted name: its glyph runs, which share a line
fn painted_run(layout: &TextLayout) -> Option<(f32, f32, f32)> {
  let mut found: Option<(f32, f32, f32)> = None;
  for line in layout.lines() {
    for item in line.items() {
      let PositionedLayoutItem::GlyphRun(gr) = item else { continue };
      if gr.style().brush.0 & Ink::PAINTED == 0 {
        continue;
      }
      let (a, b) = (gr.offset(), gr.offset() + gr.advance());
      found = Some(match found {
        None => (a, b, gr.baseline()),
        Some((x0, x1, base)) => (x0.min(a), x1.max(b), base),
      });
    }
    if found.is_some() {
      break;
    }
  }
  found.map(|(x0, x1, base)| (x0, x1 - x0, base))
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
  let mut found: Vec<(Box<str>, Option<u32>)> =
    res.badges.for_message(chan, m).into_iter().map(|b| (b.url.clone(), b.bg)).collect();
  // 7tv's badge sits last, right before the name (ext order). seeing a chatter is also what
  // asks heatsync for their cosmetics: batched, cached, answered into the same slot
  if let Some(uid) = twitch_uid(m)
    && let Some(b) = res.cosmetics.get(uid).and_then(|c| c.badge.as_ref())
  {
    found.push((b.url.clone(), b.bg));
  }
  let h = res.images.px(BADGE_PX);
  for (url, bg) in found {
    let img = res.images.id(&url, BADGE_PX);
    let w = res.images.width(img, (18, 18));
    ib.push(inline(boxes.len(), s.len(), w, h));
    boxes.push(EmoteBox { img, zw: Vec::new(), bg, word: (0, 0), blocked: false });
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
  for word in m.body.split(' ') {
    let (a, b) = (at, at + word.len());
    at = b + 1;
    let found = find_word(res, chan, m, a, b, word);
    let blocked = found.as_ref().is_some_and(|f| res.blocked.contains(word, &f.0));
    match found {
      // a blocked overlay just vanishes; a blocked base keeps its slot
      Some((_, _, true, _)) if prev_emote && blocked => continue,
      Some((url, _, true, _)) if prev_emote => {
        let img = res.images.id(&url, EMOTE_PX);
        if let Some(last) = boxes.last_mut() {
          last.zw.push(img);
        }
        continue;
      }
      Some((url, dims, _, _)) => {
        if !first {
          s.push(' ');
        }
        let img = res.images.id(&url, EMOTE_PX);
        let w = res.images.width(img, dims);
        ib.push(inline(boxes.len(), s.len(), w, h));
        boxes.push(EmoteBox { img, zw: Vec::new(), bg: None, word: (a as u32, b as u32), blocked });
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

// the word at body bytes a..b as an emote: twitch's own by position, else by name
fn find_word(res: &mut Res, chan: &str, m: &Message, a: usize, b: usize, word: &str) -> Option<Found> {
  let sender = (m.platform == Platform::Twitch).then(|| m.user_id.parse::<u64>().ok()).flatten();
  match m.emotes.iter().find(|(r, _)| r.start as usize == a && r.end as usize == b) {
    Some((_, url)) => Some((url.clone(), (0, 0), false, Scope::Native)),
    None if word.is_empty() => None,
    None => resolve(res, chan, m, sender, word),
  }
}

// ext order: the line's own server-verified refs, the channel catalog, the sender's heatsync
// inventory, then globals. the sender's set is the only heatsync source (privacy invariant)
fn resolve(res: &mut Res, chan: &str, m: &Message, sender: Option<u64>, word: &str) -> Option<Found> {
  let e = |e: &crate::emotes::Emote, sc| (e.url.clone(), (e.w, e.h), e.zero_width, sc);
  if let Some((_, u, zw)) = m.hs.iter().find(|(w, _, _)| **w == *word) {
    return Some((u.clone(), (0, 0), *zw, Scope::Hs));
  }
  if let Some(x) = res.emotes.channel(chan, word) {
    return Some(e(x, Scope::Channel));
  }
  if let Some(x) = sender.and_then(|id| res.senders.lookup(id, word, m.ts)) {
    return Some((x.url.clone(), (x.w, x.h), x.zero_width, Scope::Hs));
  }
  if m.has(flag::OWN)
    && let Some(x) = res.emotes.twitch_global(word)
  {
    return Some(e(x, Scope::Global));
  }
  res.emotes.global(word).map(|x| e(x, Scope::Global))
}

type Found = (Box<str>, (u16, u16), bool, Scope);

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
      selected: None,
      glow: true,
      seen: Seen::default(),
    }
  }

  pub fn len(&self) -> usize {
    self.rows.len()
  }

  pub fn push(&mut self, mut msg: Message) {
    // seen is marked even with the glow off, so toggling it on doesn't light up regulars
    if msg.kind != Kind::System && msg.system.is_none() {
      let who = if msg.login.is_empty() { &msg.nick } else { &msg.login };
      if self.seen.first(who) && !msg.has(flag::FIRST) {
        msg.flags |= flag::NEW;
      }
    }
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

  // recent chatters, newest first (tab-complete after `@`)
  pub fn chatters(&self) -> Vec<&str> {
    let mut seen = HashSet::new();
    self
      .rows
      .iter()
      .rev()
      .filter(|r| r.msg.kind != Kind::System && !r.msg.nick.is_empty())
      .map(|r| &*r.msg.nick)
      .filter(|n| seen.insert(*n))
      .take(300)
      .collect()
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

  fn height_at(&mut self, i: usize, res: &mut Res) -> i32 {
    let (w, ts, tags) = (self.width, self.timestamps, self.tags);
    let r = &mut self.rows[i];
    if r.height == 0 {
      r.height = layout_row(res, &self.channel, &r.msg, w, ts, tags).layout.height().ceil() as i32;
    }
    r.height
  }

  fn index_of(&self, seq: i64) -> Option<usize> {
    let i = seq.checked_sub(self.rows.front()?.seq)?;
    usize::try_from(i).ok().filter(|&i| self.rows.get(i).is_some_and(|r| r.seq == seq))
  }

  pub fn selected(&self) -> Option<&Message> {
    Some(&self.rows[self.index_of(self.selected?)?].msg)
  }

  pub fn deselect(&mut self) {
    if self.selected.take().is_some() {
      self.stale = true;
    }
  }

  // move the selection `by` lines (negative = older), skipping system lines; starts at the newest.
  // scrolls just enough to keep it on screen
  pub fn select_move(&mut self, by: i32, res: &mut Res, view_h: i32) {
    let Some(last) = self.rows.len().checked_sub(1) else { return };
    let mut i = match self.selected.and_then(|s| self.index_of(s)) {
      Some(i) => i,
      None => last + 1,
    } as i64;
    let mut left = by.unsigned_abs().max(1);
    let step = if by < 0 { -1 } else { 1 };
    let mut found = None;
    while left > 0 {
      i += step;
      let Some(r) = usize::try_from(i).ok().and_then(|j| self.rows.get(j)) else { break };
      if r.msg.kind != Kind::System {
        found = Some(i as usize);
        left -= 1;
      }
    }
    let Some(i) = found.or_else(|| self.selected.and_then(|s| self.index_of(s))) else { return };
    self.selected = Some(self.rows[i].seq);
    // distance from the bottom of the content to the bottom of this row
    let below: i32 = (i + 1..self.rows.len()).map(|j| self.height_at(j, res)).sum();
    let h = self.height_at(i, res);
    if below < self.scroll {
      self.scroll = below;
    } else if below + h > self.scroll + view_h {
      self.scroll = below + h - view_h;
    }
    self.stale = true;
  }

  // the line under y (in `area`'s coordinates), as drawn last
  pub fn select_at(&mut self, area: Rect, y: i32, res: &mut Res) -> bool {
    let mut bottom = area.bottom() + self.scroll;
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        break;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        if self.rows[i].msg.kind == Kind::System {
          return false;
        }
        self.selected = Some(self.rows[i].seq);
        self.stale = true;
        return true;
      }
      bottom = top;
    }
    false
  }

  // the emote under (x, y), as drawn last. badges and plain text are not emotes
  pub fn emote_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<EmoteHit> {
    let mut bottom = area.bottom() + self.scroll;
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        let m = &self.rows[i].msg;
        let laid = layout_row(res, &self.channel, m, self.width, self.timestamps, self.tags);
        let (lx, ly) = ((x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32);
        let eb = laid.layout.lines().flat_map(|l| l.items().collect::<Vec<_>>()).find_map(|it| match it {
          PositionedLayoutItem::InlineBox(b)
            if (b.x..b.x + b.width).contains(&lx) && (b.y..b.y + b.height).contains(&ly) =>
          {
            laid.boxes.get(b.id as usize)
          }
          _ => None,
        })?;
        let (a, b) = (eb.word.0 as usize, eb.word.1 as usize);
        let word = m.body.get(a..b).filter(|w| !w.is_empty())?;
        let (url, _, zero_width, scope) = find_word(res, &self.channel, m, a, b, word)?;
        return Some(EmoteHit { name: word.into(), url, zero_width, scope });
      }
      bottom = top;
    }
    None
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
        } else if r.msg.has(flag::NEW) && self.glow {
          Some(theme::YELLOW)
        } else {
          None
        };
        if let Some(m) = mark {
          c.fill(Rect::new(area.x, top, MARK_W, r.height).intersect(clip), m);
        }
        if self.selected == Some(r.seq) {
          // 1px white outline: emphasis by border, never a fill behind text
          for e in [
            Rect::new(area.x, top, area.w, 1),
            Rect::new(area.x, top + r.height - 1, area.w, 1),
            Rect::new(area.x, top, 1, r.height),
            Rect::new(area.right() - 1, top, 1, r.height),
          ] {
            c.fill(e.intersect(clip), theme::WHITE);
          }
        }
        if let Some(laid) = &r.laid {
          let Res { text, images, arts, .. } = res;
          let anims = &mut self.anims;
          let x = (area.x + MARK_W) as f32 + theme::PAD_X;
          text.draw(&laid.layout, c, x, top as f32, clip, |c, b| {
            let Some(eb) = laid.boxes.get(b.id as usize) else {
              return;
            };
            let rect =
              Rect::new(b.x.round() as i32, b.y.round() as i32, b.width.round() as i32, b.height.round() as i32);
            if eb.blocked {
              c.dashed(rect, clip, theme::WHITE);
              return;
            }
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
          if let Some(n) = &laid.name {
            let key = ArtKey {
              uid: n.uid,
              color: n.color,
              font_px: text.font_px.to_bits(),
              paint: Arc::as_ptr(&n.paint) as usize,
            };
            // image paints wait for their (still) picture; the first bitmap is redone when it lands
            let pic_id = match &n.paint.fill {
              Fill::Image { still } => Some(images.id(still, PAINT_IMG_PX)),
              _ => None,
            };
            let pic = pic_id.and_then(|id| images.frame(id)).map(|(f, w, h, _)| (&f.rgba[..], w, h));
            let w = n.w.ceil() as i32;
            let art = arts.get(key, &n.nick, pic.is_some() || pic_id.is_none(), || {
              name_art::build(text, &n.nick, w, n.color, &n.paint, pic)
            });
            let (ax, ay) = ((x + n.x).round() as i32, (top as f32 + n.baseline - art.base).round() as i32);
            c.rgba(ax, ay, art.w, art.h, &art.rgba, clip);
          }
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::fake::Fake;

  #[test]
  fn seventv_badge_follows_the_cosmetics_answer() {
    use crate::badges::Badge;
    use crate::cosmetics::Cosmetic;
    let mut res = Res::new(1.0);
    let mut m = Message::system(1, Platform::Twitch, "x");
    m.kind = Kind::Chat;
    m.user_id = "42".into();
    m.nick = "bob".into();
    m.body = "hi".into();
    let n = |res: &mut Res| layout_row(res, "c", &m, 400, false, false).boxes.len();
    // unknown chatter: no badge yet, but the ask is queued
    assert_eq!(n(&mut res), 0);
    assert_eq!(res.cosmetics.take_wanted(), vec![42]);
    let badge = Badge { url: "https://cdn.7tv.app/badge/B/2x.webp".into(), title: "t".into(), bg: None };
    res.cosmetics.put(42, Some(Cosmetic { paint: None, badge: Some(badge) }));
    assert_eq!(n(&mut res), 1);
    res.cosmetics.on = false;
    assert_eq!(n(&mut res), 0);
  }

  #[test]
  fn painted_name_draws_shaded_and_only_the_name() {
    use crate::cosmetics::{Cosmetic, paint::Paint as P};
    let mut res = Res::new(1.0);
    let paint = P::parse(&serde_json::json!({"id": "G", "function": "linear-gradient", "angle": 90,
      "stops": [{"at": 0, "color": 0xff0000ffu32 as i32}, {"at": 1, "color": 0x0000ffffu32 as i32}]}))
    .unwrap();
    let mut m = Message::system(1, Platform::Twitch, "x");
    m.kind = Kind::Chat;
    m.user_id = "42".into();
    m.nick = "wwwwwwwwwwww".into();
    m.color = theme::GREEN;
    m.body = "wwwwwwwwwwww".into();
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.resize(400);
    v.push(m);
    let area = Rect::new(0, 0, 400, 40);
    let mut c = Canvas::new(400, 40);
    // unknown chatter: plain green name, ask queued
    v.draw(&mut c, &mut res, area, area);
    assert_eq!(res.cosmetics.take_wanted(), vec![42]);
    let tint = |c: &Canvas| {
      let (mut red, mut blue, mut green) = (0, 0, 0);
      for p in &c.px {
        let (r, g, b) = (p >> 16 & 255, p >> 8 & 255, p & 255);
        red += (r > 200 && g < 60 && b < 100) as u32;
        blue += (b > 200 && r < 100 && g < 60) as u32;
        green += (g > 200 && r < 60 && b < 60) as u32;
      }
      (red, blue, green)
    };
    let (r0, b0, g0) = tint(&c);
    assert!(g0 > 20 && r0 == 0 && b0 == 0);
    res.cosmetics.put(42, Some(Cosmetic { paint: Some(Arc::new(paint)), badge: None }));
    v.touch_sender("42");
    v.draw(&mut c, &mut res, area, area);
    let (r1, b1, g1) = tint(&c);
    // the name now runs red to blue; the body line is untouched (still plain white, no green left)
    assert!(r1 > 10 && b1 > 10 && g1 == 0, "{r1} {b1} {g1}");
    assert!(res.arts.len() == 1);
    // switching paints off puts the plain name back
    res.cosmetics.on = false;
    v.relayout();
    v.draw(&mut c, &mut res, area, area);
    assert_eq!(tint(&c).2, g0);
  }

  #[test]
  fn first_line_per_person_glows_once() {
    let mut v = ChatView::new(100, "c".into());
    let mk = |login: &str, first: bool| {
      let mut m = Message::system(1, Platform::Twitch, "x");
      m.kind = Kind::Chat;
      m.login = login.into();
      if first {
        m.flags |= flag::FIRST;
      }
      m
    };
    for m in [mk("a", false), mk("A", false), mk("b", true), mk("c", false)] {
      v.push(m);
    }
    v.push(Message::system(2, Platform::Twitch, "sys"));
    let new: Vec<bool> = v.rows.iter().map(|r| r.msg.has(flag::NEW)).collect();
    // second "a" is not new; a channel-first line keeps its own (magenta) mark
    assert_eq!(new, [true, false, false, true, false]);
  }

  #[test]
  fn selection_walks_lines_and_skips_system() {
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(100, "c".into());
    v.resize(400);
    let mut f = Fake::new(1);
    for i in 0..5 {
      let mut m = f.message(i);
      m.kind = Kind::Chat;
      m.id = format!("m{i}").into();
      v.push(m);
    }
    v.push(Message::system(9, Platform::Twitch, "joined"));
    assert!(v.selected().is_none());
    v.select_move(-1, &mut res, 1000);
    assert_eq!(&*v.selected().unwrap().id, "m4");
    v.select_move(-2, &mut res, 1000);
    assert_eq!(&*v.selected().unwrap().id, "m2");
    // past the newest line it stays put rather than landing on the system line
    v.select_move(5, &mut res, 1000);
    assert_eq!(&*v.selected().unwrap().id, "m4");
    v.deselect();
    assert!(v.selected().is_none());
  }

  #[test]
  fn emote_hit_testing_and_blocked_slot() {
    use crate::emotes::{Emote, Provider};
    let mut res = Res::new(1.0);
    let url = "https://cdn.7tv.app/emote/01ABC/1x.webp";
    let emote = Emote { url: url.into(), w: 28, h: 28, zero_width: false };
    res.emotes.set(None, Provider::SevenTv, [("Kappa".into(), emote)].into());
    let mut v = ChatView::new(10, "c".into());
    v.resize(400);
    let mut m = Fake::new(1).message(1);
    m.kind = Kind::Chat;
    m.emotes = Box::new([]);
    m.body = "hi Kappa yo".into();
    v.push(m);
    let area = Rect::new(0, 0, 400, 200);
    let scan = |v: &mut ChatView, res: &mut Res| {
      (0..400).step_by(2).find_map(|x| (150..200).find_map(|y| v.emote_at(area, x, y, res)))
    };
    let hit = scan(&mut v, &mut res).expect("emote under the pointer");
    assert_eq!((&*hit.name, &*hit.url), ("Kappa", url));
    // plain words are not emotes
    assert!(v.emote_at(area, 10, 199, &mut res).is_none());
    // blocked: still hit-testable (so a click can unblock) and still holds its slot
    res.blocked.block("Kappa", url, Scope::Channel);
    v.relayout();
    assert_eq!(&*scan(&mut v, &mut res).unwrap().name, "Kappa");
  }
}
