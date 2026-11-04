// the message box under the chat: where it sends ([T]/[K]/[Y] on merged tabs), the text, a bar
// cursor while typing, a block cursor in vi normal mode. an empty box shows a dim hint. wraps up to MAX_LINES, growing upward.
// chip words (picked emotes) are drawn as the emote itself, overlays stacked on their base, like the ext's input preview
use std::ops::Range;

use parley::editing::{Cursor, Selection};
use parley::{Affinity, InlineBox, InlineBoxKind, PositionedLayoutItem, VerticalAlign};

use super::canvas::{Canvas, Rect};
use super::chat_view::blit_layer;
use super::logo::Logo;
use super::mode_label::{self, Seg};
use super::text::{Ink, Span, Text, TextLayout};
use super::theme;
use crate::images::{Images, ImgId};
use crate::model::line::{Line, next_break};
use crate::model::vi::Mode;

const MAX_LINES: usize = 4;
const CURSOR_W: i32 = 2;

// one emote drawn in the box: the line bytes it stands for (a base and the overlays stacked on it), its layers, its size
pub struct Chip {
  pub range: Range<usize>,
  pub layers: Vec<ImgId>,
  pub w: u32,
  pub h: u32,
}

pub struct Bar<'a> {
  pub line: &'a Line,
  pub mode: Mode,
  // e.g. ("[K] ", GREEN) when the tab merges platforms
  pub target: Option<(&'a str, u32)>,
  pub hint: &'a str,
  // the hint's color: dim, or red/yellow when the room won't let you chat
  pub hint_ink: u32,
  // the first `link` bytes of the hint are a button (underlined; reversed while the pointer is on it)
  pub link: usize,
  pub link_on: bool,
  // replying to @nick: shown dim before the text
  pub reply: Option<&'a str>,
  // editing your own post: shown dim before its words
  pub edit: bool,
  // uncommitted input-method text, drawn underlined at the cursor, and its caret (byte offset)
  pub preedit: &'a str,
  pub preedit_caret: usize,
  // the picker is open or the pointer is on the button: drawn reversed
  pub picker_on: bool,
  // the upload button: an upload is running, the path prompt is open or the pointer is on it
  pub upload_on: bool,
  pub logo: &'a Logo,
  // vi mode line (empty with vi keys off) and the half-typed command
  pub label: &'a [Seg],
  pub showcmd: &'a str,
  // the line's chip words, in order, not overlapping
  pub chips: &'a [Chip],
  // byte in the line where the text grows past what the platform takes: from there on it draws red (ext
  // .hl-over). the line's length when it fits
  pub over: usize,
}

// the left gutter: wide enough for the widest short label, so the typed text never moves
fn gutter(text: &mut Text, b: &Bar) -> f32 {
  if b.label.is_empty() {
    return 0.0;
  }
  let w = text.layout(mode_label::WIDEST, &[], &[], f32::MAX).full_width();
  // a long label (select hint) only shows over an empty box, so it may outgrow the gutter
  let long = text.layout(&mode_label::text(b.label), &[], &[], f32::MAX).full_width();
  (if b.line.text.is_empty() { long.max(w) } else { w }) + text.line_px * 0.5
}

// the heatsync logo button at the right end of the first row
pub fn button(area: Rect, line: i32) -> Rect {
  let w = line * 3 / 2;
  Rect::new(area.right() - w, area.y, w, line.min(area.h))
}

// the upload plus, just left of the logo
pub fn upload_button(area: Rect, line: i32) -> Rect {
  let b = button(area, line);
  Rect::new(b.x - b.w, b.y, b.w, b.h)
}

struct Laid {
  l: TextLayout,
  // byte index in the layout string where the typed text starts
  at: usize,
  // preedit range in the layout string
  pre: Range<usize>,
  // the typed text's plain runs: (line bytes, where they start in the layout string)
  runs: Vec<(Range<usize>, usize)>,
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

// an empty box shows the dim hint in place of text
fn hinting(b: &Bar) -> bool {
  b.line.text.is_empty() && b.preedit.is_empty() && !b.hint.is_empty() && b.reply.is_none() && !b.edit
}

// the hint's button, in bytes: only a whole piece of the hint counts
fn link_len(b: &Bar) -> usize {
  if b.hint.is_char_boundary(b.link) { b.link.min(b.hint.len()) } else { 0 }
}

// where a byte range of the laid-out text landed, in window pixels (the lines it covers, as one box)
fn span_rect(l: &TextLayout, range: Range<usize>, ox: f32, oy: f32) -> Option<Rect> {
  let sel = Selection::new(
    Cursor::from_byte_index(l, range.start, Affinity::Downstream),
    Cursor::from_byte_index(l, range.end, Affinity::Upstream),
  );
  let boxes = sel.geometry(l);
  let x0 = boxes.iter().map(|(r, _)| r.x0).reduce(f64::min)?;
  let y0 = boxes.iter().map(|(r, _)| r.y0).reduce(f64::min)?;
  let x1 = boxes.iter().map(|(r, _)| r.x1).reduce(f64::max)?;
  let y1 = boxes.iter().map(|(r, _)| r.y1).reduce(f64::max)?;
  Some(Rect::new((ox as f64 + x0) as i32, (oy as f64 + y0) as i32, (x1 - x0).ceil() as i32, (y1 - y0).ceil() as i32))
}

fn layout(text: &mut Text, b: &Bar, width: i32) -> Laid {
  let mut s = String::new();
  let mut spans = Vec::with_capacity(2);
  if let Some((tag, ink)) = b.target {
    s.push_str(tag);
    spans.push(Span { range: 0..tag.len().saturating_sub(1), ink: Ink(ink), bold: true });
  }
  if b.edit {
    let a = s.len();
    s.push_str("\u{270e} editing ");
    spans.push(Span { range: a..s.len(), ink: Ink(theme::DIM), bold: false });
  }
  if let Some(nick) = b.reply {
    let a = s.len();
    s.push_str("↳ @");
    s.push_str(nick);
    s.push(' ');
    spans.push(Span { range: a..s.len(), ink: Ink(theme::DIM), bold: false });
  }
  let at = s.len();
  let mut pre = 0..0;
  let mut runs = Vec::new();
  let mut boxes = Vec::new();
  if hinting(b) {
    s.push_str(b.hint);
    let lead = at + link_len(b);
    if lead > at {
      spans.push(Span { range: at..lead, ink: Ink(theme::WHITE), bold: false });
    }
    spans.push(Span { range: lead..s.len(), ink: Ink(b.hint_ink), bold: false });
  } else {
    let (t, cur) = (&b.line.text, b.line.cursor);
    // the composition goes in at the cursor: before a chip it starts, after one it ends or sits in
    let mut pending = !b.preedit.is_empty();
    let put_pre = |s: &mut String, pre: &mut Range<usize>| {
      *pre = s.len()..s.len() + b.preedit.len();
      s.push_str(b.preedit);
    };
    let mut plain = |s: &mut String, pre: &mut Range<usize>, r: Range<usize>, pending: &mut bool| {
      let split = *pending && r.start <= cur && cur <= r.end;
      let cut = if split { cur } else { r.end };
      runs.push((r.start..cut, s.len()));
      s.push_str(&t[r.start..cut]);
      if split {
        put_pre(s, pre);
        *pending = false;
        runs.push((cut..r.end, s.len()));
        s.push_str(&t[cut..r.end]);
      }
    };
    let mut i = 0;
    for (k, c) in b.chips.iter().enumerate() {
      plain(&mut s, &mut pre, i..c.range.start, &mut pending);
      boxes.push(inline(k, s.len(), c.w, c.h));
      if pending && c.range.start < cur && cur <= c.range.end {
        put_pre(&mut s, &mut pre);
        pending = false;
      }
      i = c.range.end;
    }
    plain(&mut s, &mut pre, i..t.len(), &mut pending);
    for (r, at) in runs.iter().filter(|(r, _)| r.end > b.over) {
      let from = at + r.start.max(b.over) - r.start;
      spans.push(Span { range: from..at + r.len(), ink: Ink(theme::RED), bold: false });
    }
  }
  let cmd = if b.label.is_empty() { 0.0 } else { text.layout("00000", &[], &[], f32::MAX).full_width() };
  let room = width as f32 - 2.0 * theme::PAD_X - text.line_px * 3.0 - gutter(text, b) - cmd;
  Laid { l: text.layout(&s, &spans, &boxes, room), at, pre, runs }
}

// where the text was drawn, to turn a click into a cursor
pub struct Hit {
  // the hint's button, when it is showing
  pub link: Option<Rect>,
  l: TextLayout,
  ox: f32,
  oy: f32,
  at: usize,
  runs: Vec<(Range<usize>, usize)>,
  chips: Vec<Range<usize>>,
  len: usize,
}

impl Hit {
  // the line byte nearest a click: a picture's nearer side, else between the letters
  pub fn byte_at(&self, x: i32, y: i32) -> usize {
    let (lx, ly) = (x as f32 - self.ox, y as f32 - self.oy);
    for (k, r) in self.chips.iter().enumerate() {
      if let Some((bx, by, bw, bh)) = chip_box(&self.l, k)
        && (bx..bx + bw).contains(&lx)
        && (by..by + bh).contains(&ly)
      {
        return if lx < bx + bw / 2.0 { r.start } else { r.end };
      }
    }
    let i = Cursor::from_point(&self.l, lx, ly).index();
    if i < self.at {
      return 0;
    }
    self.runs.iter().find(|(r, o)| (*o..=o + r.len()).contains(&i)).map_or(self.len, |(r, o)| r.start + i - o)
  }
}

// a chip's spot in the layout: x, y, w, h
fn chip_box(l: &TextLayout, k: usize) -> Option<(f32, f32, f32, f32)> {
  l.lines().flat_map(|ln| ln.items().collect::<Vec<_>>()).find_map(|it| match it {
    PositionedLayoutItem::InlineBox(b) if b.id == k as u64 => Some((b.x, b.y, b.width, b.height)),
    _ => None,
  })
}

// the line around layout height `y`: its top and bottom
fn line_span(l: &TextLayout, y: f32) -> (f32, f32) {
  let mut last = (0.0, 0.0);
  for ln in l.lines() {
    let m = ln.metrics();
    last = (m.block_min_coord, m.block_max_coord);
    if y < m.block_max_coord {
      break;
    }
  }
  last
}

// where the caret sits (layout coordinates: x, top, bottom), and the chip it rests on in normal mode
fn caret(b: &Bar, g: &Laid) -> (f32, f32, f32, Option<usize>) {
  let at_byte = |i: usize| {
    let c = Cursor::from_byte_index(&g.l, i, Affinity::Downstream).geometry(&g.l, CURSOR_W as f32);
    (c.x0 as f32, c.y0 as f32, c.y1 as f32, None)
  };
  if !g.pre.is_empty() {
    return at_byte(g.pre.start + b.preedit_caret.min(b.preedit.len()));
  }
  let cur = b.line.cursor;
  for (k, c) in b.chips.iter().enumerate() {
    let side = if c.range.start == cur {
      Some(false)
    } else if c.range.start < cur && cur <= c.range.end {
      Some(true)
    } else {
      None
    };
    if let Some(after) = side
      && let Some((x, y, w, h)) = chip_box(&g.l, k)
    {
      let (top, bot) = line_span(&g.l, y + h / 2.0);
      return (if after { x + w } else { x }, top, bot, (!after).then_some(k));
    }
  }
  let i = g.runs.iter().find(|(r, _)| r.contains(&cur) || r.end == cur).map_or(g.at, |(r, o)| o + cur - r.start);
  at_byte(i)
}

// pixel height the bar wants at this width (one line minimum)
pub fn height(text: &mut Text, b: &Bar, width: i32) -> i32 {
  let l = layout(text, b, width).l;
  let cap = l.lines().nth(MAX_LINES - 1).map_or(l.height(), |ln| ln.metrics().block_max_coord);
  (l.height().min(cap).max(text.line_px)).ceil() as i32
}

// composition underline (vt320 underline attr): one row under every line the range spans
pub fn underline(c: &mut Canvas, l: &TextLayout, range: std::ops::Range<usize>, ox: f32, oy: f32, clip: Rect) {
  if range.is_empty() {
    return;
  }
  let sel = Selection::new(
    Cursor::from_byte_index(l, range.start, Affinity::Downstream),
    Cursor::from_byte_index(l, range.end, Affinity::Upstream),
  );
  for (r, _) in sel.geometry(l) {
    let u = Rect::new((ox as f64 + r.x0) as i32, (oy as f64 + r.y1) as i32 - 1, (r.x1 - r.x0).ceil() as i32, 1);
    c.fill(u.intersect(clip), theme::WHITE);
  }
}

// returns the caret rect (window px) so the input method can park its candidate window there, when an
// animated chip next changes frame (ms), and where the typed text landed (for clicks)
pub fn draw(c: &mut Canvas, text: &mut Text, images: &mut Images, area: Rect, b: &Bar) -> (Rect, Option<u32>, Hit) {
  c.fill(area, theme::BLACK);
  let g = layout(text, b, area.w);
  let l = &g.l;
  let (cx, cy0, cy1, on_chip) = caret(b, &g);
  // keep the cursor's line visible when the text is taller than the bar
  let overflow = (l.height() - area.h as f32).max(0.0);
  let shift = (cy1 - area.h as f32).clamp(0.0, overflow);
  let gut = gutter(text, b);
  let (ox, oy) = (area.x as f32 + theme::PAD_X + gut, area.y as f32 - shift);
  let (x, y, h) = ((ox + cx) as i32, (oy + cy0) as i32, (cy1 - cy0) as i32);
  // normal mode on a chip: the emote sits on the reversed block
  if b.mode == Mode::Normal
    && let Some((_, _, bw, _)) = on_chip.and_then(|k| chip_box(l, k))
  {
    c.fill(Rect::new(x, y, bw.ceil() as i32, h).intersect(area), theme::WHITE);
  }
  let mut next: Option<u32> = None;
  text.draw(l, c, ox, oy, area, |c, pb| {
    let Some(chip) = b.chips.get(pb.id as usize) else { return };
    let rect = Rect::new(pb.x.round() as i32, pb.y.round() as i32, pb.width.round() as i32, pb.height.round() as i32);
    for &id in &chip.layers {
      let Some((f, w, h, flip)) = images.frame(id) else { continue };
      blit_layer(c, &f.rgba, (w, h), (w as i32, h as i32), rect, area);
      if let Some(ms) = flip {
        next = Some(next.map_or(ms, |n| n.min(ms)));
      }
    }
  });
  // the hint's button: underlined, reversed while the pointer is on it
  let link = (hinting(b) && link_len(b) > 0).then(|| span_rect(l, g.at..g.at + link_len(b), ox, oy)).flatten();
  if let Some(r) = link.filter(|r| !r.intersect(area).is_empty()) {
    let clip = r.intersect(area);
    if b.link_on {
      let s = &b.hint[..link_len(b)];
      let black = text.layout(s, &[Span { range: 0..s.len(), ink: Ink(theme::BLACK), bold: false }], &[], f32::MAX);
      c.fill(clip, theme::WHITE);
      text.draw(&black, c, r.x as f32, r.y as f32, clip, |_, _| {});
    } else {
      c.fill(Rect::new(r.x, r.bottom() - 1, r.w, 1).intersect(area), theme::WHITE);
    }
  }
  if !b.label.is_empty() {
    let s = mode_label::text(b.label);
    let ll = text.layout(&s, &mode_label::spans(b.label), &[], f32::MAX);
    text.draw(&ll, c, area.x as f32 + theme::PAD_X, area.y as f32, area, |_, _| {});
  }
  if !b.showcmd.is_empty() {
    let sl =
      text.layout(b.showcmd, &[Span { range: 0..b.showcmd.len(), ink: Ink(theme::DIM), bold: false }], &[], f32::MAX);
    let x = upload_button(area, text.line_px as i32).x as f32 - sl.full_width() - theme::PAD_X;
    text.draw(&sl, c, x, area.y as f32, area, |_, _| {});
  }
  underline(c, l, g.pre.clone(), ox, oy, area);
  match b.mode {
    Mode::Insert => {
      let r = Rect::new(x, y, CURSOR_W, h).intersect(area);
      if !r.is_empty() {
        c.fill(r, theme::WHITE);
      }
    }
    // on a chip the block is drawn under it; over the hint, the mode line says normal
    Mode::Normal if on_chip.is_some() || hinting(b) => {}
    Mode::Normal => {
      // reversed block on the char (or whole emoji) under the cursor; a one-char-wide box at the end
      let cur = b.line.cursor;
      let ch = Some(&b.line.text[cur..next_break(&b.line.text, cur)]).filter(|g| !g.is_empty());
      let em = text.layout("0", &[], &[], 1000.0).full_width();
      let here = g.runs.iter().find(|(r, _)| r.contains(&cur)).map(|(r, o)| o + cur - r.start);
      let w = match (ch, here) {
        (Some(ch), Some(i)) => {
          let nx = Cursor::from_byte_index(l, i + ch.len(), Affinity::Downstream).geometry(l, CURSOR_W as f32);
          if nx.y0 as f32 == cy0 && nx.x0 as f32 > cx { nx.x0 as f32 - cx } else { em }
        }
        _ => em,
      };
      let r = Rect::new(x, y, w.ceil() as i32, h).intersect(area);
      if !r.is_empty() {
        c.fill(r, theme::WHITE);
        if let Some(s) = ch.filter(|g| !g.trim().is_empty()) {
          let span = Span { range: 0..s.len(), ink: Ink(theme::BLACK), bold: false };
          let gl = text.layout(s, &[span], &[], 1000.0);
          text.draw(&gl, c, x as f32, y as f32, r, |_, _| {});
        }
      }
    }
  }
  let btn = button(area, text.line_px as i32);
  let (bg, art) = if b.picker_on { (theme::WHITE, &b.logo.black) } else { (theme::BLACK, &b.logo.color) };
  c.fill(btn, bg);
  if let Some((d, f)) = art.as_ref().and_then(|d| Some((d, d.frames.first()?))) {
    let (w, h) = (d.w as i32, d.h as i32);
    c.rgba(btn.x + (btn.w - w) / 2, btn.y + (btn.h - h) / 2, w, h, &f.rgba, btn);
  }
  let up = upload_button(area, text.line_px as i32);
  let (bg, fg) = if b.upload_on { (theme::WHITE, theme::BLACK) } else { (theme::BLACK, theme::WHITE) };
  c.fill(up, bg);
  // a big bold plus (the ext's image icon is an svg; glyphs only here), drawn square from two bars
  let arm = (up.h * 3 / 5).max(5) | 1;
  let bar = (up.h / 7).max(2) | 1;
  let (cx, cy) = (up.x + up.w / 2, up.y + up.h / 2);
  c.fill(Rect::new(cx - arm / 2, cy - bar / 2, arm, bar), fg);
  c.fill(Rect::new(cx - bar / 2, cy - arm / 2, bar, arm), fg);
  let chips = b.chips.iter().map(|c| c.range.clone()).collect();
  let hit = Hit { link, l: g.l, ox, oy, at: g.at, runs: g.runs, chips, len: b.line.text.len() };
  (Rect::new(x, y, CURSOR_W, h), next, hit)
}
