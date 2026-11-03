// virtualized chat rows: heights for every row, layouts only for rows on screen
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use parley::{Cluster, InlineBox, InlineBoxKind, PositionedLayoutItem, VerticalAlign};

use super::canvas::{Canvas, Rect};
use super::embed;
use super::name_art::anim::AnimArt;
use super::name_art::{self, ArtKey, PAINT_IMG_PX};
use super::res::{BADGE_PX, CARD_PX, EMBED_PX, MEDIA_MULTI_PX, MEDIA_PX, Res};
use super::text::{Ink, Span, TextLayout};
use super::theme;
use crate::cosmetics::id::PaintId;
use crate::cosmetics::paint::{Fill, Paint as NamePaint};
use crate::emotes::block::Scope;
use crate::heatsync::content;
use crate::images::{Images, ImgId};
use crate::model::filter;
use crate::model::find::Find;
use crate::model::links::Link;
use crate::model::message::{Kind, Message, Platform, Seg, flag};
use crate::model::preview::{Kind as PreviewKind, Preview};
use crate::model::seen::Seen;
use crate::model::video;
use crate::util::time::{clock, now_ms};

// one emote slot in a line; zero-width overlays stack on the base image
struct EmoteBox {
  img: ImgId,
  zw: Vec<ImgId>,
  // the body words of those overlays, same order
  zw_words: Vec<(u32, u32)>,
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
  // the zero-width overlays drawn on it, bottom to top (a blocked one is not drawn, so not here)
  pub over: Vec<Layer>,
}

// one piece of a stacked emote
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Layer {
  pub name: Box<str>,
  pub url: Box<str>,
  pub scope: Scope,
}

// a username wearing a 7tv paint: where the shaded bitmap goes (layout coords)
struct PaintedName {
  uid: PaintId,
  nick: Box<str>,
  color: u32,
  paint: Arc<NamePaint>,
  x: f32,
  w: f32,
  baseline: f32,
}

// a person named in a line's words: where the name sits in the laid-out text, who, and their paint
struct Mention {
  range: std::ops::Range<usize>,
  // lowercase, without the `@`
  login: Box<str>,
  color: u32,
  paint: Option<PaintId>,
  // the paint it wears, once their cosmetics are here
  art: Option<Arc<NamePaint>>,
}

// what a line's words gave: its links, where their text sits, and the people named
type Words = (Vec<Link>, Vec<(std::ops::Range<usize>, usize)>, Vec<Mention>);

struct Laid {
  layout: TextLayout,
  // where the name sits in the laid-out text (empty for notices)
  nick: std::ops::Range<usize>,
  // the feed's ▲ and the post's own >>id (empty on every other line)
  vote: std::ops::Range<usize>,
  thread: std::ops::Range<usize>,
  // a feed post's media: its `[img]` tag, the held box's button, and the frames drawn around them
  media: std::ops::Range<usize>,
  reveal: std::ops::Range<usize>,
  // an automod hold's allow and deny buttons
  allow: std::ops::Range<usize>,
  deny: std::ops::Range<usize>,
  // `>>id` written in the words: where, and the post (server id) it names
  refs: Vec<(std::ops::Range<usize>, Box<str>)>,
  frames: Vec<(std::ops::Range<usize>, Frame)>,
  // the post's pictures, asked for at draw time like `embed`
  pics: Vec<ImgId>,
  // the pictures that are a video's still (a ▶ over them), and what a click plays
  plays: Vec<(ImgId, Box<str>)>,
  boxes: Vec<EmoteBox>,
  // names wearing a paint: the sender's and any mentioned in the words
  names: Vec<PaintedName>,
  // people named in the words (`@bob`, or `bob` once bob has spoken), in text order
  mentions: Vec<Mention>,
  // the line's inline picture, asked for at draw time (never for rows only measured)
  embed: Option<ImgId>,
  // links in the words, and where each one's text sits in the laid-out text
  links: Vec<Link>,
  link_runs: Vec<(std::ops::Range<usize>, usize)>,
}

#[derive(Clone, Copy)]
enum Frame {
  // the held box: the dashed cyan the ext's flagged things wear
  Held,
  // its button: 1px white
  Button,
  // an automod hold's allow / deny: 1px yellow (ext .hs-mc-automod-btn)
  Hold,
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

// someone under the pointer: where (line, text offset: tells two spots on a line apart) and who
#[derive(Debug, PartialEq, Eq)]
pub struct WhoHit {
  pub seq: i64,
  pub start: usize,
  // None = the line's sender
  pub mention: Option<Mentioned>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Mentioned {
  pub login: Box<str>,
  pub color: u32,
  pub paint: Option<PaintId>,
}

// which of `ranges` (text offsets) the point (lx, ly) in layout coords is over: on a line, inside a
// glyph run that overlaps one, and the character there is inside it
fn over(layout: &TextLayout, lx: f32, ly: f32, ranges: &[std::ops::Range<usize>]) -> Option<usize> {
  let hit = layout.lines().any(|line| {
    let lm = line.metrics();
    (lm.block_min_coord..lm.block_max_coord).contains(&ly)
      && line.items().any(|it| match it {
        PositionedLayoutItem::GlyphRun(gr) => {
          let at = gr.run().text_range();
          ranges.iter().any(|r| !r.is_empty() && r.start < at.end && at.start < r.end)
            && (gr.offset()..gr.offset() + gr.advance()).contains(&lx)
        }
        _ => false,
      })
  });
  if !hit {
    return None;
  }
  let at = Cluster::from_point(layout, lx, ly)?.0.text_range().start;
  ranges.iter().position(|r| r.contains(&at))
}

pub struct ChatView {
  pub channel: Box<str>,
  rows: VecDeque<Row>,
  cap: usize,
  seq_back: i64,
  seq_front: i64,
  pub width: i32,
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
// narrowest width a row lays out at: a transient 0 (view born before the first resize) must not wrap one word per line
const MIN_W: i32 = 160;

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

// ": words" after `head`, cut with … so the two fit one line `w` wide. control chars become spaces
fn one_line(res: &mut Res, head: &str, words: &str, w: f32) -> String {
  let mut out = String::from(": ");
  out.extend(words.chars().map(|c| if c.is_control() { ' ' } else { c }));
  let full = format!("{head}{out}");
  let l = res.text.layout(&full, &[], &[], f32::MAX);
  if l.full_width() <= w {
    return out;
  }
  let ell = res.text.layout("…", &[], &[], f32::MAX).full_width();
  let y = res.text.line_px / 2.0;
  let at = Cluster::from_point(&l, (w - ell).max(0.0), y).map_or(full.len(), |c| c.0.text_range().start);
  let keep = at.saturating_sub(head.len()).min(out.len());
  let keep = (0..=keep).rev().find(|&i| out.is_char_boundary(i)).unwrap_or(0);
  out.truncate(keep);
  out.truncate(out.trim_end().len());
  out.push('…');
  out
}

fn layout_row(res: &mut Res, chan: &str, m: &Message, muted: bool, width: i32, o: Opts) -> Laid {
  let Opts { ts, tags, find } = o;
  let (ts, tags) = (ts && res.look.timestamps, tags && res.look.tags);
  // a line from someone who was timed out or banned
  let cleared = m.has(flag::CLEARED) && res.look.dim_timeouts;
  let mut s = String::with_capacity(m.nick.len() + m.body.len() + 16);
  let mut spans = Vec::with_capacity(4);
  let mut reply = None;
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
    let r0 = s.len();
    dim(&mut s, &mut spans, "↳ ");
    // the parent is always an `@name`: the same person (color, paint, lookup, hover) as one in the words
    let a = s.len();
    s.push('@');
    s.push_str(parent);
    let found = crate::model::chatters::names(&s[a..]);
    if let [f] = &found[..]
      && f.full == (0..s.len() - a)
      && let Some(p) = res.chatters.person(m.platform, &s[a + f.name.start..], true)
    {
      let login = parent.to_ascii_lowercase().into();
      reply = Some(Mention { range: a..s.len(), login, color: p.color, paint: p.paint, art: None });
    } else {
      spans.push(Span { range: a..s.len(), ink: Ink(theme::DIM), bold: false });
    }
    // then the parent's words, dim, cut to the one line (ext's reply snippet)
    if let Some(t) = m.reply_text.as_deref().filter(|t| !t.trim().is_empty()) {
      let line0 = s.rfind('\n').map_or(0, |i| i + 1);
      let words = one_line(res, &s[line0..], t, (width - MARK_W) as f32 - 2.0 * theme::PAD_X);
      dim(&mut s, &mut spans, &words);
    }
    // the reply line's dim stays readable on a zebra stripe
    for sp in spans.iter_mut().filter(|sp| sp.range.start >= r0 && sp.ink.0 == theme::DIM) {
      sp.ink.0 |= Ink::LIFT;
    }
    s.push('\n');
  }
  if ts {
    dim(&mut s, &mut spans, &clock(m.ts, res.look.twelve));
    s.push(' ');
  }
  let mut ib = Vec::new();
  if tags {
    let (t, c) = match m.platform {
      Platform::Twitch => ("[T]", theme::TWITCH),
      Platform::Kick => ("[K]", theme::GREEN),
      Platform::Youtube => ("[Y]", theme::RED),
    };
    let a = s.len();
    s.push_str(t);
    spans.push(Span { range: a..a + 3, ink: Ink(c), bold: true });
    // ext: 3px after the tag, not a whole monospace cell
    ib.push(gap(s.len(), res.images.px(3)));
  }
  if let Some(x) = x.filter(|x| !x.chan.is_empty() && m.kind != Kind::System) {
    dim(&mut s, &mut spans, &format!("#{} ", x.chan));
  }
  let mut boxes = Vec::new();
  let mut links = Vec::new();
  let mut link_runs = Vec::new();
  let mut painted = None;
  let mut mentions = Vec::new();
  let mut embed = None;
  let mut nick = 0..0;
  let mut vote = 0..0;
  let mut thread = 0..0;
  let mut media = 0..0;
  let mut reveal = 0..0;
  let (mut allow, mut deny) = (0..0, 0..0);
  let mut refs = Vec::new();
  let mut frames = Vec::new();
  let mut pics = Vec::new();
  let mut plays = Vec::new();
  // where the words sit in the text, for search hits
  let mut words_at = 0..0;
  if m.kind == Kind::System {
    // stream lines carry their color (red live, yellow game switch); notices stay dim
    if m.color == 0 {
      dim(&mut s, &mut spans, &m.body);
    } else {
      let a = s.len();
      s.push_str(&m.body);
      spans.push(Span { range: a..s.len(), ink: Ink(m.color), bold: false });
    }
  } else {
    for g in x.map_or(&[][..], |x| &x.head) {
      let a = s.len();
      seg(&mut s, &mut spans, g);
      if g.text.starts_with(">>") {
        thread = a..a + g.text.trim_end().len();
      }
    }
    badges(res, chan, m, &s, &mut ib, &mut boxes);
    let n0 = s.len();
    s.push_str(&m.nick);
    nick = n0..s.len();
    // a painted name's glyphs are drawn from the shaded bitmap instead (paints: names only)
    painted = PaintId::of_message(m).and_then(|uid| {
      let paint = res.cosmetics.paint(&uid)?;
      Some((uid, paint))
    });
    let tag = if painted.is_some() { Ink::PAINTED } else { 0 };
    let name_ink = res.look.ink(m.color);
    spans.push(Span { range: n0..s.len(), ink: Ink(name_ink | tag), bold: true });
    for g in x.map_or(&[][..], |x| &x.tail) {
      seg(&mut s, &mut spans, g);
      if g.text.ends_with('▲') {
        vote = s.len() - '▲'.len_utf8()..s.len();
      }
    }
    if m.kind == Kind::Chat {
      s.push(':');
    }
    s.push(' ');
    // a muted sender's words and pictures are stripped; the name and time stay
    let (pic, fold, preview) = if muted { (None, None, None) } else { picture(res, m, width, &mut plays) };
    let b0 = s.len();
    if !muted {
      (links, link_runs, mentions) = body(res, chan, m, fold.as_ref(), &mut s, &mut ib, &mut boxes);
      // a link's words are drawn in link blue
      for (r, _) in &link_runs {
        spans.push(Span { range: r.clone(), ink: Ink(theme::BLUE), bold: false });
      }
    }
    let ink = match (cleared, m.kind) {
      (true, _) => Some(theme::DIM),
      (_, Kind::Action) => Some(name_ink),
      _ => None,
    };
    if let Some(c) = ink {
      spans.push(Span { range: b0..s.len(), ink: Ink(c), bold: false });
    }
    words_at = b0..s.len();
    refs = post_refs(&s, b0);
    for (r, _) in &refs {
      spans.push(Span { range: r.clone(), ink: Ink(theme::YELLOW), bold: false });
    }
    mentions.splice(0..0, reply);
    // people named in the words wear their color, bold, over the line's ink; a painted one is drawn
    // from its own bitmap. a deleted line stays dim
    if cleared {
      mentions.clear();
    }
    for n in &mut mentions {
      n.color = res.look.ink(n.color);
      n.art = n.paint.as_ref().and_then(|id| res.cosmetics.paint(id));
      let tag = if n.art.is_some() { Ink::PAINTED } else { 0 };
      spans.push(Span { range: n.range.clone(), ink: Ink(n.color | tag), bold: true });
    }
    if let Some(p) = x.and_then(|x| x.pic.as_deref()).filter(|_| !muted) {
      s.push(' ');
      let a = s.len();
      s.push_str(&p.tag);
      spans.push(Span { range: a..s.len(), ink: Ink(theme::CYAN), bold: false });
      media = a..s.len();
      if p.inline {
        post_media(
          res,
          p,
          width,
          &mut s,
          &mut spans,
          &mut ib,
          &mut boxes,
          (&mut reveal, &mut frames, &mut pics, &mut plays),
        );
      }
    }
    // the picture sits on its own line under the text once its size is known
    if let Some(id) = pic {
      embed = Some(id);
      if let Some((w, h)) = res.images.dims(id) {
        s.push('\n');
        ib.push(inline(boxes.len(), s.len(), w, h));
        boxes.push(EmoteBox { img: id, zw: Vec::new(), zw_words: Vec::new(), bg: None, word: (0, 0), blocked: false });
      }
    }
    if let Some(c) = &preview {
      card(res, c, width, &mut s, &mut spans, &mut ib, &mut boxes, &mut frames, (&mut links, &mut link_runs));
    }
    if let Some(h) = x.and_then(|x| x.held.as_deref()) {
      (allow, deny) = held_row(h, &mut s, &mut spans, &mut frames);
    }
  }
  if let Some(f) = find {
    // a painted name is drawn from its own bitmap: leave it be
    let name = if painted.is_some() { 0..0 } else { nick.clone() };
    let words = if m.kind == Kind::System { 0..s.len() } else { words_at.clone() };
    hits(f, &s, &name, words, &mut spans);
  }
  let line = (res.text.line_px * res.look.spacing).round();
  let emoji = (words_at.clone(), res.look.emoji_mul);
  let layout = res.text.layout_at(&s, &spans, &ib, (width - MARK_W) as f32 - 2.0 * theme::PAD_X, line, emoji);
  let mut names = Vec::new();
  if let Some((uid, paint)) = painted
    && let Some((x, w, baseline)) = painted_run(&layout, &nick)
  {
    names.push(PaintedName { uid, nick: m.nick.clone(), color: res.look.ink(m.color), paint, x, w, baseline });
  }
  for n in &mentions {
    if let (Some(uid), Some(paint)) = (&n.paint, &n.art)
      && let Some((x, w, baseline)) = painted_run(&layout, &n.range)
    {
      let text = s.get(n.range.clone()).unwrap_or("").into();
      names.push(PaintedName { uid: uid.clone(), nick: text, color: n.color, paint: paint.clone(), x, w, baseline });
    }
  }
  Laid {
    layout,
    nick,
    vote,
    thread,
    media,
    reveal,
    allow,
    deny,
    refs,
    frames,
    pics,
    plays,
    boxes,
    names,
    mentions,
    embed,
    links,
    link_runs,
  }
}

// `>>4a` in the words from `from` on: a whole word, 1-6 base36 chars, read as the server's padded id
fn post_refs(s: &str, from: usize) -> Vec<(std::ops::Range<usize>, Box<str>)> {
  let mut out = Vec::new();
  let mut at = from;
  while let Some(i) = s[at..].find(">>") {
    let a = at + i;
    let tail = &s[a + 2..];
    let n = tail.bytes().take_while(|b| b.is_ascii_digit() || b.is_ascii_lowercase()).count();
    let end = a + 2 + n;
    let starts = a == from || s[..a].ends_with([' ', '\n']);
    let ends = s[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
    if starts && ends && (1..=6).contains(&n) && out.len() < 8 {
      out.push((a..end, format!("{:0>6}", &tail[..n]).into()));
    }
    at = end.max(a + 2);
  }
  out
}

// an automod hold's last line: allow / deny (after why the last try failed), or where it stands
fn held_row(
  h: &crate::model::held::Row,
  s: &mut String,
  spans: &mut Vec<Span>,
  frames: &mut Vec<(std::ops::Range<usize>, Frame)>,
) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
  use crate::model::held::State;
  let mut text = |s: &mut String, t: &str, ink: u32| {
    let a = s.len();
    s.push_str(t);
    spans.push(Span { range: a..s.len(), ink: Ink(ink), bold: false });
    a..s.len()
  };
  s.push('\n');
  match &h.state {
    State::Busy => {
      text(s, "resolving…", theme::YELLOW);
    }
    State::Ended(e) => {
      text(s, &e.text(), theme::YELLOW);
    }
    State::Open | State::Failed(_) => {
      if let State::Failed(why) = &h.state {
        text(s, why, theme::RED);
        s.push(' ');
      }
      let allow = text(s, " allow ", theme::WHITE);
      s.push(' ');
      let deny = text(s, " deny ", theme::WHITE);
      frames.push((allow.clone(), Frame::Hold));
      frames.push((deny.clone(), Frame::Hold));
      return (allow, deny);
    }
  }
  (0..0, 0..0)
}

type Spots<'a> = (
  &'a mut std::ops::Range<usize>,
  &'a mut Vec<(std::ops::Range<usize>, Frame)>,
  &'a mut Vec<ImgId>,
  &'a mut Vec<(ImgId, Box<str>)>,
);

// an open thread's media under the words: the held box and its button, or the pictures (each on the
// row's width, wrapping) with a hide over them once a held one is shown
#[allow(clippy::too_many_arguments)]
fn post_media(
  res: &mut Res,
  p: &crate::model::message::Pic,
  width: i32,
  s: &mut String,
  spans: &mut Vec<Span>,
  ib: &mut Vec<InlineBox>,
  boxes: &mut Vec<EmoteBox>,
  (reveal, frames, pics, plays): Spots,
) {
  let button = |s: &mut String, spans: &mut Vec<Span>, frames: &mut Vec<_>, b: &str| {
    let a = s.len();
    s.push(' ');
    s.push_str(b);
    s.push(' ');
    spans.push(Span { range: a..s.len(), ink: Ink(theme::WHITE), bold: false });
    frames.push((a..s.len(), Frame::Button));
    a..s.len()
  };
  if let Some(h) = &p.hidden {
    s.push('\n');
    let a = s.len();
    s.push(' ');
    s.push_str(h);
    s.push(' ');
    spans.push(Span { range: a..s.len(), ink: Ink(theme::WHITE), bold: false });
    frames.push((a..s.len(), Frame::Held));
    if let Some(b) = &p.button {
      s.push(' ');
      *reveal = button(s, spans, frames, b);
    }
    return;
  }
  if let Some(b) = &p.button {
    s.push('\n');
    *reveal = button(s, spans, frames, b);
  }
  if !res.media {
    return;
  }
  let avail = ((width - MARK_W) as f32 - 2.0 * theme::PAD_X).max(1.0) as u32;
  let px = if p.items.len() > 1 { MEDIA_MULTI_PX } else { MEDIA_PX };
  let mut first = true;
  for (url, video) in &p.items {
    let id = res.images.id_fit(url, px, avail);
    if res.images.failed(id) {
      continue;
    }
    pics.push(id);
    if let Some(v) = video {
      plays.push((id, v.clone()));
    }
    let Some((w, h)) = res.images.dims(id) else { continue };
    if first {
      s.push('\n');
      first = false;
    } else {
      ib.push(gap(s.len(), res.images.px(3)));
    }
    ib.push(inline(boxes.len(), s.len(), w, h));
    boxes.push(EmoteBox { img: id, zw: Vec::new(), zw_words: Vec::new(), bg: None, word: (0, 0), blocked: false });
  }
}

// a ▶ in the middle of a video's still: white on a black square with a white edge
fn play_mark(c: &mut Canvas, r: Rect, clip: Rect) {
  let side = (r.w.min(r.h) / 3).clamp(12, 48);
  let sq = Rect::new(r.x + (r.w - side) / 2, r.y + (r.h - side) / 2, side, side);
  c.fill(sq.intersect(clip), theme::BLACK);
  c.outline(sq, clip, theme::WHITE);
  // a right-pointing triangle, half the square tall, drawn one row at a time
  let th = side / 2;
  let tw = th * 7 / 8;
  let (x0, y0) = (sq.x + (side - tw) / 2 + 1, sq.y + (side - th) / 2);
  for i in 0..th {
    let len = tw * (th - (2 * i - th + 1).abs()) / th;
    c.fill(Rect::new(x0, y0 + i, len.max(1), 1).intersect(clip), theme::WHITE);
  }
}

// the box around `range` on its first line: (x, top, width, bottom) in layout coordinates
fn range_rect(layout: &TextLayout, range: &std::ops::Range<usize>) -> Option<(f32, f32, f32, f32)> {
  for line in layout.lines() {
    let m = line.metrics();
    let mut found: Option<(f32, f32)> = None;
    for item in line.items() {
      let PositionedLayoutItem::GlyphRun(gr) = item else { continue };
      let at = gr.run().text_range();
      if at.start >= range.end || range.start >= at.end {
        continue;
      }
      // only the clusters inside the range: a run can carry more than it
      let mut x = gr.offset();
      for c in gr.run().visual_clusters() {
        let w = c.advance();
        if range.contains(&c.text_range().start) {
          found = Some(found.map_or((x, x + w), |(a, b)| (a.min(x), b.max(x + w))));
        }
        x += w;
      }
    }
    if let Some((a, b)) = found {
      return Some((a, m.block_min_coord, b - a, m.block_max_coord));
    }
  }
  None
}

// (x, width, baseline) of the painted name at `range`: its glyph runs, which share a line
fn painted_run(layout: &TextLayout, range: &std::ops::Range<usize>) -> Option<(f32, f32, f32)> {
  let mut found: Option<(f32, f32, f32)> = None;
  for line in layout.lines() {
    for item in line.items() {
      let PositionedLayoutItem::GlyphRun(gr) = item else { continue };
      let at = gr.run().text_range();
      if gr.style().brush.0 & Ink::PAINTED == 0 || at.start >= range.end || range.start >= at.end {
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

// a preview's card under a line (rich and audio answers, a video with no still, a held one): who
// asked the resolver, and what the card says
struct Card {
  url: Box<str>,
  p: Arc<Preview>,
  // the box's words when the media is held (`[media hidden — nsfw]`)
  held: Option<Box<str>>,
}

// the line's inline picture: its slot, the link word to fold away once the picture is on screen, and
// a preview's card when the line's link is one
fn picture(
  res: &mut Res,
  m: &Message,
  width: i32,
  plays: &mut Vec<(ImgId, Box<str>)>,
) -> (Option<ImgId>, Option<std::ops::Range<usize>>, Option<Card>) {
  if !res.media || (m.has(flag::CLEARED) && res.look.dim_timeouts) {
    return (None, None, None);
  }
  let avail = ((width - MARK_W) as f32 - 2.0 * theme::PAD_X).max(1.0) as u32;
  let e = match embed::find(&m.body, res.partial_links) {
    None => return (None, None, None),
    Some(embed::Found::Pic(e)) => e,
    Some(embed::Found::Preview(url)) => {
      let Some(p) = res.previews.get(&url) else { return (None, None, None) };
      let held = content::held(p.flagged.iter().map(|c| &**c)).map(|h| format!("[media hidden — {}]", h.label).into());
      // the picture itself, or a video's still that plays it (the file when mpv can take it straight)
      let pic = match p.kind {
        _ if held.is_some() => None,
        PreviewKind::Image => p.media.as_deref().map(|u| (u, None)),
        PreviewKind::Video => p.thumb.as_deref().map(|t| {
          let file = p.media.as_deref().filter(|u| video::file(u));
          (t, Some(file.map_or_else(|| url.clone(), Into::into)))
        }),
        _ => None,
      };
      let Some((u, play)) = pic else { return (None, None, Some(Card { url, p, held })) };
      let id = res.images.id_fit(&embed::proxied(u), EMBED_PX, avail);
      if res.images.failed(id) {
        return (None, None, Some(Card { url, p, held: None }));
      }
      if let Some(v) = play {
        plays.push((id, v));
      }
      return (Some(id), None, None);
    }
  };
  let id = res.images.id_fit(&e.display, EMBED_PX, avail);
  if res.images.failed(id) {
    return (None, None, None);
  }
  let loaded = res.images.dims(id).is_some();
  if let Some(p) = e.play {
    plays.push((id, p));
  }
  (Some(id), e.word.filter(|_| loaded), None)
}

// a preview's card on its own line: the still (small), `[site]` cyan, the title white, the author dim; or
// the held box; or a quoted chat line. the whole of it is one link (a click opens it, or plays it)
#[allow(clippy::too_many_arguments)]
fn card(
  res: &mut Res,
  c: &Card,
  width: i32,
  s: &mut String,
  spans: &mut Vec<Span>,
  ib: &mut Vec<InlineBox>,
  boxes: &mut Vec<EmoteBox>,
  frames: &mut Vec<(std::ops::Range<usize>, Frame)>,
  (links, link_runs): (&mut Vec<Link>, &mut Vec<(std::ops::Range<usize>, usize)>),
) {
  s.push('\n');
  if let Some(h) = &c.held {
    let a = s.len();
    s.push(' ');
    s.push_str(h);
    s.push(' ');
    spans.push(Span { range: a..s.len(), ink: Ink(theme::WHITE), bold: false });
    frames.push((a..s.len(), Frame::Held));
    s.push(' ');
  } else if let Some(t) = &c.p.thumb {
    let avail = ((width - MARK_W) as f32 - 2.0 * theme::PAD_X).max(1.0) as u32;
    let id = res.images.id_fit(&embed::proxied(t), CARD_PX, avail / 3);
    if !res.images.failed(id)
      && let Some((w, h)) = res.images.dims(id)
    {
      ib.push(inline(boxes.len(), s.len(), w, h));
      boxes.push(EmoteBox { img: id, zw: Vec::new(), zw_words: Vec::new(), bg: None, word: (0, 0), blocked: false });
      s.push(' ');
    }
  }
  let a = s.len();
  let mut part = |s: &mut String, t: &str, ink: u32, bold: bool| {
    let b = s.len();
    s.push_str(t);
    spans.push(Span { range: b..s.len(), ink: Ink(ink), bold });
  };
  if c.p.kind == PreviewKind::Line {
    // ext .hs-log-embed: where and when, dim; the speaker in their color; what they said
    let when = if c.p.ts > 0 { format!(" {}", clock(c.p.ts, res.look.twelve)) } else { String::new() };
    part(s, &format!("{}{when} ", c.p.place), theme::DIM, false);
    let ink =
      if c.p.color == 0 { crate::model::color::default_for(&c.p.author.to_ascii_lowercase()) } else { c.p.color };
    part(s, &c.p.author, crate::model::color::readable(ink), true);
    part(s, &format!(": {}", c.p.title), theme::WHITE, false);
  } else {
    part(s, &format!("[{}]", c.p.site), theme::CYAN, false);
    if !c.p.title.is_empty() {
      part(s, &format!(" {}", c.p.title), theme::WHITE, false);
    }
    if !c.p.author.is_empty() {
      part(s, &format!(" · {}", c.p.author), theme::DIM, false);
    }
  }
  links.push(Link { range: 0..0, url: c.url.to_string() });
  link_runs.push((a..s.len(), links.len() - 1));
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

fn badges(res: &mut Res, chan: &str, m: &Message, s: &str, ib: &mut Vec<InlineBox>, boxes: &mut Vec<EmoteBox>) {
  let mut found: Vec<(Box<str>, Option<u32>)> = res
    .badges
    .for_message(chan, m, res.look.ffz_badges, res.look.chatterino_badges)
    .into_iter()
    .map(|b| (b.url.clone(), b.bg))
    .collect();
  // 7tv's badge sits last, right before the name (ext order). seeing a chatter is also what
  // asks heatsync for their cosmetics: batched, cached, answered into the same slot
  if let Some(uid) = PaintId::of_message(m)
    && let Some(b) = res.cosmetics.get(&uid).and_then(|c| c.badge.as_ref())
  {
    found.push((b.url.clone(), b.bg));
  }
  let h = res.images.px(BADGE_PX);
  for (url, bg) in found {
    let img = res.images.id(&url, BADGE_PX);
    let w = res.images.width(img, (18, 18));
    ib.push(inline(boxes.len(), s.len(), w, h));
    boxes.push(EmoteBox { img, zw: Vec::new(), zw_words: Vec::new(), bg, word: (0, 0), blocked: false });
    // ext: 2px between badges (a monospace space is a whole cell)
    ib.push(gap(s.len(), res.images.px(2)));
  }
}

// an empty run of exact width: its id names no box, so drawing and hit-tests pass over it
fn gap(index: usize, w: u32) -> InlineBox {
  InlineBox { id: u64::MAX, ..inline(0, index, w, 1) }
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
) -> Words {
  let links = crate::model::links::find(&m.body, res.partial_links);
  let mut runs = Vec::new();
  let mut people = Vec::new();
  let px = res.look.emote_px();
  let h = res.images.px(px);
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
        let img = res.images.id(&url, px);
        if let Some(last) = boxes.last_mut() {
          last.zw.push(img);
          last.zw_words.push((a as u32, b as u32));
        }
        continue;
      }
      Some((url, dims, _, _)) => {
        if !first {
          s.push(' ');
        }
        let img = res.images.id(&url, px);
        let w = res.images.width(img, dims);
        ib.push(inline(boxes.len(), s.len(), w, h));
        boxes.push(EmoteBox {
          img,
          zw: Vec::new(),
          zw_words: Vec::new(),
          bg: None,
          word: (a as u32, b as u32),
          blocked,
        });
        prev_emote = true;
      }
      None => {
        if !first {
          s.push(' ');
        }
        let p0 = s.len();
        s.push_str(word);
        // the words of a link answer clicks
        let mut linked = false;
        for (k, _) in links.iter().enumerate().filter(|(_, l)| l.range.start < b && a < l.range.end) {
          runs.push((p0..s.len(), k));
          linked = true;
        }
        // names in the words (a link's words are not names)
        if !linked && word.len() >= 3 {
          for f in crate::model::chatters::names(word) {
            let Some(p) = res.chatters.person(m.platform, &word[f.name.clone()], f.at) else { continue };
            people.push(Mention {
              range: p0 + f.full.start..p0 + f.full.end,
              login: word[f.name].to_ascii_lowercase().into(),
              color: p.color,
              paint: p.paint,
              art: None,
            });
          }
        }
        prev_emote = word.is_empty() && prev_emote;
      }
    }
    first = false;
  }
  (links, runs, people)
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

// one layer of an emote, drawn `dst` big (its decoded size in chat), centered in `rect`: zero-width
// overlays center on the base. the hover popup draws its enlarged stack through this same placement
pub fn blit_layer(c: &mut Canvas, rgba: &[u8], src: (u32, u32), dst: (i32, i32), rect: Rect, clip: Rect) {
  let x = rect.x + (rect.w - dst.0) / 2;
  let y = rect.y + (rect.h - dst.1) / 2;
  if dst == (src.0 as i32, src.1 as i32) {
    c.rgba(x, y, dst.0, dst.1, rgba, clip);
  } else {
    c.rgba_to(x, y, (src.0 as i32, src.1 as i32), dst, rgba, clip);
  }
}

fn blit_emote(
  c: &mut Canvas,
  images: &mut Images,
  play: bool,
  imgs: &[ImgId],
  rect: Rect,
  clip: Rect,
  next: &mut Option<u32>,
) {
  for &id in imgs {
    let Some((f, w, h, flip)) = images.frame_with(id, play) else {
      continue;
    };
    blit_layer(c, &f.rgba, (w, h), (w as i32, h as i32), rect, clip);
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
      width: MIN_W,
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
  pub fn hidden_user(&self, who: &str) -> bool {
    self.tab_hidden.contains(&filter::key(who))
  }

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

  // an automod hold's row moved on (asked, settled, lapsed): every row of it, in place
  pub fn set_held(&mut self, id: &str, state: &crate::model::held::State) -> bool {
    let mut hit = false;
    for r in &mut self.rows {
      if let Some(h) = r.msg.x.as_mut().and_then(|x| x.held.as_mut()).filter(|h| &*h.id == id)
        && h.state != *state
      {
        h.state = state.clone();
        Self::touch(r);
        hit = true;
      }
    }
    self.stale |= hit;
    hit
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

  // who spoke here since `since` (ms), newest first, at most `cap`; muted senders left out
  pub fn recent_chatters(&self, since: i64, cap: usize) -> Vec<&str> {
    let mut seen = HashSet::new();
    (self.rows.iter().rev())
      .take_while(|r| r.msg.ts >= since)
      .filter(|r| r.msg.kind != Kind::System && !r.msg.nick.is_empty() && !r.muted())
      .map(|r| &*r.msg.nick)
      .filter(|n| seen.insert(*n))
      .take(cap)
      .collect()
  }

  // a twitch sender's heatsync inventory arrived or changed: re-resolve only their lines
  pub fn touch_sender(&mut self, uid: &str) {
    for r in self.rows.iter_mut().filter(|r| r.msg.platform == Platform::Twitch && &*r.msg.user_id == uid) {
      Self::touch(r);
      self.stale = true;
    }
  }

  // a twitch login's id is known now: their lines that came without one (heatsync's archived
  // history) take it, so they ask for and wear the paint
  pub fn adopt_id(&mut self, login: &str, uid: &str) {
    let bare = |r: &Row| r.msg.platform == Platform::Twitch && r.msg.user_id.is_empty() && &*r.msg.login == login;
    for r in self.rows.iter_mut().filter(|r| bare(r)) {
      r.msg.user_id = uid.into();
      Self::touch(r);
      self.stale = true;
    }
  }

  // their paint or badge arrived or changed: re-lay out only the lines that name them
  pub fn touch_paint(&mut self, id: &PaintId) {
    let wears = |r: &Row| {
      PaintId::of_message(&r.msg).as_ref() == Some(id)
        || r.laid.as_ref().is_some_and(|l| l.mentions.iter().any(|n| n.paint.as_ref() == Some(id)))
    };
    for r in self.rows.iter_mut().filter(|r| wears(r)) {
      Self::touch(r);
      self.stale = true;
    }
  }

  // a lookup learned someone's color or id: lines on screen that name them are laid out again
  pub fn touch_person(&mut self, p: Platform, login: &str) {
    for r in self
      .rows
      .iter_mut()
      .filter(|r| r.msg.platform == p && r.laid.as_ref().is_some_and(|l| l.mentions.iter().any(|n| *n.login == *login)))
    {
      Self::touch(r);
      self.stale = true;
    }
  }

  // stream lines whose name color the frame didn't give take what the app knows of them now:
  // `f` says (color, twitch id) for a login. stream lines are rare; this runs on a lookup or a color
  // frame, never per frame
  pub fn restyle_late(&mut self, mut f: impl FnMut(Platform, &str) -> (u32, Option<u64>)) {
    for r in self.rows.iter_mut().filter(|r| r.msg.x.as_ref().is_some_and(|x| x.late)) {
      let (color, uid) = f(r.msg.platform, &r.msg.login);
      let uid: Box<str> = uid.map(|n| n.to_string().into()).unwrap_or_else(|| r.msg.user_id.clone());
      if r.msg.color != color || r.msg.user_id != uid {
        r.msg.color = color;
        r.msg.user_id = uid;
        Self::touch(r);
        self.stale = true;
      }
    }
  }

  pub fn resize(&mut self, width: i32) {
    let width = width.max(MIN_W);
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
        let over = eb
          .zw_words
          .iter()
          .filter_map(|&(a, b)| {
            let word = m.body.get(a as usize..b as usize)?;
            let (url, _, _, scope) = find_word(res, room(m, &self.channel), m, a as usize, b as usize, word)?;
            Some(Layer { name: word.into(), url, scope })
          })
          .collect();
        return Some(EmoteHit { name: word.into(), url, zero_width, scope, over });
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

  // the id of the line whose ▲ is under (x, y), as drawn last
  pub fn vote_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Box<str>> {
    self.spot_at(area, (x, y), res, |l| &l.vote)
  }

  // the feed post whose held box's button (show / hide / retry) is under (x, y)
  pub fn reveal_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Box<str>> {
    self.spot_at(area, (x, y), res, |l| &l.reveal)
  }

  // the picture under (x, y): a line's inline picture or a feed post's, never an emote or badge; and
  // the video it plays, when it is a still
  pub fn pic_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<(ImgId, Option<Box<str>>)> {
    let (i, top) = self.row_at(area, y, res)?;
    let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
    let laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
    let (lx, ly) = ((x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32);
    let img = laid.layout.lines().flat_map(|l| l.items().collect::<Vec<_>>()).find_map(|it| match it {
      PositionedLayoutItem::InlineBox(b)
        if (b.x..b.x + b.width).contains(&lx) && (b.y..b.y + b.height).contains(&ly) =>
      {
        laid.boxes.get(b.id as usize).map(|eb| eb.img)
      }
      _ => None,
    })?;
    let play = laid.plays.into_iter().find(|p| p.0 == img).map(|p| p.1);
    (laid.embed == Some(img) || laid.pics.contains(&img)).then_some((img, play))
  }

  // the feed post whose `[img]` tag is under (x, y)
  pub fn media_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Box<str>> {
    self.spot_at(area, (x, y), res, |l| &l.media)
  }

  // the feed post whose >>id is under (x, y): a post's own, or one written in a line's words
  pub fn thread_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<Box<str>> {
    if let Some(id) = self.spot_at(area, (x, y), res, |l| &l.thread) {
      return Some(id);
    }
    let (i, top) = self.row_at(area, y, res)?;
    let (m, muted) = (&self.rows[i].msg, self.rows[i].muted());
    let laid = layout_row(res, &self.channel, m, muted, self.width, opts!(self));
    let ranges: Vec<_> = laid.refs.iter().map(|r| r.0.clone()).collect();
    let k = over(&laid.layout, (x - area.x - MARK_W) as f32 - theme::PAD_X, (y - top) as f32, &ranges)?;
    laid.refs.into_iter().nth(k).map(|r| r.1)
  }

  // the row under height `y` and its top, as drawn last
  fn row_at(&mut self, area: Rect, y: i32, res: &mut Res) -> Option<(usize, i32)> {
    let mut bottom = self.edge(area, res);
    for i in (0..self.rows.len()).rev() {
      if bottom <= area.y {
        return None;
      }
      let top = bottom - self.height_at(i, res);
      if y >= top && y < bottom {
        return Some((i, top));
      }
      bottom = top;
    }
    None
  }

  // the line whose `spot` (a range of its laid-out text) is under (x, y)
  fn spot_at(
    &mut self,
    area: Rect,
    at: (i32, i32),
    res: &mut Res,
    spot: fn(&Laid) -> &std::ops::Range<usize>,
  ) -> Option<Box<str>> {
    self.text_at(area, at, res, |m, laid, i| spot(laid).contains(&i).then(|| m.id.clone()))
  }

  // the automod hold whose allow (true) or deny (false) is under (x, y)
  pub fn held_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<(Box<str>, bool)> {
    self.text_at(area, (x, y), res, |m, laid, i| {
      let id = m.x.as_ref()?.held.as_ref()?.id.clone();
      [(true, &laid.allow), (false, &laid.deny)].into_iter().find(|(_, r)| r.contains(&i)).map(|(a, _)| (id, a))
    })
  }

  // ask `f` about the line under (x, y) and where in its laid-out text the pointer is
  fn text_at<T>(
    &mut self,
    area: Rect,
    (x, y): (i32, i32),
    res: &mut Res,
    f: impl FnOnce(&Message, &Laid, usize) -> Option<T>,
  ) -> Option<T> {
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
        let at = Cluster::from_point(&laid.layout, lx, ly)?.0.text_range();
        return f(m, &laid, at.start);
      }
      bottom = top;
    }
    None
  }

  // the person under (x, y): a line's sender, or someone named in its words. only an identity (no
  // clone of the line), so a pointer moving over chat can ask on every move
  pub fn who_at(&mut self, area: Rect, x: i32, y: i32, res: &mut Res) -> Option<WhoHit> {
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
        let mut ranges = Vec::with_capacity(1 + laid.mentions.len());
        ranges.push(laid.nick.clone());
        ranges.extend(laid.mentions.iter().map(|n| n.range.clone()));
        let k = over(&laid.layout, lx, ly, &ranges)?;
        let seq = self.rows[i].seq;
        return Some(match k.checked_sub(1).and_then(|k| laid.mentions.get(k)) {
          Some(n) => WhoHit {
            seq,
            start: n.range.start,
            mention: Some(Mentioned { login: n.login.clone(), color: n.color, paint: n.paint.clone() }),
          },
          None => WhoHit { seq, start: laid.nick.start, mention: None },
        });
      }
      bottom = top;
    }
    None
  }

  #[cfg(test)]
  pub fn last_msg(&self) -> Option<&Message> {
    self.rows.back().map(|r| &r.msg)
  }

  pub fn message(&self, seq: i64) -> Option<&Message> {
    self.index_of(seq).map(|i| &self.rows[i].msg)
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

  // the oldest line is on screen (or every line fits): a log view asks for older ones now
  pub fn at_oldest(&mut self, res: &mut Res, view_h: i32) -> bool {
    let max = self.content_height(res);
    self.scroll + view_h >= max
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
    let (w, zebra) = (self.width, self.zebra && res.look.zebra);
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
        let stripe = zebra && r.seq % 2 == 0;
        let bg = match (r.msg.has(flag::ALERT), stripe) {
          (true, true) => theme::MENTION_STRIPE,
          (true, false) => theme::MENTION_BG,
          (false, true) => theme::ZEBRA,
          (false, false) => theme::BLACK,
        };
        if bg != theme::BLACK {
          c.fill(row.intersect(clip), bg);
        }
        // a mention's signal is its red fill (ext), so it takes no edge of its own
        let mark = if r.msg.has(flag::BRAND) {
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
          c.outline(Rect::new(area.x, top, area.w, r.height), clip, theme::WHITE);
        }
        if let Some(laid) = &r.laid {
          let play = res.look.animate_emotes;
          let Res { text, images, arts, .. } = res;
          if let Some(id) = laid.embed {
            images.want(id);
          }
          for &id in &laid.pics {
            images.want(id);
          }
          let x0 = (area.x + MARK_W) as f32 + theme::PAD_X;
          for (range, f) in &laid.frames {
            let Some((fx, ft, fw, fb)) = range_rect(&laid.layout, range) else { continue };
            let r =
              Rect::new((x0 + fx).round() as i32, top + ft.round() as i32, fw.round() as i32, (fb - ft).round() as i32);
            match f {
              Frame::Held => c.dashed(r, clip, theme::CYAN),
              Frame::Button => c.outline(r, clip, theme::WHITE),
              Frame::Hold => c.outline(r, clip, theme::YELLOW),
            }
          }
          let anims = &mut self.anims;
          let name_anims = &mut self.name_anims;
          let x = (area.x + MARK_W) as f32 + theme::PAD_X;
          text.stripe = stripe;
          let mut stills = Vec::new();
          text.draw(&laid.layout, c, x, top as f32, clip, |c, b| {
            let Some(eb) = laid.boxes.get(b.id as usize) else {
              return;
            };
            let rect =
              Rect::new(b.x.round() as i32, b.y.round() as i32, b.width.round() as i32, b.height.round() as i32);
            if laid.plays.iter().any(|p| p.0 == eb.img) {
              stills.push(rect);
            }
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
            blit_emote(c, images, play, &imgs, rect, clip, &mut flip);
            if flip.is_some() {
              anims.push(Anim { rect: rect.intersect(area), bg, imgs });
            }
          });
          for r in stills {
            play_mark(c, r, clip);
          }
          for n in &laid.names {
            let key = ArtKey {
              uid: n.uid.clone(),
              text: name_art::text_key(&n.nick),
              color: n.color,
              font_px: text.font_px.to_bits(),
              paint: Arc::as_ptr(&n.paint) as usize,
            };
            let w = n.w.ceil() as i32;
            if let Some(fx) = &n.paint.fx {
              // scene + effects: built once, then a premultiplied frame per tick
              let (aw, ah, base, animated, smooth) = {
                let a = arts.anim(&key, &n.nick, |motion| AnimArt::build(text, &n.nick, w, fx, motion, n.color));
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
              let art = arts.get(&key, &n.nick, pic.is_some() || pic_id.is_none(), || {
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
      blit_emote(c, &mut res.images, res.look.animate_emotes, &a.imgs, a.rect, r, &mut next);
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
  use crate::ui::res::Look;

  #[test]
  fn post_refs_are_whole_words() {
    let ids = |s: &str| post_refs(s, 0).into_iter().map(|r| r.1.into_string()).collect::<Vec<_>>();
    assert_eq!(ids(">>3t and >>4a, not a>>5 or >>xyz1234 or >>Ab"), ["00003t", "00004a"]);
    assert_eq!(post_refs("x >>9", 2)[0].0, 2..5);
    assert!(ids(">> >>").is_empty());
  }

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
    assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(42)]);
    let badge = Badge { url: "https://cdn.7tv.app/badge/B/2x.webp".into(), title: "t".into(), bg: None };
    res.cosmetics.put(&crate::cosmetics::id::PaintId::Twitch(42), Some(Cosmetic { paint: None, badge: Some(badge) }));
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
    assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(42)]);
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
    res
      .cosmetics
      .put(&crate::cosmetics::id::PaintId::Twitch(42), Some(Cosmetic { paint: Some(Arc::new(paint)), badge: None }));
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
      assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(37005319)]);
      res.cosmetics.put(
        &crate::cosmetics::id::PaintId::Twitch(37005319),
        Some(Cosmetic { paint: Some(Arc::new(paint)), badge: None }),
      );
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
      res.cosmetics.put(
        &crate::cosmetics::id::PaintId::Twitch(37005319),
        Some(Cosmetic { paint: Some(paint.clone()), badge: None }),
      );
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

  // a feed post: its author's twitch id asks for their paint, the paint animates like in chat, and
  // only the ▲ (not the heat or the replies beside it) answers a vote click
  // archived history comes without ids: once one is known those lines take it and ask for the paint,
  // but never someone else's lines or another platform's
  #[test]
  fn lines_without_an_id_adopt_it_and_ask_for_the_paint() {
    let line = |p: Platform, login: &str, uid: &str| {
      let mut m = Message::system(0, p, "hi");
      m.kind = Kind::Chat;
      m.login = login.into();
      m.nick = login.into();
      m.user_id = uid.into();
      m
    };
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(10, "x".into());
    v.resize(500);
    v.push(line(Platform::Twitch, "ennortix", ""));
    v.push(line(Platform::Twitch, "alice", ""));
    v.push(line(Platform::Kick, "ennortix", ""));
    let area = Rect::new(0, 0, 500, 200);
    let mut c = Canvas::new(500, 200);
    v.draw(&mut c, &mut res, area, area);
    assert!(res.cosmetics.take_wanted().is_empty(), "no ids, nothing to ask");
    v.adopt_id("ennortix", "37005319");
    v.draw(&mut c, &mut res, area, area);
    assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(37005319)]);
    let ids: Vec<&str> = v.rows.iter().map(|r| &*r.msg.user_id).collect();
    assert_eq!(ids, vec!["37005319", "", ""]);
  }

  #[test]
  fn feed_rows_wear_animated_paints_and_the_arrow_alone_votes() {
    use crate::cosmetics::{Cosmetic, paint::Paint as P};
    let spec = serde_json::json!({"v": 2, "base": {"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]},
      "glow": null, "scene": null, "effects": [{"id": "twirl", "speed": 1}]});
    let paint = Arc::new(P::from_hs(&spec).unwrap());
    let post = crate::model::post::parse(&serde_json::json!({"base36_id": "00abc1", "username": "ennortix",
      "user_id": "37005319", "platform": "twitch", "content": "gg", "heat": 20, "reply_count": 2}))
    .unwrap();
    let mut res = Res::new(1.0);
    res.look.timestamps = true;
    let mut v = ChatView::new(10, "feed".into());
    v.top = true;
    v.zebra = false;
    v.resize(500);
    v.push(crate::model::post::message(&post, Some("me"), true));
    let area = Rect::new(0, 0, 500, 40);
    let mut c = Canvas::new(500, 40);
    v.draw(&mut c, &mut res, area, area);
    assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(37005319)]);
    res
      .cosmetics
      .put(&crate::cosmetics::id::PaintId::Twitch(37005319), Some(Cosmetic { paint: Some(paint), badge: None }));
    v.touch_sender("37005319");
    v.draw(&mut c, &mut res, area, area);
    v.draw(&mut c, &mut res, area, area);
    assert_eq!(v.next_anim, Some(TICK_SMOOTH_MS), "a painted feed name ticks while on screen");
    let hits: Vec<i32> = (0..500).filter(|&x| v.vote_at(area, x, 8, &mut res).is_some()).collect();
    assert!(!hits.is_empty(), "the arrow is hittable");
    assert!(hits.windows(2).all(|w| w[1] == w[0] + 1) && hits.len() < 30, "only the arrow: {hits:?}");
    assert_eq!(v.vote_at(area, hits[0], 8, &mut res).as_deref(), Some("00abc1"));
    assert!(v.vote_at(area, 2, 8, &mut res).is_none());
    // the >>id alone opens the thread: a short run at the line's start, not the name or the arrow
    let ids: Vec<i32> = (0..500).filter(|&x| v.thread_at(area, x, 8, &mut res).is_some()).collect();
    assert!(!ids.is_empty() && ids.windows(2).all(|w| w[1] == w[0] + 1) && ids.len() < 60, "only >>abc1: {ids:?}");
    assert_eq!(v.thread_at(area, ids[0], 8, &mut res).as_deref(), Some("00abc1"));
    assert!(!ids.contains(&hits[0]));
    // logged out: no arrow, nothing to hit
    let mut v = ChatView::new(10, "feed".into());
    v.top = true;
    v.resize(500);
    v.push(crate::model::post::message(&post, Some("me"), false));
    assert!((0..500).all(|x| v.vote_at(area, x, 8, &mut res).is_none()));
  }

  // the same number from twitch, kick and youtube asks for three different people's paints,
  // and a paint arriving for one dresses only that one's lines
  #[test]
  fn chat_lines_ask_for_paints_in_their_own_namespace() {
    use crate::cosmetics::{Cosmetic, id::PaintId, paint::Paint as P};
    let spec = serde_json::json!({"v": 2, "base": {"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]}, "glow": null, "scene": null, "effects": []});
    let paint = Arc::new(P::from_hs(&spec).unwrap());
    let yt = "UC0123456789abcdefghij_-";
    let line = |p: Platform, id: &str| {
      let mut m = said("bob", "hi");
      m.platform = p;
      m.user_id = id.into();
      m
    };
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.resize(500);
    for m in
      [line(Platform::Twitch, "5"), line(Platform::Kick, "5"), line(Platform::Youtube, yt), line(Platform::Kick, "")]
    {
      v.push(m);
    }
    let area = Rect::new(0, 0, 500, 120);
    let mut c = Canvas::new(500, 120);
    v.draw(&mut c, &mut res, area, area);
    let mut asked = res.cosmetics.take_wanted();
    asked.sort_by_key(PaintId::wire);
    assert_eq!(asked, vec![PaintId::Twitch(5), PaintId::Kick(5), PaintId::Youtube(yt.into())]);
    // kick 5 wears a paint: twitch 5's line stays plain
    res.cosmetics.put(&PaintId::Kick(5), Some(Cosmetic { paint: Some(paint), badge: None }));
    assert!(res.cosmetics.get(&PaintId::Twitch(5)).is_none());
    assert!(res.cosmetics.get(&PaintId::Kick(5)).is_some());
  }

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
  fn a_hit_on_a_stacked_emote_yields_every_layer() {
    use crate::emotes::{Emote, Provider};
    let mut res = Res::new(1.0);
    let (base, ice, spark) = (
      "https://cdn.7tv.app/emote/a/1x.webp",
      "https://cdn.7tv.app/emote/b/1x.webp",
      "https://cdn.7tv.app/emote/c/1x.webp",
    );
    let e = |url: &str, zero_width| Emote { url: url.into(), w: 28, h: 28, zero_width };
    res.emotes.set(
      None,
      Provider::SevenTv,
      [("Kappa".into(), e(base, false)), ("frozen0".into(), e(ice, true)), ("spark".into(), e(spark, true))].into(),
    );
    let mut v = ChatView::new(10, "c".into());
    v.resize(400);
    let mut m = Fake::new(1).message(1);
    m.kind = Kind::Chat;
    m.emotes = Box::new([]);
    m.body = "hi Kappa frozen0 spark yo".into();
    v.push(m);
    let area = Rect::new(0, 0, 400, 200);
    let hit = (0..400)
      .step_by(2)
      .find_map(|x| (150..200).find_map(|y| v.emote_at(area, x, y, &mut res)))
      .expect("the stack under the pointer");
    assert_eq!((&*hit.name, &*hit.url), ("Kappa", base));
    let over: Vec<(&str, &str)> = hit.over.iter().map(|l| (&*l.name, &*l.url)).collect();
    assert_eq!(over, [("frozen0", ice), ("spark", spark)]);
    // a blocked overlay is not drawn, so it is not listed
    res.blocked.block("frozen0", ice, Scope::Channel);
    v.relayout();
    let hit = (0..400).step_by(2).find_map(|x| (150..200).find_map(|y| v.emote_at(area, x, y, &mut res))).unwrap();
    assert_eq!(hit.over.len(), 1);
  }

  #[test]
  fn names_in_words_wear_their_color_and_answer_the_pointer() {
    let mut res = Res::new(1.0);
    let mut alice = said("alice", "x");
    alice.color = theme::GREEN;
    res.chatters.heard(&alice);
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.resize(700);
    let mut m = said("bob", "hi alice, welcome @carol you what https://alice.example/x");
    m.color = theme::MAGENTA;
    v.push(m);
    let area = Rect::new(0, 0, 700, 200);
    let mut c = Canvas::new(700, 200);
    v.draw(&mut c, &mut res, area, area);
    let who = |v: &mut ChatView, res: &mut Res| {
      let mut seen: Vec<(Option<Box<str>>, u32)> = Vec::new();
      for x in (0..700).step_by(2) {
        if let Some(h) = (150..200).find_map(|y| v.who_at(area, x, y, res)) {
          let k = h.mention.map(|n| (Some(n.login), n.color)).unwrap_or((None, theme::MAGENTA));
          if !seen.contains(&k) {
            seen.push(k);
          }
        }
      }
      seen
    };
    // the sender, a spoken name (bare, with its comma left out), an unknown @name (white until looked up);
    // plain words, the everyday "you"/"what" and the link's words are none of them
    let got = who(&mut v, &mut res);
    assert_eq!(
      got,
      vec![(None, theme::MAGENTA), (Some("alice".into()), theme::GREEN), (Some("carol".into()), theme::WHITE)]
    );
    let count = |c: &Canvas, ink| c.px.iter().filter(|&&p| p == ink).count();
    assert!(count(&c, theme::GREEN) > 20, "alice is drawn green");
    assert_eq!(res.chatters.take_wanted(), vec![(Platform::Twitch, "carol".into())]);
    // the answer arrives: carol's lines are laid out again in her color
    res.chatters.learned(Platform::Twitch, "carol", theme::CYAN, None);
    v.touch_person(Platform::Twitch, "carol");
    v.draw(&mut c, &mut res, area, area);
    assert!(who(&mut v, &mut res).contains(&(Some("carol".into()), theme::CYAN)));
    assert!(count(&c, theme::CYAN) > 20);
    // a deleted line stays dim: no colors, nothing to hover
    v.clear_msg("");
    let mut gone = said("bob", "hi alice");
    gone.id = "g".into();
    gone.flags |= flag::CLEARED;
    let mut w = ChatView::new(10, "c".into());
    w.resize(700);
    w.push(gone);
    assert!(
      (0..700)
        .step_by(2)
        .all(|x| (150..200).all(|y| w.who_at(area, x, y, &mut res).is_none_or(|h| h.mention.is_none())))
    );
  }

  #[test]
  fn a_reply_shows_its_parents_words_on_one_line() {
    let mut res = Res::new(1.0);
    let mut m = said("bob", "lol");
    m.reply_to = Some("wollip".into());
    m.reply_text = Some("short one".into());
    let text = |l: &Laid| l.layout.lines().map(|ln| ln.text_range()).collect::<Vec<_>>();
    let l = layout_row(&mut res, "c", &m, false, 400, NO_OPTS);
    assert_eq!(text(&l).len(), 2);
    // long words (and a newline from a tag) are cut with …, never a third line
    m.reply_text = Some(format!("{}\nline two", "words ".repeat(60)).into());
    for w in [140, 400, 900] {
      let l = layout_row(&mut res, "c", &m, false, w, NO_OPTS);
      assert_eq!(text(&l).len(), 2, "at {w}px");
    }
    let l = layout_row(&mut res, "c", &m, false, 400, NO_OPTS);
    assert!(l.layout.lines().next().is_some_and(|ln| ln.text_range().len() > 20), "the line is filled, not emptied");
  }

  #[test]
  fn a_reply_headers_name_is_a_mention() {
    let mut res = Res::new(1.0);
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.resize(700);
    let mut m = said("bob", "lol");
    m.reply_to = Some("NL_Kripp".into());
    v.push(m);
    let area = Rect::new(0, 0, 700, 200);
    let mut c = Canvas::new(700, 200);
    v.draw(&mut c, &mut res, area, area);
    let ids = |v: &mut ChatView, res: &mut Res| {
      let mut seen: Vec<(Box<str>, u32)> = Vec::new();
      for x in (0..700).step_by(2) {
        for y in 0..200 {
          if let Some(n) = v.who_at(area, x, y, res).and_then(|h| h.mention) {
            let k = (n.login, n.color);
            if !seen.contains(&k) {
              seen.push(k);
            }
          }
        }
      }
      seen
    };
    // unknown: white, and a lookup is asked for
    assert_eq!(ids(&mut v, &mut res), vec![("nl_kripp".into(), theme::WHITE)]);
    assert_eq!(res.chatters.take_wanted(), vec![(Platform::Twitch, "nl_kripp".into())]);
    res.chatters.learned(Platform::Twitch, "nl_kripp", theme::CYAN, None);
    v.touch_person(Platform::Twitch, "nl_kripp");
    v.draw(&mut c, &mut res, area, area);
    assert_eq!(ids(&mut v, &mut res), vec![("nl_kripp".into(), theme::CYAN)]);
  }

  #[test]
  fn a_channel_owner_answers_the_pointer_by_bare_name_without_speaking() {
    let mut res = Res::new(1.0);
    res.chatters.host(Platform::Twitch, "nl_kripp");
    assert_eq!(res.chatters.take_wanted(), vec![(Platform::Twitch, "nl_kripp".into())]);
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.resize(700);
    v.push(said("bob", "gg nl_kripp"));
    let area = Rect::new(0, 0, 700, 200);
    let mut c = Canvas::new(700, 200);
    v.draw(&mut c, &mut res, area, area);
    let hit = |v: &mut ChatView, res: &mut Res| {
      (0..700)
        .step_by(2)
        .find_map(|x| (0..200).find_map(|y| v.who_at(area, x, y, res)?.mention))
        .map(|n| (n.login, n.color))
    };
    assert_eq!(hit(&mut v, &mut res), Some(("nl_kripp".into(), theme::WHITE)));
    // their color lands from the lookup
    res.chatters.learned(Platform::Twitch, "nl_kripp", theme::CYAN, None);
    v.touch_person(Platform::Twitch, "nl_kripp");
    v.draw(&mut c, &mut res, area, area);
    assert_eq!(hit(&mut v, &mut res), Some(("nl_kripp".into(), theme::CYAN)));
  }

  #[test]
  fn a_painted_mention_is_drawn_from_its_bitmap() {
    use crate::cosmetics::{Cosmetic, paint::Paint as P};
    let mut res = Res::new(1.0);
    let paint = P::parse(&serde_json::json!({"id": "G", "function": "linear-gradient", "angle": 90,
      "stops": [{"at": 0, "color": 0xff0000ffu32 as i32}, {"at": 1, "color": 0x0000ffffu32 as i32}]}))
    .unwrap();
    let mut alice = said("alice", "x");
    alice.user_id = "42".into();
    alice.color = theme::GREEN;
    res.chatters.heard(&alice);
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.resize(400);
    v.push(said("bob", "wwwwwwwwwwww alice"));
    let area = Rect::new(0, 0, 400, 40);
    let mut c = Canvas::new(400, 40);
    v.draw(&mut c, &mut res, area, area);
    // their cosmetics are asked for, and alice is plain green meanwhile
    assert_eq!(res.cosmetics.take_wanted(), vec![PaintId::Twitch(42)]);
    let green = |c: &Canvas| c.px.iter().filter(|&&p| p == theme::GREEN).count();
    assert!(green(&c) > 20);
    res.cosmetics.put(&PaintId::Twitch(42), Some(Cosmetic { paint: Some(Arc::new(paint)), badge: None }));
    v.touch_paint(&PaintId::Twitch(42));
    v.draw(&mut c, &mut res, area, area);
    assert_eq!(green(&c), 0, "the name now runs red to blue");
    let (red, blue) = c.px.iter().fold((0, 0), |(r, b), p| {
      let (pr, pg, pb) = (p >> 16 & 255, p >> 8 & 255, p & 255);
      (r + (pr > 200 && pg < 60 && pb < 100) as u32, b + (pb > 200 && pr < 100 && pg < 60) as u32)
    });
    assert!(red > 5 && blue > 5, "{red} {blue}");
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

  // the chat look: the clock before a line, its format, platform tags, line spacing, emote size
  #[test]
  fn the_look_changes_what_a_row_is_made_of() {
    let mut res = Res::new(1.0);
    let mut m = plain("a", "hi");
    m.ts = 1_700_000_000_000;
    let at = |res: &mut Res, o: Opts| layout_row(res, "c", &m, false, 600, o).nick.start;
    let on = Opts { ts: true, tags: true, find: None };
    // off (the ext's default): no time column at all
    assert_eq!(at(&mut res, on), 3, "just the tag: the clock is off");
    res.look.tags = false;
    assert_eq!(at(&mut res, on), 0, "no tags, no clock: the name leads");
    res.look.timestamps = true;
    assert_eq!(at(&mut res, on), "00:00 ".len(), "24h");
    assert_eq!(at(&mut res, Opts { ts: false, ..on }), 0, "a view that never shows time still doesn't");
    res.look.twelve = true;
    let n = at(&mut res, on);
    assert!(n == "4:05pm ".len() || n == "12:05pm ".len(), "12h with am/pm, no leading zero: {n}");
    // line spacing: 22 and 26 are +10% and +30% of the 20px line
    let h = |res: &mut Res| layout_row(res, "c", &m, false, 600, NO_OPTS).layout.height().round() as i32;
    let tight = h(&mut res);
    res.look.spacing = 1.3;
    assert_eq!((tight, h(&mut res)), (20, 26));
    // emote size multiplies the base box; badges and the rest keep theirs
    assert_eq!([1u16, 2, 4].map(|k| Look { emote_mul: k, ..res.look }.emote_px()), [28, 56, 112]);
  }

  // readable names off keeps the platform's raw color; dim timeouts off leaves a timed-out line as it was
  #[test]
  fn the_look_picks_name_colors_and_dimming() {
    let look = Look::default();
    assert_ne!(look.ink(0x0000ff), 0x0000ff, "readable lifts dark blue");
    assert_eq!(Look { readable: false, ..look }.ink(0x0000ff), 0x0000ff, "off: as sent");
    let mut res = Res::new(1.0);
    res.chatters.learned(Platform::Twitch, "carol", theme::CYAN, None);
    let mut m = said("bob", "hi @carol");
    m.flags |= flag::CLEARED;
    let seen = |res: &mut Res| layout_row(res, "c", &m, false, 600, NO_OPTS).mentions.len();
    assert_eq!(seen(&mut res), 0, "a timed-out line is dim: no colored names");
    res.look.dim_timeouts = false;
    assert_eq!(seen(&mut res), 1, "off: the line reads as normal");
  }

  // animate emotes never: an animated picture holds its first frame and asks for no ticks
  #[test]
  fn animate_never_holds_the_first_frame() {
    use crate::images::decode::{Decoded, Frame};
    let mut res = Res::new(1.0);
    let id = res.images.id("https://x/a.gif", 28);
    assert!(res.images.frame(id).is_none());
    let (_, _, want) = res.images.take_wanted().remove(0);
    let frame = |v: u8| Frame { rgba: vec![v; 4].into(), delay_ms: 50 };
    res.images.loaded(id, want, Ok(Decoded { w: 1, h: 1, frames: vec![frame(1), frame(2)] }));
    assert!(res.images.frame_with(id, true).is_some_and(|f| f.3.is_some()), "plays: a flip is due");
    let still = res.images.frame_with(id, false).map(|f| (f.0.rgba[0], f.3));
    assert_eq!(still, Some((1, None)), "never: frame one, no ticks");
  }

  #[test]
  fn alert_lines_get_the_mention_fill_and_copies_keep_their_room() {
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
    // newest line sits at the bottom, on the dark red fill; the one above it stays black
    assert_eq!(edge(70), theme::MENTION_BG);
    assert_eq!(edge(50), theme::BLACK);
    // on a zebra row a mention takes the lighter red, never the gray stripe
    v.zebra = true;
    v.draw(&mut c, &mut res, area, area);
    let (ping, plain_row) = (c.px[70 * 400 + 200], c.px[50 * 400 + 200]);
    assert!([theme::MENTION_BG, theme::MENTION_STRIPE].contains(&ping), "{ping:06x}");
    assert!([theme::BLACK, theme::ZEBRA].contains(&plain_row), "{plain_row:06x}");
    assert_ne!(ping == theme::MENTION_STRIPE, plain_row == theme::ZEBRA, "stripes alternate");
    // a copied line draws a room tag and head/tail segs around the name
    let mut x = plain("c", "hi");
    x.x = Some(Box::new(Extra {
      chan: "forsen".into(),
      head: Box::new([Seg { text: ">>1 ".into(), ink: theme::YELLOW, bold: true }]),
      tail: Box::new([Seg { text: " 5".into(), ink: theme::WHITE, bold: false }]),
      edge: 0,
      paint: None,
      pic: None,
      late: false,
      held: None,
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
  fn a_view_made_after_startup_lays_a_system_line_on_one_row() {
    let mut res = Res::new(1.0);
    let sys = || Message::system(1, Platform::Kick, "not sent: log in to send (ctrl-l)");
    let mut v = ChatView::new(10, "dellor".into());
    v.push(sys());
    v.resize(600);
    let one = v.height_at(0, &mut res);
    let line = layout_row(&mut res, "dellor", &plain("c", "x"), false, 600, NO_OPTS).layout.height().ceil() as i32;
    assert_eq!(one, line, "one row at a normal width");
    // a transient 0 clamps: a few rows at worst, never a word per line (9 words)
    let mut w = ChatView::new(10, "dellor".into());
    w.push(sys());
    w.resize(0);
    assert!(w.height_at(0, &mut res) <= 4 * line);
    w.resize(600);
    assert_eq!(w.height_at(0, &mut res), one);
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
