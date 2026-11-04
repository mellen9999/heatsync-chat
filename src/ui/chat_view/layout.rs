// a line becomes a row: its text, spans, emote boxes and the links, mentions and paints inside it

use super::{EmoteBox, Frame, Laid, MARK_W, Mention, Opts, PaintedName, Words, room};
use crate::cosmetics::id::PaintId;
use crate::emotes::resolve::find_word;
use crate::heatsync::content;
use crate::images::ImgId;
use crate::model::find::Find;
use crate::model::links::Link;
use crate::model::message::{Kind, Message, Notice, Platform, Seg, flag};
use crate::model::preview::{Kind as PreviewKind, Preview};
use crate::model::video;
use crate::ui::res::{AVATAR_PX, BADGE_PX, CARD_PX, EMBED_PX, MEDIA_MULTI_PX, MEDIA_PX, Res};
use crate::ui::text::{Ink, Span, TextLayout};
use crate::ui::{badge_text, embed, text, theme};
use crate::util::time::clock;
use parley::{Cluster, InlineBox, InlineBoxKind, PositionedLayoutItem, VerticalAlign};
use std::sync::Arc;

pub(super) fn seg(s: &mut String, spans: &mut Vec<Span>, g: &Seg) {
  let a = s.len();
  s.push_str(&g.text);
  spans.push(Span { range: a..s.len(), ink: Ink(g.ink), bold: g.bold });
}

// ": words" after `head`, cut with … so the two fit one line `w` wide. control chars become spaces
pub(super) fn one_line(res: &mut Res, head: &str, words: &str, w: f32) -> String {
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

// an event row's color (ext 08-message-rows.css notice kinds, nearest palette color): its words and its
// left edge. None = a plain notice
pub(super) fn notice_ink(n: Notice) -> Option<u32> {
  Some(match n {
    Notice::Plain => return None,
    Notice::Sub | Notice::Raid => theme::MAGENTA,
    Notice::Bits | Notice::Announce => theme::YELLOW,
    Notice::Streak => theme::WHITE,
    Notice::Milestone => theme::CYAN,
    Notice::Redeem => theme::TWITCH,
  })
}

pub(super) fn layout_row(res: &mut Res, chan: &str, m: &Message, muted: bool, width: i32, o: Opts) -> Laid {
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
  let event = notice_ink(m.notice());
  let mut frames = Vec::new();
  // an announcement opens with the ext's `announce` chip (::before on the row): yellow words in a 1px
  // yellow box
  if m.notice() == Notice::Announce {
    let a = s.len();
    s.push_str(" announce ");
    spans.push(Span { range: a..s.len(), ink: Ink(theme::YELLOW), bold: true });
    frames.push((a..s.len(), Frame::Announce));
    s.push(' ');
  }
  if let Some(sys) = &m.system {
    match event {
      Some(ink) => {
        let a = s.len();
        s.push_str(sys);
        spans.push(Span { range: a..s.len(), ink: Ink(ink), bold: true });
      }
      None => dim(&mut s, &mut spans, sys),
    }
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
  // a line sent through heatsync.org says so even where platform tags are off (ext: [H] is never the host)
  let hs = x.is_some_and(|x| x.origin);
  if tags || hs {
    let (t, c) = match m.platform {
      _ if hs => ("[H]", theme::HEATSYNC),
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
  let mut tags = Vec::new();
  let mut pics = Vec::new();
  let mut plays = Vec::new();
  // where the words sit in the text, for search hits
  let mut words_at = 0..0;
  if m.kind == Kind::System {
    // an event says itself in its color; stream lines carry theirs (red live, yellow game switch); notices
    // stay dim
    if let Some(ink) = event {
      let a = s.len();
      s.push_str(&m.body);
      spans.push(Span { range: a..s.len(), ink: Ink(ink), bold: true });
    } else if m.color == 0 {
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
    // a cheer's amount (ext .hs-mc-bits-badge): twitch-colored words in a 1px twitch-colored box
    if let Some(n) = x.map(|x| x.bits).filter(|&n| n > 0) {
      let a = s.len();
      s.push_str(&format!(" {n} bits "));
      spans.push(Span { range: a..s.len(), ink: Ink(theme::TWITCH), bold: true });
      frames.push((a..s.len(), Frame::Bits));
      s.push(' ');
    }
    badges(res, chan, m, &mut s, &mut spans, &mut ib, &mut boxes);
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
    // magenta #tags (ext .hs-hashtag), never inside a link or a person's name
    if !cleared {
      let taken = |r: &std::ops::Range<usize>| {
        link_runs
          .iter()
          .map(|l| &l.0)
          .chain(mentions.iter().map(|n| &n.range))
          .any(|x| x.start < r.end && r.start < x.end)
      };
      tags = crate::model::tagcard::find(&s, b0);
      tags.retain(|(r, _)| r.end <= words_at.end && !taken(r));
      for (r, _) in &tags {
        spans.push(Span { range: r.clone(), ink: Ink(theme::MAGENTA), bold: false });
      }
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
        boxes.push(EmoteBox {
          img: id,
          zw: Vec::new(),
          zw_words: Vec::new(),
          bg: None,
          word: (0, 0),
          blocked: false,
          stand_in: None,
          glyph: None,
        });
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
    let words = if !m.kind.said() { 0..s.len() } else { words_at.clone() };
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
    tags,
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
pub(super) fn post_refs(s: &str, from: usize) -> Vec<(std::ops::Range<usize>, Box<str>)> {
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
pub(super) fn held_row(
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

pub(super) type Spots<'a> = (
  &'a mut std::ops::Range<usize>,
  &'a mut Vec<(std::ops::Range<usize>, Frame)>,
  &'a mut Vec<ImgId>,
  &'a mut Vec<(ImgId, Box<str>)>,
);

// an open thread's media under the words: the held box and its button, or the pictures (each on the
// row's width, wrapping) with a hide over them once a held one is shown
#[allow(clippy::too_many_arguments)]
pub(super) fn post_media(
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
    boxes.push(EmoteBox {
      img: id,
      zw: Vec::new(),
      zw_words: Vec::new(),
      bg: None,
      word: (0, 0),
      blocked: false,
      stand_in: None,
      glyph: None,
    });
  }
}

// (x, width, baseline) of the painted name at `range`: its glyph runs, which share a line
pub(super) fn painted_run(layout: &TextLayout, range: &std::ops::Range<usize>) -> Option<(f32, f32, f32)> {
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
pub(super) fn hits(
  f: &Find,
  s: &str,
  nick: &std::ops::Range<usize>,
  words: std::ops::Range<usize>,
  spans: &mut Vec<Span>,
) {
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
pub(super) struct Card {
  url: Box<str>,
  p: Arc<Preview>,
  // the box's words when the media is held (`[media hidden — nsfw]`)
  held: Option<Box<str>>,
}

// the line's inline picture: its slot, the link word to fold away once the picture is on screen, and
// a preview's card when the line's link is one
pub(super) fn picture(
  res: &mut Res,
  m: &Message,
  width: i32,
  plays: &mut Vec<(ImgId, Box<str>)>,
) -> (Option<ImgId>, Option<std::ops::Range<usize>>, Option<Card>) {
  if !res.media || (m.has(flag::CLEARED) && res.look.dim_timeouts) {
    return (None, None, None);
  }
  let avail = ((width - MARK_W) as f32 - 2.0 * theme::PAD_X).max(1.0) as u32;
  let e = match embed::find(&m.body, res.links && res.partial_links) {
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
pub(super) fn card(
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
      boxes.push(EmoteBox {
        img: id,
        zw: Vec::new(),
        zw_words: Vec::new(),
        bg: None,
        word: (0, 0),
        blocked: false,
        stand_in: None,
        glyph: None,
      });
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

pub(super) fn inline(id: usize, index: usize, w: u32, h: u32) -> InlineBox {
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

pub(super) fn badges(
  res: &mut Res,
  chan: &str,
  m: &Message,
  s: &mut String,
  spans: &mut Vec<Span>,
  ib: &mut Vec<InlineBox>,
  boxes: &mut Vec<EmoteBox>,
) {
  let look = res.look;
  let all = if matches!(m.platform, Platform::Kick | Platform::Youtube) {
    // kick + youtube: roles in order, each a picture (kick sub art) or its word. no per-chatter
    // twitch-id sets here: a kick id is not a twitch id
    for (role, ver) in m.badges.iter() {
      let sub = (m.platform == Platform::Kick && !look.text_badges && &**role == "subscriber")
        .then(|| ver.parse().ok().and_then(|n| res.badges.kick_sub(chan, n)))
        .flatten()
        .map(Box::<str>::from);
      if let Some(url) = sub {
        push_badge(res, &url, None, s, ib, boxes);
        continue;
      }
      let chip = if m.platform == Platform::Kick { badge_text::chip(role) } else { badge_text::youtube_chip(role) };
      let Some((word, ink)) = chip else { continue };
      let a = s.len();
      s.push_str(word);
      spans.push(Span { range: a..s.len(), ink: Ink(ink), bold: false });
      ib.push(gap(s.len(), res.images.px(2)));
    }
    Vec::new()
  } else if look.text_badges {
    // ext textBadges: twitch / kick roles become short words, no picture asked for. the per-chatter
    // third-party sets (ffz, chatterino, 7tv) stay pictures, like the ext
    for (role, _) in m.badges.iter() {
      let Some((word, ink)) = badge_text::chip(role) else { continue };
      let a = s.len();
      s.push_str(word);
      spans.push(Span { range: a..s.len(), ink: Ink(ink), bold: false });
      // ext: 2px between badges (a monospace space is a whole cell)
      ib.push(gap(s.len(), res.images.px(2)));
    }
    res.badges.third_party(m, look.ffz_badges, look.chatterino_badges)
  } else {
    res.badges.for_message(chan, m, look.ffz_badges, look.chatterino_badges)
  };
  let mut found: Vec<(Box<str>, Option<u32>)> = all.into_iter().map(|b| (b.url.clone(), b.bg)).collect();
  // 7tv's badge sits last, right before the name (ext order). seeing a chatter is also what
  // asks heatsync for their cosmetics: batched, cached, answered into the same slot
  if let Some(uid) = PaintId::of_message(m)
    && let Some(b) = res.cosmetics.badge(&uid)
  {
    found.push((b.url.clone(), b.bg));
  }
  for (url, bg) in found {
    push_badge(res, &url, bg, s, ib, boxes);
  }
  avatar(res, m, s, ib, boxes);
}

// one badge picture at the end of the text so far, then the ext's 2px
fn push_badge(res: &mut Res, url: &str, bg: Option<u32>, s: &str, ib: &mut Vec<InlineBox>, boxes: &mut Vec<EmoteBox>) {
  let h = res.images.px(BADGE_PX);
  let img = res.images.id(url, BADGE_PX);
  let w = res.images.width(img, (18, 18));
  ib.push(inline(boxes.len(), s.len(), w, h));
  boxes.push(EmoteBox {
    img,
    zw: Vec::new(),
    zw_words: Vec::new(),
    bg,
    word: (0, 0),
    blocked: false,
    stand_in: None,
    glyph: None,
  });
  // ext: 2px between badges (a monospace space is a whole cell)
  ib.push(gap(s.len(), res.images.px(2)));
}

// the picture before the name (ext: after the badges, 18px square, 3px after). off: no box, no ask.
// on: the square is held from the first layout so the name never moves when the picture lands; the
// first letter in a 1px box stands in until then, and stays when there is no picture
pub(super) fn avatar(res: &mut Res, m: &Message, s: &str, ib: &mut Vec<InlineBox>, boxes: &mut Vec<EmoteBox>) {
  if !res.look.avatars || m.kind == Kind::System {
    return;
  }
  let img = match res.avatars.get(m.platform, &m.login) {
    Some(url) => res.images.id(url, AVATAR_PX),
    None => ImgId::MAX,
  };
  let side = res.images.px(AVATAR_PX);
  ib.push(inline(boxes.len(), s.len(), side, side));
  boxes.push(EmoteBox {
    img,
    zw: Vec::new(),
    zw_words: Vec::new(),
    bg: None,
    word: (0, 0),
    blocked: false,
    stand_in: Some(crate::avatars::tile(&m.login, &m.nick)),
    glyph: None,
  });
  ib.push(gap(s.len(), res.images.px(3)));
}

// an empty run of exact width: its id names no box, so drawing and hit-tests pass over it
pub(super) fn gap(index: usize, w: u32) -> InlineBox {
  InlineBox { id: u64::MAX, ..inline(0, index, w, 1) }
}

// words become text or emote boxes. twitch's own emotes come by position, the rest by name
pub(super) fn body(
  res: &mut Res,
  chan: &str,
  m: &Message,
  fold: Option<&std::ops::Range<usize>>,
  s: &mut String,
  ib: &mut Vec<InlineBox>,
  boxes: &mut Vec<EmoteBox>,
) -> Words {
  let links = if res.links { crate::model::links::find(&m.body, res.partial_links) } else { Vec::new() };
  let mut runs = Vec::new();
  let mut people = Vec::new();
  let px = res.look.emote_px();
  let h = res.images.px(px);
  let mut prev_emote = false;
  // a lone emoji just written as text (where it starts in `s`, its body bytes): an overlay emote right
  // after it stacks on it, like on an emote
  let mut emoji: Option<(usize, usize, usize)> = None;
  let mut first = true;
  let mut at = 0usize;
  for word in m.body.split(' ') {
    let (a, b) = (at, at + word.len());
    at = b + 1;
    if fold.is_some_and(|f| f.start == a && f.end == b) {
      continue;
    }
    let found = find_word(&mut res.sources(), chan, m, a, b, word);
    let blocked = found.as_ref().is_some_and(|f| res.blocked.contains(word, &f.0));
    match found {
      // a blocked overlay just vanishes; a blocked base keeps its slot
      Some((_, _, true, _)) if (prev_emote || emoji.is_some()) && blocked => continue,
      // an overlay on an emoji: the emoji's text becomes a slot with the emoji drawn under the overlay
      Some((url, dims, true, _)) if !prev_emote && let Some((p, ea, eb)) = emoji.take() => {
        s.truncate(p);
        let img = res.images.id(&url, px);
        let w = res.images.width(img, dims).max(h);
        ib.push(inline(boxes.len(), s.len(), w, h));
        boxes.push(EmoteBox {
          img,
          zw: Vec::new(),
          zw_words: Vec::new(),
          bg: None,
          word: (a as u32, b as u32),
          blocked: false,
          stand_in: None,
          glyph: Some(m.body[ea..eb].into()),
        });
        prev_emote = true;
      }
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
        emoji = None;
        ib.push(inline(boxes.len(), s.len(), w, h));
        boxes.push(EmoteBox {
          img,
          zw: Vec::new(),
          zw_words: Vec::new(),
          bg: None,
          word: (a as u32, b as u32),
          blocked,
          stand_in: None,
          glyph: None,
        });
        prev_emote = true;
      }
      None => {
        if !first {
          s.push(' ');
        }
        let p0 = s.len();
        s.push_str(word);
        if !word.is_empty() {
          emoji = text::lone_emoji(word).then_some((p0, a, b));
        }
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
