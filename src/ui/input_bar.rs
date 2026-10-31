// the message box under the chat: where it sends ([T]/[K]/[Y] on merged tabs), the text, a bar
// cursor while typing, a block cursor in vi normal mode. an empty box shows a dim hint. wraps up to MAX_LINES, growing upward
use parley::Affinity;
use parley::editing::Cursor;

use super::canvas::{Canvas, Rect};
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
}

fn layout(text: &mut Text, b: &Bar, width: i32) -> (TextLayout, usize) {
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
  if b.line.text.is_empty() && !b.hint.is_empty() && b.reply.is_none() {
    s.push_str(b.hint);
    spans.push(Span { range: at..s.len(), ink: Ink(theme::DIM), bold: false });
  } else {
    s.push_str(&b.line.text);
  }
  (text.layout(&s, &spans, &[], width as f32 - 2.0 * theme::PAD_X), at)
}

// pixel height the bar wants at this width (one line minimum)
pub fn height(text: &mut Text, b: &Bar, width: i32) -> i32 {
  let (l, _) = layout(text, b, width);
  let line = text.line_px;
  (l.height().max(line).min(line * MAX_LINES)).ceil() as i32
}

pub fn draw(c: &mut Canvas, text: &mut Text, area: Rect, b: &Bar) {
  c.fill(area, theme::BLACK);
  let (l, at) = layout(text, b, area.w);
  // keep the cursor's line visible when the text is taller than the bar
  let cur = Cursor::from_byte_index(&l, at + b.line.cursor, Affinity::Downstream).geometry(&l, CURSOR_W as f32);
  let overflow = (l.height() - area.h as f32).max(0.0);
  let shift = (cur.y1 as f32 - area.h as f32).clamp(0.0, overflow);
  let (ox, oy) = (area.x as f32 + theme::PAD_X, area.y as f32 - shift);
  text.draw(&l, c, ox, oy, area, |_, _| {});
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
        let nx = Cursor::from_byte_index(&l, at + b.line.cursor + ch.len_utf8(), Affinity::Downstream)
          .geometry(&l, CURSOR_W as f32);
        if nx.y0 == cur.y0 && nx.x0 > cur.x0 { (nx.x0 - cur.x0) as f32 } else { em }
      });
      let r = Rect::new(x, y, w.ceil() as i32, h).intersect(area);
      if !r.is_empty() {
        c.fill(r, theme::WHITE);
        if let Some(ch) = ch.filter(|c| !c.is_whitespace()) {
          let s = ch.to_string();
          let span = Span { range: 0..s.len(), ink: Ink(theme::BLACK), bold: false };
          let g = text.layout(&s, &[span], &[], 1000.0);
          text.draw(&g, c, x as f32, y as f32, r, |_, _| {});
        }
      }
    }
    Mode::Chat => {}
  }
}
