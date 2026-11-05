// virtualized chat rows: heights for every row, layouts only for rows on screen. this file is the view's
// state and its public api; layout.rs turns a line into a row, draw.rs paints rows and the repaints of
// what animates, hit.rs answers what is under the pointer
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use super::canvas::Rect;
use super::name_art::ArtKey;
use super::res::Res;
use super::text::TextLayout;
use crate::cosmetics::id::PaintId;
use crate::cosmetics::paint::Paint as NamePaint;
use crate::emotes::block::Scope;
use crate::images::ImgId;
use crate::model::filter;
use crate::model::find::Find;
use crate::model::links::Link;
use crate::model::message::{Kind, Message, Platform, flag};
use crate::model::seen::Seen;
use crate::util::time::now_ms;

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
  // a name's picture: what stands in until it is drawn (or when there is none)
  stand_in: Option<(char, u32)>,
  // an emoji under the images (an overlay emote sent right after it, like the ext): drawn first
  glyph: Option<Box<str>>,
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
  // a reply's `↳ @name words` line (the chain it opens)
  hdr: std::ops::Range<usize>,
  // a feed post's media: its `[img]` tag, the held box's button, and the frames drawn around them
  media: std::ops::Range<usize>,
  reveal: std::ops::Range<usize>,
  // an automod hold's allow and deny buttons
  allow: std::ops::Range<usize>,
  deny: std::ops::Range<usize>,
  // `>>id` written in the words: where, and the post (server id) it names
  refs: Vec<(std::ops::Range<usize>, Box<str>)>,
  // `#tag` in the words: where, and the tag
  tags: Vec<(std::ops::Range<usize>, Box<str>)>,
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
  // a cheer's `N bits`: 1px twitch color
  Bits,
  // an announcement's chip: 1px yellow (ext .hs-mc-notice-announce::before)
  Announce,
  // a badge as a word: 1px in its own color
  Chip(u32),
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
  glyph: Option<Box<str>>,
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

pub struct ChatView {
  pub channel: Box<str>,
  rows: VecDeque<Row>,
  cap: usize,
  seq_back: i64,
  pub width: i32,
  // px scrolled up from the bottom; 0 = pinned to newest
  scroll: i32,
  // rows appended since the last draw, for the scroll-blit fast path
  pending: usize,
  // lines that arrived (and showed) while scrolled up; the jump bar's count, zero once back at the bottom
  unread: usize,
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
    $crate::ui::chat_view::Opts { ts: $v.timestamps, tags: $v.tags, find: $v.search.as_ref() }
  };
}

mod draw;
mod hit;
mod layout;
mod marks;
#[cfg(test)]
mod tests;

pub use draw::{blit_layer, emoji_glyph};
use layout::layout_row;

impl ChatView {
  pub fn new(cap: usize, channel: Box<str>) -> Self {
    Self {
      channel,
      rows: VecDeque::with_capacity(cap.min(1024)),
      cap,
      seq_back: 0,
      width: MIN_W,
      scroll: 0,
      pending: 0,
      unread: 0,
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

  // an empty view that reads like this one (room, look, filters, hidden people): a reply chain over the tab
  pub fn sibling(&self, cap: usize) -> Self {
    let mut v = Self::new(cap, self.channel.clone());
    v.glow = false;
    (v.zebra, v.timestamps, v.tags, v.filtered) = (self.zebra, self.timestamps, self.tags, self.filtered);
    v.tab_hidden = self.tab_hidden.clone();
    v.resize(self.width);
    v
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

  // the words of every line, oldest first (probes)
  #[cfg(test)]
  pub fn bodies(&self) -> Vec<String> {
    self.rows.iter().map(|r| r.msg.body.to_string()).collect()
  }

  pub fn push(&mut self, mut msg: Message) {
    // seen is marked even with the glow off, so toggling it on doesn't light up regulars
    if msg.kind.said() && msg.system.is_none() {
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

  // when the oldest line here was said
  pub fn oldest_ts(&self) -> Option<i64> {
    self.rows.front().map(|r| r.msg.ts)
  }

  // history arrives after live lines may already be showing, and a merged tab gets one batch per platform:
  // each line goes where its time puts it, so twitch, kick and youtube history interleave instead of
  // stacking in arrival order. a line we already have (same id) is skipped
  pub fn prepend(&mut self, history: Vec<Message>) {
    let have: HashSet<&str> = self.rows.iter().map(|r| &*r.msg.id).filter(|id| !id.is_empty()).collect();
    let fresh: Vec<Message> = history.into_iter().filter(|m| m.id.is_empty() || !have.contains(&*m.id)).collect();
    self.slot_in(fresh);
  }

  // lines that landed while the feed was down, fetched once it was back: each goes where its time puts it
  // among the lines already here (new ones may have come in since), and one we already have (same id) is
  // skipped. an older line than the oldest shown is history's job, not a gap's. returns how many went in
  pub fn fill_gap(&mut self, msgs: Vec<Message>) -> usize {
    let mut have: HashSet<Box<str>> = self.rows.iter().map(|r| r.msg.id.clone()).filter(|id| !id.is_empty()).collect();
    let oldest = self.rows.front().map(|r| r.msg.ts);
    let fresh: Vec<Message> = msgs
      .into_iter()
      .filter(|m| !m.id.is_empty() && oldest.is_none_or(|t| m.ts >= t) && have.insert(m.id.clone()))
      .map(|mut m| {
        m.flags |= flag::HISTORY;
        m
      })
      .collect();
    let n = fresh.len();
    self.slot_in(fresh);
    n
  }

  // merge lines from earlier in time into the rows by timestamp (one pass; a line goes in front of the first
  // row newer than it). seq stays one unbroken run (index_of counts on it): when every new line is older
  // than every row (plain history) the rows keep their numbers, else the ones after a slotted-in line move
  // up. over the cap, the oldest go
  fn slot_in(&mut self, mut fresh: Vec<Message>) {
    if fresh.is_empty() {
      return;
    }
    fresh.sort_by_key(|m| m.ts);
    let base = self.rows.front().map_or(self.seq_back + 1, |r| r.seq) - fresh.len() as i64;
    let picked = self.selected;
    let old = std::mem::take(&mut self.rows);
    let mut rows = VecDeque::with_capacity(old.len() + fresh.len());
    let mut fresh = fresh.into_iter().peekable();
    let mut moved = None;
    for r in old {
      while let Some(m) = fresh.next_if(|m| m.ts < r.msg.ts) {
        let miss = self.missed(&m);
        rows.push_back(Row { seq: 0, msg: m, height: 0, laid: None, look: 0, miss });
      }
      if picked == Some(r.seq) {
        moved = Some(rows.len());
      }
      rows.push_back(r);
    }
    for m in fresh {
      let miss = self.missed(&m);
      rows.push_back(Row { seq: 0, msg: m, height: 0, laid: None, look: 0, miss });
    }
    let drop = rows.len().saturating_sub(self.cap);
    rows.drain(..drop);
    for (i, r) in rows.iter_mut().enumerate() {
      r.seq = base + (drop + i) as i64;
    }
    self.seq_back = base + (drop + rows.len()) as i64 - 1;
    self.selected = moved.and_then(|p| p.checked_sub(drop)).map(|p| base + (drop + p) as i64);
    self.rows = rows;
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
    self.rows.iter().filter(|r| !r.hidden() && r.msg.kind.said()).count()
  }

  // n / N: the next or previous match, wrapping; (place, of) for the status line. starts at the oldest (n)
  // or newest (N) like the ext
  pub fn search_step(&mut self, dir: i32, res: &mut Res, view_h: i32) -> Option<(usize, usize)> {
    let hits: Vec<usize> =
      (0..self.rows.len()).filter(|&i| !self.rows[i].hidden() && self.rows[i].msg.kind.said()).collect();
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
    self.unread = 0;
    self.seq_back = 0;
    self.stale = true;
  }

  pub fn clear_msg(&mut self, id: &str) {
    if let Some(r) = self.rows.iter_mut().rev().find(|r| &*r.msg.id == id) {
      r.msg.flags |= flag::CLEARED;
      Self::touch(r);
      self.stale = true;
    }
  }

  // tag the recent lines `f` picks as sent through heatsync (an echo that beat the site's word)
  pub fn mark_origin(&mut self, f: impl Fn(&Message) -> bool) -> bool {
    let mut any = false;
    for r in self.rows.iter_mut().rev().take(60).filter(|r| !r.msg.x.as_ref().is_some_and(|x| x.origin) && f(&r.msg)) {
      r.msg.extra().origin = true;
      Self::touch(r);
      any = true;
    }
    self.stale |= any;
    any
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
      .filter(|r| r.msg.kind.said() && !r.msg.nick.is_empty())
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
      .filter(|r| r.msg.kind.said() && !r.msg.nick.is_empty() && !r.muted())
      .map(|r| &*r.msg.nick)
      .filter(|n| seen.insert(*n))
      .take(cap)
      .collect()
  }

  // heatsync's refs for a twitch line landed after it: added to that line, which alone lays out again.
  // false = no such line here
  pub fn merge_hs(&mut self, id: &str, refs: &[crate::model::message::HsEmote]) -> bool {
    let mut hit = false;
    for r in self.rows.iter_mut().filter(|r| r.msg.platform == Platform::Twitch && &*r.msg.id == id) {
      hit = true;
      if crate::net::relay::merge_hs(&mut r.msg.hs, refs) {
        Self::touch(r);
        self.stale = true;
      }
    }
    hit
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

  // something drawn before this chatter's name arrived (their picture, their heat bar): re-lay out their lines
  pub fn touch_chatter(&mut self, p: Platform, login: &str) {
    for r in self.rows.iter_mut().filter(|r| r.msg.platform == p && *r.msg.login == *login) {
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

  // the jump-to-bottom bar (ext #hs-mc-new-msgs): Some(lines that arrived below) while scrolled up in a
  // chat that follows its newest line. a list read from the top never shows it
  pub fn jump(&self) -> Option<usize> {
    (!self.top && self.scroll > 0).then_some(self.unread)
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
      if r.msg.kind.said() && !r.hidden() {
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

  #[cfg(test)]
  pub fn last_msg(&self) -> Option<&Message> {
    self.rows.back().map(|r| &r.msg)
  }

  // every line held, oldest first
  pub fn msgs(&self) -> impl DoubleEndedIterator<Item = &Message> {
    self.rows.iter().map(|r| &r.msg)
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
      .filter(|r| &*r.msg.login == login && r.msg.kind.said() && !r.msg.body.is_empty())
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
    if self.pinned() {
      self.unread = 0;
    } else {
      // keep the reader's place: content grew under them
      self.scroll += shift;
      self.unread += n;
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
}
