// virtualized chat rows: heights for every row, layouts only for rows on screen
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use parley::{Cluster, InlineBox, InlineBoxKind, PositionedLayoutItem, VerticalAlign};

use super::canvas::{Canvas, Rect};
use super::embed;
use super::name_art::anim::AnimArt;
use super::name_art::{self, ArtKey, PAINT_IMG_PX};
use super::res::{BADGE_PX, EMBED_PX, EMOTE_PX, Res};
use super::text::{Ink, Span, TextLayout};
use super::theme;
use crate::cosmetics::paint::{Fill, Paint as NamePaint};
use crate::emotes::block::Scope;
use crate::images::{Images, ImgId};
use crate::model::filter;
use crate::model::find::Find;
use crate::model::links::Link;
use crate::model::message::{Kind, Message, Platform, Seg, flag};
use crate::model::seen::Seen;
use crate::util::time::{hhmm, now_ms};

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
  // where the name sits in the laid-out text (empty for notices)
  nick: std::ops::Range<usize>,
  boxes: Vec<EmoteBox>,
  name: Option<PaintedName>,
  // the line's inline picture, asked for at draw time (never for rows only measured)
  embed: Option<ImgId>,
  // links in the words, and where each one's text sits in the laid-out text
  links: Vec<Link>,
  link_runs: Vec<(std::ops::Range<usize>, usize)>,
}

struct Row {
  // parity drives zebra; backfill counts down from 0 so stripes stay alternating
  seq: i64,
  msg: Message,
  height: i32,
  laid: Option<Box<Laid>>,
  // the filters' verdict (model::filter HIDDEN / MUTED / MARKED), kept off the data so it can change
  look: u8,
  // the tab's search doesn't match this line: out of sight like a filtered one
  miss: bool,
}

impl Row {
  fn hidden(&self) -> bool {
    self.look & filter::HIDDEN != 0 || self.miss
  }

  fn muted(&self) -> bool {
    self.look & filter::MUTED != 0
  }
}

// an animated emote currently on screen: redrawn alone when its frame flips
struct Anim {
  rect: Rect,
  bg: u32,
  imgs: Vec<ImgId>,
}

// an animated painted name on screen: redrawn alone on its tick, only while it is in view
struct NameAnim {
  rect: Rect,
  bg: u32,
  key: ArtKey,
  // motion effects tick at the display cap, a scene alone at the ext's 24 steps a second
  smooth: bool,
}

const TICK_SMOOTH_MS: u32 = 33;
const TICK_SCENE_MS: u32 = 42;

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
  name_anims: Vec<NameAnim>,
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
  // a list read from the top (the feed, newest first): `scroll` counts px down from the first row
  // instead of up from the last, and it never auto-follows
  pub top: bool,
  view_h: i32,
  // dim text shown while there are no lines (what this tab is for, or why it is empty)
  empty: Box<str>,
  // a channel tab: the chat filters (hide bots, mutes, blocks…) apply to its lines
  pub filtered: bool,
  // people hidden in this tab only (/hide name); gone on restart, like the ext
  tab_hidden: HashSet<Box<str>>,
  // the text of the last line shown, for hide duplicates
  last_text: Box<str>,
  // the search typed with / or ctrl-f: only matching lines show, hits in reverse video
  search: Option<Find>,
}

pub enum Paint {
  Nothing,
  // rows appended while pinned: blit up by `shift`, draw only the freed strip
  Append { shift: i32 },
  Full,
}

const MARK_W: i32 = 2;

// the room a line resolves its emotes and badges against: its own when copied into another tab
fn room<'a>(m: &'a Message, tab: &'a str) -> &'a str {
  m.x.as_deref().map(|x| &*x.chan).filter(|c| !c.is_empty()).unwrap_or(tab)
}

fn seg(s: &mut String, spans: &mut Vec<Span>, g: &Seg) {
  let a = s.len();
  s.push_str(&g.text);
  spans.push(Span { range: a..s.len(), ink: Ink(g.ink), bold: g.bold });
}

// what a row's layout depends on besides the line itself
#[derive(Clone, Copy)]
struct Opts<'a> {
  ts: bool,
  tags: bool,
  find: Option<&'a Find>,
}

// built from fields, not a method, so a row can be borrowed mutably next to it
macro_rules! opts {
  ($v:expr) => {
    Opts { ts: $v.timestamps, tags: $v.tags, find: $v.search.as_ref() }
  };
}

fn layout_row(res: &mut Res, chan: &str, m: &Message, muted: bool, width: i32, o: Opts) -> Laid {
  let Opts { ts, tags, find } = o;
  let mut s = String::with_capacity(m.nick.len() + m.body.len() + 16);
  let mut spans = Vec::with_capacity(4);
  // a line copied into another tab (mentions) still resolves against its own room
  let x = m.x.as_deref();
  let chan = room(m, chan);
  let dim = |s: &mut String, spans: &mut Vec<Span>, t: &str| {
    let a = s.len();
    s.push_str(t);
    spans.push(Span { range: a..s.len(), ink: Ink(theme::DIM), bold: false });
  };
  if let Some(sys) = &m.system {
    dim(&mut s, &mut spans, sys);
    s.push('\n');
  }
  if let Some(parent) = m.reply_to.as_ref().filter(|_| !muted) {
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
  if let Some(x) = x.filter(|x| !x.chan.is_empty() && m.kind != Kind::System) {
    dim(&mut s, &mut spans, &format!("#{} ", x.chan));
  }
  let mut ib = Vec::new();
  let mut boxes = Vec::new();
  let mut links = Vec::new();
  let mut link_runs = Vec::new();
  let mut painted = None;
  let mut embed = None;
  let mut nick = 0..0;
  // where the words sit in the text, for search hits
  let mut words = 0..0;
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
    for g in x.map_or(&[][..], |x| &x.head) {
      seg(&mut s, &mut spans, g);
    }
    badges(res, chan, m, &mut s, &mut ib, &mut boxes);
    let n0 = s.len();
    s.push_str(&m.nick);
    nick = n0..s.len();
    // a painted name's glyphs are drawn from the shaded bitmap instead (paints: names only)
    painted = twitch_uid(m).and_then(|uid| Some((uid, res.cosmetics.get(uid)?.paint.clone()?)));
    let tag = if painted.is_some() { Ink::PAINTED } else { 0 };
    spans.push(Span { range: n0..s.len(), ink: Ink(m.color | tag), bold: true });
    for g in x.map_or(&[][..], |x| &x.tail) {
      seg(&mut s, &mut spans, g);
    }
    if m.kind == Kind::Chat {
      s.push(':');
    }
    s.push(' ');
    // a muted sender's words and pictures are stripped; the name and time stay
    let (pic, fold) = if muted { (None, None) } else { picture(res, m, width) };
    let b0 = s.len();
    if !muted {
      (links, link_runs) = body(res, chan, m, fold.as_ref(), &mut s, &mut ib, &mut boxes);
      // a link's words are drawn in link blue
      for (r, _) in &link_runs {
        spans.push(Span { range: r.clone(), ink: Ink(theme::BLUE), bold: false });
      }
    }
    let ink = match (m.has(flag::CLEARED), m.kind) {
      (true, _) => Some(theme::DIM),
      (_, Kind::Action) => Some(m.color),
      _ => None,
    };
    if let Some(c) = ink {
      spans.push(Span { range: b0..s.len(), ink: Ink(c), bold: false });
    }
    words = b0..s.len();
    // the picture sits on its own line under the text once its size is known
    if let Some(id) = pic {
      embed = Some(id);
      if let Some((w, h)) = res.images.dims(id) {
        s.push('\n');
        ib.push(inline(boxes.len(), s.len(), w, h));
        boxes.push(EmoteBox { img: id, zw: Vec::new(), bg: None, word: (0, 0), blocked: false });
      }
    }
  }
  if let Some(f) = find {
    // a painted name is drawn from its own bitmap: leave it be
    let name = if painted.is_some() { 0..0 } else { nick.clone() };
    let words = if m.kind == Kind::System { 0..s.len() } else { words };
    hits(f, &s, &name, words, &mut spans);
  }
  let layout = res.text.layout(&s, &spans, &ib, (width - MARK_W) as f32 - 2.0 * theme::PAD_X);
  let name = painted.and_then(|(uid, paint)| {
    let (x, w, baseline) = painted_run(&layout)?;
    Some(PaintedName { uid, nick: m.nick.clone(), color: m.color, paint, x, w, baseline })
  });
  Laid { layout, nick, boxes, name, embed, links, link_runs }
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

// search hits in reverse video: the term wherever it sits in the name or the words; for @name, the name
fn hits(f: &Find, s: &str, nick: &std::ops::Range<usize>, words: std::ops::Range<usize>, spans: &mut Vec<Span>) {
  let ink = Ink(theme::BLACK | Ink::REVERSE);
  if f.user() {
    if !nick.is_empty() {
      spans.push(Span { range: nick.clone(), ink, bold: true });
    }
    return;
  }
  for r in [nick.clone(), words] {
    let Some(part) = s.get(r.clone()) else { continue };
    for h in f.spans(part) {
      spans.push(Span { range: r.start + h.start..r.start + h.end, ink, bold: false });
    }
  }
}

// the line's inline picture: its slot, and the link word to fold away once the picture is on screen
fn picture(res: &mut Res, m: &Message, width: i32) -> (Option<ImgId>, Option<std::ops::Range<usize>>) {
  if !res.media || m.has(flag::CLEARED) {
    return (None, None);
  }
  let Some(e) = embed::find(&m.body) else { return (None, None) };
  let avail = (width - MARK_W) as f32 - 2.0 * theme::PAD_X;
  let id = res.images.id_fit(&e.display, EMBED_PX, avail.max(1.0) as u32);
  if res.images.failed(id) {
    return (None, None);
  }
  let loaded = res.images.dims(id).is_some();
  (Some(id), e.word.filter(|_| loaded))
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
fn body(
  res: &mut Res,
  chan: &str,
  m: &Message,
  fold: Option<&std::ops::Range<usize>>,
  s: &mut String,
  ib: &mut Vec<InlineBox>,
  boxes: &mut Vec<EmoteBox>,
) -> (Vec<Link>, Vec<(std::ops::Range<usize>, usize)>) {
  let links = crate::model::links::find(&m.body, res.partial_links);
  let mut runs = Vec::new();
  let h = res.images.px(EMOTE_PX);
  let mut prev_emote = false;
  let mut first = true;
  let mut at = 0usize;
  for word in m.body.split(' ') {
    let (a, b) = (at, at + word.len());
    at = b + 1;
    if fold.is_some_and(|f| f.start == a && f.end == b) {
      continue;
    }
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
        let p0 = s.len();
        s.push_str(word);
        // the words of a link answer clicks
        for (k, _) in links.iter().enumerate().filter(|(_, l)| l.range.start < b && a < l.range.end) {
          runs.push((p0..s.len(), k));
        }
        prev_emote = word.is_empty() && prev_emote;
      }
    }
    first = false;
  }
  (links, runs)
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
      name_anims: Vec::new(),
      next_anim: None,
      zebra: true,
      timestamps: true,
      tags: false,
      selected: None,
      glow: true,
      seen: Seen::default(),
      top: false,
      view_h: 0,
      empty: "".into(),
      filtered: false,
      tab_hidden: HashSet::new(),
      last_text: "".into(),
      search: None,
    }
  }

  pub fn set_empty(&mut self, text: &str) {
    if *self.empty != *text {
      self.empty = text.into();
      self.stale = true;
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
    let look = self.look_of(&msg);
    let miss = self.missed(&msg);
    self.rows.push_back(Row { seq: self.seq_back, msg, height: 0, laid: None, look, miss });
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
      let miss = self.missed(&msg);
      self.rows.push_front(Row { seq: self.seq_front, msg, height: 0, laid: None, look: 0, miss });
    }
    self.refilter();
    self.stale = true;
  }

  // the search says this line is not one of its matches
  fn missed(&self, m: &Message) -> bool {
    self.search.as_ref().is_some_and(|f| !f.test(&m.nick, &m.login, &m.body))
  }

  // start, change or (empty query) end the search: lines that don't match go out of sight. returns how
  // many lines show. a picked line stays on screen; otherwise the view goes to the newest
  pub fn set_search(&mut self, q: &str, res: &mut Res, view_h: i32) -> usize {
    self.search = Find::parse(q);
    let find = self.search.as_ref();
    for r in &mut self.rows {
      r.miss = find.is_some_and(|f| !f.test(&r.msg.nick, &r.msg.login, &r.msg.body));
      Self::touch(r);
    }
    self.stale = true;
    match self.selected.and_then(|s| self.index_of(s)) {
      Some(i) if !self.rows[i].hidden() => self.reveal(i, res, view_h),
      _ => {
        self.selected = None;
        self.scroll = 0;
      }
    }
    self.found()
  }

  pub fn searching(&self) -> bool {
    self.search.is_some()
  }

  // lines on show (system notices don't count as matches)
  pub fn found(&self) -> usize {
    self.rows.iter().filter(|r| !r.hidden() && r.msg.kind != Kind::System).count()
  }

  // n / N: the next or previous match, wrapping; (place, of) for the status line. starts at the oldest (n)
  // or newest (N) like the ext
  pub fn search_step(&mut self, dir: i32, res: &mut Res, view_h: i32) -> Option<(usize, usize)> {
    let hits: Vec<usize> =
      (0..self.rows.len()).filter(|&i| !self.rows[i].hidden() && self.rows[i].msg.kind != Kind::System).collect();
    let n = hits.len() as i32;
    if n == 0 {
      return None;
    }
    let cur = self.selected.and_then(|s| self.index_of(s)).and_then(|i| hits.iter().position(|&h| h == i));
    let k = match cur {
      Some(c) => (c as i32 + dir).rem_euclid(n),
      None if dir > 0 => 0,
      None => n - 1,
    };
    let i = hits[k as usize];
    self.selected = Some(self.rows[i].seq);
    self.reveal(i, res, view_h);
    Some((k as usize + 1, n as usize))
  }

  // the filters' verdict on a new line (a list tab shows everything)
  fn look_of(&mut self, m: &Message) -> u8 {
    if !self.filtered {
      return 0;
    }
    let (hid, last) = (&self.tab_hidden, &mut self.last_text);
    filter::with(|f| f.judge(m, now_ms(), hid, last))
  }

  // the rules changed: judge every line again, oldest first
  pub fn refilter(&mut self) {
    if !self.filtered {
      return;
    }
    let now = now_ms();
    let mut last: Box<str> = "".into();
    let mut changed = false;
    filter::with(|f| {
      for r in &mut self.rows {
        let look = f.judge(&r.msg, now, &self.tab_hidden, &mut last);
        if look != r.look {
          r.look = look;
          Self::touch(r);
          changed = true;
        }
      }
    });
    self.last_text = last;
    if changed {
      // a hidden line can't stay selected
      if self.selected.and_then(|s| self.index_of(s)).is_some_and(|i| self.rows[i].hidden()) {
        self.selected = None;
      }
      self.stale = true;
    }
  }

  // /hide name and /unhide name: this tab only
  pub fn hide_user(&mut self, who: &str, hide: bool) -> bool {
    let k = filter::key(who);
    let changed = if hide { self.tab_hidden.insert(k) } else { self.tab_hidden.remove(&k) };
    if changed {
      self.refilter();
    }
    changed
  }

  fn touch(r: &mut Row) {
    r.laid = None;
    r.height = 0;
  }

  // swap a line for a new version of itself (a vote landed, a send settled); the selection stays
  pub fn replace(&mut self, id: &str, m: Message) -> bool {
    let Some(i) = self.rows.iter().position(|r| &*r.msg.id == id) else { return false };
    let look =
      if self.filtered { filter::with(|f| f.judge(&m, now_ms(), &self.tab_hidden, &mut "".into())) } else { 0 };
    let miss = self.missed(&m);
    let r = &mut self.rows[i];
    r.msg = m;
    r.look = look;
    r.miss = miss;
    Self::touch(r);
    self.stale = true;
    true
  }

  // swap the whole list for a new one (a list that changes under the reader): the scroll stays and
  // the selection follows its line by id
  pub fn reload(&mut self, msgs: Vec<Message>) {
    let keep = self.selected().map(|m| m.id.clone()).filter(|id| !id.is_empty());
    let scroll = self.scroll;
    self.clear();
    self.scroll = scroll;
    for m in msgs {
      self.seq_back += 1;
      let miss = self.missed(&m);
      self.rows.push_back(Row { seq: self.seq_back, msg: m, height: 0, laid: None, look: 0, miss });
    }
    while self.rows.len() > self.cap {
      self.rows.pop_back();
    }
    self.refilter();
    self.selected = keep.and_then(|id| self.rows.iter().find(|r| *r.msg.id == *id).map(|r| r.seq));
  }

  // forget every line (a list reloading)
  pub fn clear(&mut self) {
    self.rows.clear();
    self.anims.clear();
    self.name_anims.clear();
    self.selected = None;
    self.scroll = 0;
    self.pending = 0;
    self.seq_back = 0;
    self.seq_front = 0;
    self.stale = true;
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

  // a twitch chatter's numeric id, from their latest line here
  pub fn user_id_of(&self, login: &str) -> Option<Box<str>> {
    let r = self.rows.iter().rev().find(|r| &*r.msg.login == login && !r.msg.user_id.is_empty())?;
    Some(r.msg.user_id.clone())
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
    let (w, o) = (self.width, opts!(self));
    let r = &mut self.rows[i];
    if r.hidden() {
      return 0;
    }
    if r.height == 0 {
      r.height = layout_row(res, &self.channel, &r.msg, r.muted(), w, o).layout.height().ceil() as i32;
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
      Some(i) => i as i64,
      None if self.top => -1,
      None => last as i64 + 1,
    };
    let mut left = by.unsigned_abs().max(1);
    let step = if by < 0 { -1 } else { 1 };
    let mut found = None;
    while left > 0 {
      i += step;
      let Some(r) = usize::try_from(i).ok().and_then(|j| self.rows.get(j)) else { break };
      if r.msg.kind != Kind::System && !r.hidden() {
        found = Some(i as usize);
        left -= 1;
      }
    }
    let Some(i) = found.or_else(|| self.selected.and_then(|s| self.index_of(s))) else { return };
    self.selected = Some(self.rows[i].seq);
    self.reveal(i, res, view_h);
  }

  // scroll just enough to show row `i`
  fn reveal(&mut self, i: usize, res: &mut Res, view_h: i32) {
    let h = self.height_at(i, res);
    if self.top {
      let above: i32 = (0..i).map(|j| self.height_at(j, res)).sum();
      if above < self.scroll {
        self.scroll = above;
      } else if above + h > self.scroll + view_h {
        self.scroll = above + h - view_h;
      }
    } else {
      // distance from the bottom of the content to the bottom of this row
      let below: i32 = (i + 1..self.rows.len()).map(|j| self.height_at(j, res)).sum();
      if below < self.scroll {
        self.scroll = below;
      } else if below + h > self.scroll + view_h {
        self.scroll = below + h - view_h;
      }
    }
    self.stale = true;
  }

  // the first press of `v`: the newest line in a chat, the top post in a list
  pub fn select_start(&mut self, res: &mut Res, view_h: i32) {
    self.select_move(if self.top { 1 } else { -1 }, res, view_h);
  }

  // pick the line with this id and bring it on screen. false = it is not here (scrolled out of the ring)
  pub fn select_id(&mut self, id: &str, res: &mut Res, view_h: i32) -> bool {
    if id.is_empty() {
      return false;
    }
    let Some(i) = self.rows.iter().position(|r| &*r.msg.id == id) else { return false };
    self.selected = Some(self.rows[i].seq);
    self.reveal(i, res, view_h);
    true
  }

  // the y of the last row's bottom edge as drawn
  fn edge(&mut self, area: Rect, res: &mut Res) -> i32 {
    if !self.top {
      return area.bottom() + self.scroll;
    }
    let h = self.content_height(res);
    self.scroll = self.scroll.clamp(0, (h - area.h).max(0));
    area.y + h - self.scroll
  }

  // the line under y (in `area`'s coordinates), as drawn last
  pub fn select_at(&mut self, area: Rect, y: i32, res: &mut Res) -> bool {
    let mut bottom = self.edge(area, res);
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
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
        let laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
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
        let (url, _, zero_width, scope) = find_word(res, room(m, &self.channel), m, a, b, word)?;
        return Some(EmoteHit { name: word.into(), url, zero_width, scope });
      }
      bottom = top;
    }
    None
  }

  // the link under (x, y), as drawn last: its target
  pub fn link_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<String> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
        if m.kind == Kind::System || muted {
          return None;
        }
        let mut laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
        if laid.links.is_empty() {
          return None;
        }
        let (lx, ly) = ((x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32);
        let over = laid.layout.lines().any(|line| {
          let lm = line.metrics();
          (lm.block_min_coord..lm.block_max_coord).contains(&ly)
            && line.items().any(|it| match it {
              PositionedLayoutItem::GlyphRun(gr) => {
                let at = gr.run().text_range();
                let hit = laid.link_runs.iter().any(|(r, _)| r.start < at.end && at.start < r.end);
                hit && (gr.offset()..gr.offset() + gr.advance()).contains(&lx)
              }
              _ => false,
            })
        });
        if !over {
          return None;
        }
        // the cluster under the pointer says which link, when one run holds several
        let at = Cluster::from_point(&laid.layout, lx, ly)?.0.text_range().start;
        let k = laid.link_runs.iter().find(|(r, _)| r.contains(&at)).map(|(_, k)| *k)?;
        return Some(laid.links.swap_remove(k).url);
      }
      bottom = top;
    }
    None
  }

  // the line whose name is under (x, y), as drawn last
  pub fn name_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Message> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
        if m.kind == Kind::System {
          return None;
        }
        let laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
        let (lx, ly) = ((x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32);
        let over = laid.layout.lines().any(|line| {
          let lm = line.metrics();
          (lm.block_min_coord..lm.block_max_coord).contains(&ly)
            && line.items().any(|it| match it {
              PositionedLayoutItem::GlyphRun(gr) => {
                let r = gr.run().text_range();
                r.start < laid.nick.end
                  && laid.nick.start < r.end
                  && (gr.offset()..gr.offset() + gr.advance()).contains(&lx)
              }
              _ => false,
            })
        });
        return over.then(|| m.clone());
      }
      bottom = top;
    }
    None
  }

  // what this person said here lately (oldest first), for their card
  pub fn recent_by(&self, login: &str, n: usize) -> Vec<(i64, Box<str>)> {
    let mut v: Vec<_> = self
      .rows
      .iter()
      .rev()
      .filter(|r| &*r.msg.login == login && r.msg.kind != Kind::System && !r.msg.body.is_empty())
      .take(n)
      .map(|r| (r.msg.ts, r.msg.body.clone()))
      .collect();
    v.reverse();
    v
  }

  // px > 0 moves toward older lines (up the screen)
  pub fn scroll_by(&mut self, px: i32, res: &mut Res) {
    let max = self.content_height(res);
    self.scroll =
      if self.top { (self.scroll - px).clamp(0, (max - self.view_h).max(0)) } else { (self.scroll + px).clamp(0, max) };
    self.stale = true;
  }

  pub fn scroll_to_bottom(&mut self) {
    self.scroll = if self.top { i32::MAX } else { 0 };
    self.stale = true;
  }

  pub fn scroll_to_top(&mut self, res: &mut Res) {
    self.scroll = if self.top { 0 } else { self.content_height(res) };
    self.stale = true;
  }

  fn content_height(&mut self, res: &mut Res) -> i32 {
    let (w, o) = (self.width, opts!(self));
    let mut total = 0;
    for r in self.rows.iter_mut().filter(|r| !r.hidden()) {
      if r.height == 0 {
        r.height = layout_row(res, &self.channel, &r.msg, r.muted(), w, o).layout.height().ceil() as i32;
      }
      total += r.height;
    }
    total
  }

  // what the next frame needs, given what changed since the last draw
  pub fn take_paint(&mut self, res: &mut Res, view_h: i32) -> Paint {
    self.view_h = view_h;
    let n = std::mem::take(&mut self.pending);
    let mut shift = 0;
    let (w, o) = (self.width, opts!(self));
    let mut shown = 0;
    for r in self.rows.iter_mut().rev().take(n).filter(|r| !r.hidden()) {
      let laid = layout_row(res, &self.channel, &r.msg, r.muted(), w, o);
      r.height = laid.layout.height().ceil() as i32;
      shift += r.height;
      shown += 1;
      r.laid = Some(Box::new(laid));
    }
    // hidden lines add nothing to paint
    let n = shown;
    if self.top {
      // a list never blits: whatever changed, repaint it whole
      return if std::mem::take(&mut self.stale) || n > 0 { Paint::Full } else { Paint::Nothing };
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
    self.name_anims.retain_mut(|a| {
      a.rect.y -= dy;
      a.rect.bottom() > area.y
    });
  }

  // draw rows bottom-up into `area`, touching only pixels inside `clip`
  pub fn draw(&mut self, c: &mut Canvas, res: &mut Res, area: Rect, clip: Rect) {
    let clip = clip.intersect(area);
    c.fill(clip, theme::BLACK);
    self.anims.retain(|a| a.rect.intersect(clip).is_empty());
    self.name_anims.retain(|a| a.rect.intersect(clip).is_empty());
    if self.rows.is_empty() && !self.empty.is_empty() {
      let spans = [Span { range: 0..self.empty.len(), ink: Ink(theme::DIM), bold: false }];
      let l = res.text.layout(&self.empty, &spans, &[], (self.width - MARK_W) as f32 - 2.0 * theme::PAD_X);
      res.text.draw(&l, c, (area.x + MARK_W) as f32 + theme::PAD_X, area.y as f32, clip, |_, _| {});
      return;
    }
    let (w, zebra) = (self.width, self.zebra);
    let mut bottom = self.edge(area, res);
    let o = opts!(self);
    let mut drawn_lo = self.rows.len();
    for (i, r) in self.rows.iter_mut().enumerate().rev() {
      if bottom <= area.y {
        break;
      }
      if r.hidden() {
        continue;
      }
      if r.height == 0 || (r.laid.is_none() && bottom - r.height < clip.bottom()) {
        let laid = layout_row(res, &self.channel, &r.msg, r.muted(), w, o);
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
        let mark = if r.msg.has(flag::ALERT) {
          Some(theme::RED)
        } else if r.msg.has(flag::BRAND) {
          Some(theme::HEATSYNC)
        } else if r.msg.has(flag::FIRST) {
          Some(theme::MAGENTA)
        } else if r.msg.has(flag::HIGHLIGHTED) || r.look & filter::MARKED != 0 {
          Some(theme::CYAN)
        } else if r.msg.has(flag::NEW) && self.glow {
          Some(theme::YELLOW)
        } else {
          r.msg.x.as_ref().map(|x| x.edge).filter(|&e| e != 0)
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
          if let Some(id) = laid.embed {
            images.want(id);
          }
          let anims = &mut self.anims;
          let name_anims = &mut self.name_anims;
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
            let w = n.w.ceil() as i32;
            if let Some(fx) = &n.paint.fx {
              // scene + effects: built once, then a premultiplied frame per tick
              let (aw, ah, base, animated, smooth) = {
                let a = arts.anim(key, &n.nick, |motion| AnimArt::build(text, &n.nick, w, fx, motion, n.color));
                (a.w, a.h, a.base, a.animated(), a.smooth())
              };
              let (ax, ay) = ((x + n.x).round() as i32, (top as f32 + n.baseline - base).round() as i32);
              if animated {
                if let Some((_, px)) = arts.frame(&key, name_art::clock()) {
                  c.rgba_pm(ax, ay, aw, ah, px, clip);
                }
                name_anims.push(NameAnim { rect: Rect::new(ax, ay, aw, ah), bg, key, smooth });
              } else if let Some(art) = arts.still(&key) {
                c.rgba(ax, ay, art.w, art.h, &art.rgba, clip);
              }
            } else {
              // image paints wait for their (still) picture; the first bitmap is redone when it lands
              let pic_id = match &n.paint.fill {
                Fill::Image { still } => Some(images.id(still, PAINT_IMG_PX)),
                _ => None,
              };
              let pic = pic_id.and_then(|id| images.frame(id)).map(|(f, w, h, _)| (&f.rgba[..], w, h));
              let art = arts.get(key, &n.nick, pic.is_some() || pic_id.is_none(), || {
                name_art::build(text, &n.nick, w, n.color, &n.paint, pic)
              });
              let (ax, ay) = ((x + n.x).round() as i32, (top as f32 + n.baseline - art.base).round() as i32);
              c.rgba(ax, ay, art.w, art.h, &art.rgba, clip);
            }
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
    let mut next = self.name_tick();
    for a in &self.anims {
      for &id in &a.imgs {
        if let Some((_, _, _, Some(ms))) = res.images.frame(id) {
          next = Some(next.map_or(ms, |n: u32| n.min(ms)));
        }
      }
    }
    next
  }

  // animated painted names currently on screen
  pub fn animated_names(&self) -> usize {
    self.name_anims.len()
  }

  // when the animated names on screen tick next (none on screen: nothing wakes)
  fn name_tick(&self) -> Option<u32> {
    self.name_anims.iter().map(|a| if a.smooth { TICK_SMOOTH_MS } else { TICK_SCENE_MS }).min()
  }

  // repaint only on-screen animated emotes and painted names; returns the damaged region
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
    let t = name_art::clock();
    for a in &self.name_anims {
      let r = a.rect.intersect(area);
      if r.is_empty() {
        continue;
      }
      let Some((art, px)) = res.arts.frame(&a.key, t) else { continue };
      c.fill(r, a.bg);
      c.rgba_pm(a.rect.x, a.rect.y, art.w, art.h, px, r);
      let ms = if a.smooth { TICK_SMOOTH_MS } else { TICK_SCENE_MS };
      next = Some(next.map_or(ms, |n: u32| n.min(ms)));
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
  fn picture_waits_for_its_size_then_folds_the_link() {
    use crate::images::decode::{Decoded, Frame};
    let mut res = Res::new(1.0);
    let mut m = Message::system(1, Platform::Twitch, "x");
    m.kind = Kind::Chat;
    m.nick = "bob".into();
    m.body = "look https://i.imgur.com/a.png".into();
    let boxes = |res: &mut Res| layout_row(res, "c", &m, false, 400, NO_OPTS);
    // not loaded: no box, link stays as text, and nothing was requested by measuring
    let l = boxes(&mut res);
    assert!((l.boxes.len(), l.embed.is_some()) == (0, true));
    assert!(res.images.take_wanted().is_empty());
    // drawing asks for it, fit to the row width
    res.images.want(l.embed.unwrap());
    let (id, url, target) = res.images.take_wanted().remove(0);
    assert!(url.starts_with("https://heatsync.org/api/img?url="));
    assert_eq!((target & 0xffff, target >> 16), (EMBED_PX as u32, 400 - MARK_W as u32 - 8));
    let d = Decoded { w: 120, h: 80, frames: vec![Frame { rgba: vec![0; 120 * 80 * 4].into(), delay_ms: 0 }] };
    assert!(res.images.loaded(id, target, Ok(d)));
    let l = boxes(&mut res);
    assert_eq!(l.boxes.len(), 1);
    // the same size again needs no relayout; a failure removes the slot
    assert!(!res.images.loaded(id, target, Err("x".into())));
    assert_eq!(boxes(&mut res).boxes.len(), 0);
    res.media = false;
    assert!(boxes(&mut res).embed.is_none());
  }

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
    let n = |res: &mut Res| layout_row(res, "c", &m, false, 400, NO_OPTS).boxes.len();
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
  fn heatsync_native_solid_paint_colors_the_name() {
    use crate::cosmetics::{Cosmetic, paint::Paint as P};
    for zoom in [1.0, 1.3, 2.0] {
      let mut res = Res::new(zoom);
      let spec = serde_json::json!({"v": 2, "base": {"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]},
      "glow": null, "effects": [{"id": "twirl", "speed": 1}]});
      let paint = P::from_hs(&spec).unwrap();
      // a real nl_kripp history line, through the real irc parser
      let raw = "@flags;subscriber=1;first-msg=0;rm-received-ts=1793376500527;display-name=ennortix;emotes;color=#FF0000;turbo=0;tmi-sent-ts=1793376500347;user-type;mod=0;badge-info=subscriber/55;user-id=37005319;badges=subscriber/2048,ewc-2026-diamond/1;id=2f3d789f;room-id=29795919;historical=1 :ennortix!ennortix@ennortix.tmi.twitch.tv PRIVMSG #nl_kripp :watching the mentalist while gaming Ratting";
      let (_, crate::net::irc::Chat::Msg(m)) = crate::net::irc::to_chat(&crate::net::irc::parse(raw).unwrap()).unwrap()
      else {
        panic!()
      };
      let mut v = ChatView::new(10, "c".into());
      v.zebra = false;
      v.resize(400);
      v.push(m);
      let area = Rect::new(0, 0, 400, 40);
      let mut c = Canvas::new(400, 40);
      v.draw(&mut c, &mut res, area, area);
      assert_eq!(res.cosmetics.take_wanted(), vec![37005319]);
      res.cosmetics.put(37005319, Some(Cosmetic { paint: Some(Arc::new(paint)), badge: None }));
      v.touch_sender("37005319");
      v.draw(&mut c, &mut res, area, area);
      let amber = c.px.iter().filter(|p| (*p >> 16 & 255) > 200 && (*p >> 8 & 255) > 120 && (*p & 255) < 60).count();
      let red = c.px.iter().filter(|p| (*p >> 16 & 255) > 200 && (*p >> 8 & 255) < 60 && (*p & 255) < 60).count();
      assert!(amber > 20 && red == 0, "zoom {zoom} amber {amber} red {red}");
    }
  }

  // ennortix's real paint: an animated name ticks only while it is on screen, and never with motion off
  #[test]
  fn animated_names_tick_only_while_visible() {
    use crate::cosmetics::{Cosmetic, paint::Paint as P};
    let spec = serde_json::json!({"v": 2, "base": {"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]},
      "glow": null, "scene": {"weather": {"id": "petals", "tint": "#d7408c", "speed": 1, "density": 1},
      "backdrop": {"id": "synth", "tint": "#0ca43d", "speed": 1}}, "effects": [{"id": "twirl", "speed": 1}, {"id": "coin", "speed": 1}]});
    let paint = Arc::new(P::from_hs(&spec).unwrap());
    let mut m = said("ennortix", "gg");
    m.user_id = "37005319".into();
    let setup = |motion: bool| {
      let mut res = Res::new(1.0);
      res.arts.set_motion(motion);
      let mut v = ChatView::new(10, "c".into());
      v.zebra = false;
      v.resize(400);
      v.push(m.clone());
      let area = Rect::new(0, 0, 400, 40);
      let mut c = Canvas::new(400, 40);
      v.draw(&mut c, &mut res, area, area);
      res.cosmetics.put(37005319, Some(Cosmetic { paint: Some(paint.clone()), badge: None }));
      v.touch_sender("37005319");
      v.draw(&mut c, &mut res, area, area);
      (res, v, c, area)
    };
    let (mut res, mut v, mut c, area) = setup(true);
    assert_eq!(v.next_anim, Some(TICK_SMOOTH_MS), "a visible animated name asks for ticks");
    // a viewport that does not contain the name: no repaint, no further ticks
    let out = v.animate(&mut c, &mut res, Rect::new(300, 0, 100, 40));
    assert!(out.is_none() && v.next_anim.is_none(), "off-screen: idle");
    // on screen: only the name's own rect is damaged
    let (mut res, mut v, mut c, area2) = setup(true);
    let r = v.animate(&mut c, &mut res, area2).expect("damage");
    assert!(r.w < area.w && r.h <= 40);
    assert_eq!(v.next_anim, Some(TICK_SMOOTH_MS));
    // motion off: a still frame, nothing ever ticks
    let (mut res, mut v, mut c, area) = setup(false);
    assert_eq!(v.next_anim, None);
    assert!(v.animate(&mut c, &mut res, area).is_none());
  }

  const NO_OPTS: Opts = Opts { ts: false, tags: false, find: None };

  fn said(login: &str, body: &str) -> Message {
    let mut m = Message::system(1, Platform::Twitch, "x");
    m.kind = Kind::Chat;
    m.login = login.into();
    m.nick = login.into();
    m.body = body.into();
    m
  }

  #[test]
  fn filtered_lines_take_no_room_and_come_back() {
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(50, "c".into());
    v.filtered = true;
    v.resize(400);
    filter::edit(|f| f.bots = true);
    v.push(said("bob", "hi"));
    let one = v.content_height(&mut res);
    v.push(said("nightbot", "!commands"));
    v.push(said("nightbot", "!discord"));
    assert_eq!(v.content_height(&mut res), one);
    // can't select what is hidden
    v.select_move(-1, &mut res, 100);
    assert_eq!(&*v.selected().unwrap().login, "bob");
    v.select_move(-1, &mut res, 100);
    assert_eq!(&*v.selected().unwrap().login, "bob");
    // the switch goes off: they are all back
    filter::edit(|f| f.bots = false);
    v.refilter();
    assert_eq!(v.content_height(&mut res), one * 3);
    // a list tab (not filtered) ignores the rules
    filter::edit(|f| f.bots = true);
    let mut list = ChatView::new(50, "feed".into());
    list.resize(400);
    list.push(said("nightbot", "!x"));
    assert_eq!(list.content_height(&mut res), one);
  }

  #[test]
  fn search_keeps_matching_lines_walks_them_and_clears() {
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(50, "c".into());
    v.resize(400);
    for (who, what) in [("bob", "hello world"), ("alice", "bye"), ("bobby", "HELLO again")] {
      v.push(said(who, what));
    }
    let h = v.height_at(0, &mut res);
    assert_eq!(v.content_height(&mut res), 3 * h);
    assert_eq!(v.set_search("hello", &mut res, 200), 2);
    assert_eq!(v.content_height(&mut res), 2 * h);
    // lines that arrive later are judged too
    v.push(said("carl", "hello!"));
    v.push(said("dan", "nope"));
    assert_eq!(v.found(), 3);
    assert_eq!(v.set_search("@bob", &mut res, 200), 2);
    assert_eq!(v.set_search("hello", &mut res, 200), 3);
    // n / N wrap round the three
    assert_eq!(v.search_step(1, &mut res, 200), Some((1, 3)));
    assert_eq!(&*v.selected().unwrap().login, "bob");
    assert_eq!(v.search_step(1, &mut res, 200), Some((2, 3)));
    assert_eq!(v.search_step(1, &mut res, 200), Some((3, 3)));
    assert_eq!(v.search_step(1, &mut res, 200), Some((1, 3)));
    assert_eq!(v.search_step(-1, &mut res, 200), Some((3, 3)));
    assert_eq!(v.set_search("zzz", &mut res, 200), 0);
    assert!(v.selected().is_none() && v.search_step(1, &mut res, 200).is_none());
    // clearing brings every line back
    assert_eq!(v.set_search("", &mut res, 200), 5);
    assert!(!v.searching());
    assert_eq!(v.content_height(&mut res), 5 * h);
  }

  #[test]
  fn a_hit_is_drawn_in_reverse_video() {
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(50, "c".into());
    v.resize(400);
    v.push(said("bob", "hello world"));
    let (mut c, area) = (Canvas::new(400, 60), Rect::new(0, 0, 400, 60));
    let white = |c: &Canvas| c.px.iter().filter(|&&p| p == theme::WHITE).count();
    v.draw(&mut c, &mut res, area, area);
    let plain = white(&c);
    v.set_search("hello", &mut res, 60);
    v.draw(&mut c, &mut res, area, area);
    // a solid bar behind five letters is far more white than the glyphs ever were
    assert!(white(&c) > plain + 300, "{} vs {plain}", white(&c));
  }

  #[test]
  fn a_muted_line_keeps_its_name_but_not_its_words() {
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(50, "c".into());
    v.filtered = true;
    v.resize(120);
    filter::edit(|f| _ = f.mute("quiet", None));
    // far longer than one row
    v.push(said("quiet", &"word ".repeat(80)));
    v.push(said("loud", &"word ".repeat(80)));
    let h = |v: &mut ChatView, res: &mut Res, i| v.height_at(i, res);
    assert!(h(&mut v, &mut res, 0) < h(&mut v, &mut res, 1));
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

  #[test]
  fn links_hit_test_to_their_canonical_target() {
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(10, "c".into());
    v.resize(600);
    let mut m = Fake::new(1).message(1);
    m.kind = Kind::Chat;
    m.emotes = Box::new([]);
    m.body = "aaaa watch?v=Gz0fAz9n_Os bbbb".into();
    v.push(m);
    let area = Rect::new(0, 0, 600, 200);
    let scan = |v: &mut ChatView, res: &mut Res| {
      (0..600).step_by(2).filter_map(|x| (150..200).find_map(|y| v.link_at(area, x, y, res))).collect::<Vec<_>>()
    };
    let urls = scan(&mut v, &mut res);
    assert!(!urls.is_empty() && urls.iter().all(|u| u == "https://www.youtube.com/watch?v=Gz0fAz9n_Os"), "{urls:?}");
    // plain words are not links, and the switch turns the partial rule off
    assert!(v.link_at(area, 2, 199, &mut res).is_none());
    res.partial_links = false;
    v.relayout();
    assert!(scan(&mut v, &mut res).is_empty());
  }

  fn plain(id: &str, body: &str) -> Message {
    let mut m = Message::system(1, Platform::Twitch, "x");
    m.kind = Kind::Chat;
    m.id = id.into();
    m.nick = "bob".into();
    m.body = body.into();
    m
  }

  #[test]
  fn alert_lines_get_a_red_edge_and_copies_keep_their_room() {
    use crate::model::message::{Extra, Seg};
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.glow = false;
    v.resize(400);
    let mut m = plain("a", "hello");
    v.push(m.clone());
    m.flags |= flag::ALERT;
    m.id = "b".into();
    v.push(m);
    let area = Rect::new(0, 0, 400, 80);
    let mut c = Canvas::new(400, 80);
    v.draw(&mut c, &mut res, area, area);
    let edge = |y: usize| c.px[y * 400];
    // newest line sits at the bottom, red; the one above it has no edge
    assert_eq!(edge(70), theme::RED);
    assert_eq!(edge(50), theme::BLACK);
    // a copied line draws a room tag and head/tail segs around the name
    let mut x = plain("c", "hi");
    x.x = Some(Box::new(Extra {
      chan: "forsen".into(),
      head: Box::new([Seg { text: ">>1 ".into(), ink: theme::YELLOW, bold: true }]),
      tail: Box::new([Seg { text: " 5".into(), ink: theme::WHITE, bold: false }]),
      edge: 0,
    }));
    let l = layout_row(&mut res, "tab", &x, false, 400, NO_OPTS);
    let n = l.layout.lines().count();
    assert_eq!(n, 1);
    assert!(l.layout.width() > layout_row(&mut res, "tab", &plain("c", "hi"), false, 400, NO_OPTS).layout.width());
  }

  #[test]
  fn a_top_list_reads_from_the_first_row_and_scrolls_down() {
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(50, "feed".into());
    v.top = true;
    v.zebra = false;
    v.resize(400);
    for i in 0..30 {
      v.push(plain(&format!("p{i}"), &format!("post {i}")));
    }
    let area = Rect::new(0, 0, 400, 100);
    let mut c = Canvas::new(400, 100);
    let ink = |c: &Canvas, y0: usize, y1: usize| (y0 * 400..y1 * 400).any(|i| c.px[i] != theme::BLACK);
    assert!(matches!(v.take_paint(&mut res, 100), Paint::Full));
    v.draw(&mut c, &mut res, area, area);
    // first post at the very top, and a short list hangs from the top too
    assert!(ink(&c, 0, 20));
    // scrolling "down the list" (wheel down = negative px) moves content up
    v.scroll_by(-40, &mut res);
    assert!(matches!(v.take_paint(&mut res, 100), Paint::Full));
    v.draw(&mut c, &mut res, area, area);
    assert!(ink(&c, 0, 20));
    // end of list clamps
    v.scroll_to_bottom();
    v.draw(&mut c, &mut res, area, area);
    assert!(ink(&c, 80, 100));
    v.scroll_to_top(&mut res);
    // selection starts at the top post and reveals what it moves to
    v.select_start(&mut res, 100);
    assert_eq!(&*v.selected().unwrap().id, "p0");
    v.select_move(25, &mut res, 100);
    assert_eq!(&*v.selected().unwrap().id, "p25");
    v.draw(&mut c, &mut res, area, area);
    assert!(v.select_id("p3", &mut res, 100));
    assert_eq!(&*v.selected().unwrap().id, "p3");
    assert!(!v.select_id("nope", &mut res, 100));
    // a click lands on the row drawn there
    v.scroll_to_top(&mut res);
    v.draw(&mut c, &mut res, area, area);
    assert!(v.select_at(area, 5, &mut res));
    assert_eq!(&*v.selected().unwrap().id, "p0");
  }
  #[test]
  fn replace_and_clear() {
    let mut v = ChatView::new(10, "c".into());
    v.push(plain("a", "one"));
    v.push(plain("b", "two"));
    assert!(v.replace("b", plain("b", "TWO")));
    assert!(!v.replace("zz", plain("zz", "x")));
    v.clear();
    assert_eq!(v.len(), 0);
    v.push(plain("c", "three"));
    assert_eq!(v.len(), 1);
    assert!(v.selected().is_none());
  }

  #[test]
  fn reload_keeps_scroll_and_selection() {
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(10, "c".into());
    v.top = true;
    v.resize(400);
    for id in ["a", "b", "c"] {
      v.push(plain(id, id));
    }
    assert!(v.select_id("b", &mut res, 200));
    v.reload(vec![plain("z", "new"), plain("a", "a"), plain("b", "b")]);
    assert_eq!(v.len(), 3);
    assert_eq!(&*v.selected().unwrap().id, "b");
    v.reload(vec![plain("q", "only")]);
    assert!(v.selected().is_none());
  }
}
