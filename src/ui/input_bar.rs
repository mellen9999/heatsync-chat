// the message box under the chat: where it sends ([T]/[K]/[Y] on merged tabs), the text, a bar
// cursor while typing, a block cursor in vi normal mode. an empty box shows a dim hint. wraps up to MAX_LINES, growing upward
use parley::Affinity;
use parley::editing::{Cursor, Selection};

use super::canvas::{Canvas, Rect};
use super::logo::Logo;
use super::text::{Ink, Span, Text, TextLayout};
use super::theme;
use crate::model::line::Line;
use crate::model::vi::Mode;

const MAX_LINES: f32 = 4.0;
const CURSOR_W: i32 = 2;

pub struct Bar<'a> {
  pub line: &'a Line,
  pub mode: Mode,
  // e.g. ("[K] ", GREEN) when the tab merges platforms
  pub target: Option<(&'a str, u32)>,
  pub hint: &'a str,
  // replying to @nick: shown dim before the text
  pub reply: Option<&'a str>,
  // uncommitted input-method text, drawn underlined at the cursor, and its caret (byte offset)
  pub preedit: &'a str,
  pub preedit_caret: usize,
  // the picker is open or the pointer is on the button: drawn reversed
  pub picker_on: bool,
  // the upload button: an upload is running, the path prompt is open or the pointer is on it
  pub upload_on: bool,
  pub logo: &'a Logo,
}

// the heatsync logo button at the right end of the first row
pub fn button(area: Rect, line: i32) -> Rect {
  let w = line * 3 / 2;
  Rect::new(area.right() - w, area.y, w, line.min(area.h))
}

// the upload arrow, just left of the logo
pub fn upload_button(area: Rect, line: i32) -> Rect {
  let b = button(area, line);
  Rect::new(b.x - b.w, b.y, b.w, b.h)
}

struct Laid {
  l: TextLayout,
  // byte index in the layout string where the typed text starts
  at: usize,
  // preedit range in the layout string
  pre: std::ops::Range<usize>,
}

fn layout(text: &mut Text, b: &Bar, width: i32) -> Laid {
  let mut s = String::new();
  let mut spans = Vec::with_capacity(2);
  if let Some((tag, ink)) = b.target {
    s.push_str(tag);
    spans.push(Span { range: 0..tag.len().saturating_sub(1), ink: Ink(ink), bold: true });
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
  if b.line.text.is_empty() && b.preedit.is_empty() && !b.hint.is_empty() && b.reply.is_none() {
    s.push_str(b.hint);
    spans.push(Span { range: at..s.len(), ink: Ink(theme::DIM), bold: false });
  } else if b.preedit.is_empty() {
    s.push_str(&b.line.text);
  } else {
    let (head, tail) = b.line.text.split_at(b.line.cursor);
    s.push_str(head);
    pre = s.len()..s.len() + b.preedit.len();
    s.push_str(b.preedit);
    s.push_str(tail);
  }
  let room = width as f32 - 2.0 * theme::PAD_X - text.line_px * 3.0;
  Laid { l: text.layout(&s, &spans, &[], room), at, pre }
}

// where the caret sits in the layout string: after the composition's caret, else at the cursor
fn caret_index(b: &Bar, g: &Laid) -> usize {
  if g.pre.is_empty() { g.at + b.line.cursor } else { g.pre.start + b.preedit_caret.min(b.preedit.len()) }
}

// pixel height the bar wants at this width (one line minimum)
pub fn height(text: &mut Text, b: &Bar, width: i32) -> i32 {
  let l = layout(text, b, width).l;
  let line = text.line_px;
  (l.height().max(line).min(line * MAX_LINES)).ceil() as i32
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

// returns the caret rect (window px) so the input method can park its candidate window there
pub fn draw(c: &mut Canvas, text: &mut Text, area: Rect, b: &Bar) -> Rect {
  c.fill(area, theme::BLACK);
  let g = layout(text, b, area.w);
  let (l, at) = (&g.l, g.at);
  let caret = caret_index(b, &g);
  // keep the cursor's line visible when the text is taller than the bar
  let cur = Cursor::from_byte_index(l, caret, Affinity::Downstream).geometry(l, CURSOR_W as f32);
  let overflow = (l.height() - area.h as f32).max(0.0);
  let shift = (cur.y1 as f32 - area.h as f32).clamp(0.0, overflow);
  let (ox, oy) = (area.x as f32 + theme::PAD_X, area.y as f32 - shift);
  text.draw(l, c, ox, oy, area, |_, _| {});
  underline(c, l, g.pre.clone(), ox, oy, area);
  let (x, y, h) = ((ox + cur.x0 as f32) as i32, (oy + cur.y0 as f32) as i32, (cur.y1 - cur.y0) as i32);
  match b.mode {
    Mode::Insert => {
      let r = Rect::new(x, y, CURSOR_W, h).intersect(area);
      if !r.is_empty() {
        c.fill(r, theme::WHITE);
      }
    }
    Mode::Normal => {
      // reversed block on the char under the cursor; a one-char-wide box at the end
      let ch = b.line.text[b.line.cursor..].chars().next();
      let em = text.layout("0", &[], &[], 1000.0).full_width();
      let w = ch.map_or(em, |ch| {
        let nx = Cursor::from_byte_index(l, at + b.line.cursor + ch.len_utf8(), Affinity::Downstream)
          .geometry(l, CURSOR_W as f32);
        if nx.y0 == cur.y0 && nx.x0 > cur.x0 { (nx.x0 - cur.x0) as f32 } else { em }
      });
      let r = Rect::new(x, y, w.ceil() as i32, h).intersect(area);
      if !r.is_empty() {
        c.fill(r, theme::WHITE);
        if let Some(ch) = ch.filter(|c| !c.is_whitespace()) {
          let s = ch.to_string();
          let span = Span { range: 0..s.len(), ink: Ink(theme::BLACK), bold: false };
          let gl = text.layout(&s, &[span], &[], 1000.0);
          text.draw(&gl, c, x as f32, y as f32, r, |_, _| {});
        }
      }
    }
    Mode::Chat => {}
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
  let arrow = "↑";
  let span = Span { range: 0..arrow.len(), ink: Ink(fg), bold: true };
  let gl = text.layout(arrow, &[span], &[], 1000.0);
  let (gw, gh) = (gl.full_width(), gl.height());
  text.draw(&gl, c, up.x as f32 + (up.w as f32 - gw) / 2.0, up.y as f32 + (up.h as f32 - gh) / 2.0, up, |_, _| {});
  Rect::new(x, y, CURSOR_W, h)
}
