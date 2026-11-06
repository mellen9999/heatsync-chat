// one row of tabs: active reversed (white bg, black text), unread bold in its call color.
// a `+` after the last tab adds one; hover reverses it like the active tab
use super::canvas::{Canvas, Rect};
use super::mode_label::{self, Seg};
use super::text::{Ink, Span, Text};
use super::theme;

pub struct TabLabel<'a> {
  pub name: &'a str,
  // the counter after the name (ext tab-counter), empty for none
  pub count: &'a str,
  pub active: bool,
  pub unread: bool,
  // the color of an unread tab: white for chat, yellow after a stream line, red for mentions, cyan
  // for whispers (the ext's)
  pub hot: u32,
  // tab mode's cursor is on it: underlined
  pub cursor: bool,
  // its channel is live: a red dot at its right end, centered on the line (ext data-live)
  pub live: bool,
  // the one unsaved tab: its name is underlined
  pub temp: bool,
}

// the buttons after the tabs: which is under the pointer (or open), and a newer version's label
pub struct Hot<'a> {
  // the tab under the pointer (its position in `tabs`): reversed like the active one
  pub tab: Option<usize>,
  pub plus: bool,
  pub gear: bool,
  pub keys: bool,
  pub update: Option<(&'a str, bool)>,
}

// returns each tab's rect, then the `+`, the gear and the `?` (for clicks). hot: pointer on a button, or its screen open.
// an area taller than one line is a column (tabs on the left or right, ext tabPosition): a tab a row,
// full width, a 1px white edge toward the chat
pub fn draw(
  c: &mut Canvas,
  text: &mut Text,
  area: Rect,
  tabs: &[TabLabel],
  // the vi mode line when auto-hide folded the input bar away
  mode: (&[Seg], &str),
  hot: Hot,
) -> Vec<Rect> {
  c.fill(area, theme::BLACK);
  let line = text.line_px as i32;
  let column = area.h > line;
  let inner = if column {
    // on the left the edge is its right side, on the right its left side
    let left = area.x == 0;
    c.fill(Rect::new(if left { area.right() - 1 } else { area.x }, area.y, 1, area.h), theme::WHITE);
    Rect::new(if left { area.x } else { area.x + 1 }, area.y, area.w - 1, area.h)
  } else {
    area
  };
  let (mut x, mut y) = (inner.x, inner.y);
  let mut hits = Vec::with_capacity(tabs.len() + 3);
  // the live dot: the ext's 8px at a 20px line, kept clear of the label by its own width
  let d = (line * 2 / 5).max(4);
  let cell =
    |c: &mut Canvas, text: &mut Text, x: &mut i32, y: &mut i32, label: &str, ink: u32, bold: bool, on: bool| {
      let l = text.layout(
        label,
        &[Span { range: 0..label.len(), ink: Ink(if on { theme::BLACK } else { ink }), bold }],
        &[],
        f32::MAX,
      );
      let r = Rect::new(*x, *y, if column { inner.w } else { l.width().ceil() as i32 }, line);
      if on {
        c.fill(r.intersect(inner), theme::WHITE);
      }
      text.draw(&l, c, *x as f32, *y as f32, inner, |_, _| {});
      if column {
        *y += line;
      } else {
        *x += r.w;
      }
      r
    };
  for (i, t) in tabs.iter().enumerate() {
    let rev = t.active || hot.tab == Some(i);
    let ink = if t.unread { t.hot } else { theme::DIM };
    let count = if t.count.is_empty() { String::new() } else { format!(" {}", t.count) };
    let label = if column { format!(" {}{count}", t.name) } else { format!(" {}{count} ", t.name) };
    let r = cell(c, text, &mut x, &mut y, &label, ink, t.unread || t.active, rev);
    let r = if t.live && !column {
      // a row tab grows by the dot; a column tab is full width already
      let wide = Rect::new(r.x, r.y, r.w + d + 2, r.h);
      if rev {
        c.fill(Rect::new(r.right(), r.y, d + 2, r.h).intersect(inner), theme::WHITE);
      }
      x += d + 2;
      wide
    } else {
      r
    };
    if t.live {
      dot(c, Rect::new(r.right() - d - 2, r.y + (r.h - d) / 2, d, d).intersect(inner), d, theme::RED);
    }
    if t.temp && !t.cursor {
      // under the name, not the padding
      let pad = text.layout(" ", &[], &[], f32::MAX).width().ceil() as i32;
      let u =
        Rect::new(r.x + pad, r.bottom() - 2, (r.w - 2 * pad - if t.live && !column { d + 2 } else { 0 }).max(1), 1);
      c.fill(u.intersect(inner), if rev { theme::BLACK } else { theme::WHITE });
    }
    if t.cursor {
      let u = Rect::new(r.x + 2, r.bottom() - 2, (r.w - 4).max(1), 1);
      c.fill(u.intersect(inner), if rev { theme::BLACK } else { theme::WHITE });
    }
    hits.push(r);
  }
  // the three buttons: same width, each glyph centered on its own ink (the gear comes from a fallback
  // font that sits lower than the `+`). in a column they share one row
  let w = text.layout(" + ", &[], &[], f32::MAX).width().ceil() as i32;
  for (glyph, on) in [("+", hot.plus), ("⚙", hot.gear), ("?", hot.keys)] {
    let r = Rect::new(x, y, w, line);
    icon(c, text, r.intersect(inner), glyph, on);
    hits.push(r);
    x += w;
  }
  if column {
    (x, y) = (inner.x, y + line);
  }
  if let Some((label, on)) = hot.update {
    hits.push(cell(c, text, &mut x, &mut y, &format!(" {label} "), theme::YELLOW, true, on));
  }
  // the mode line: after the buttons on a row, wrapped under them in a column
  // a column indents every wrapped line by one cell, like the tab names
  let indent = if column { text.layout(" ", &[], &[], f32::MAX).width().ceil() as i32 } else { 0 };
  let wrap = if column { (inner.w - 2 * indent) as f32 } else { f32::MAX };
  let lead = if column { "" } else { "  " };
  let mut put = |text: &mut Text, s: &str, spans: &[Span], x: &mut i32, y: &mut i32| {
    let l = text.layout(s, spans, &[], wrap);
    text.draw(&l, c, (*x + indent) as f32, *y as f32, inner, |_, _| {});
    if column {
      *y += l.height().ceil() as i32;
    } else {
      *x += l.width().ceil() as i32;
    }
  };
  if !mode.0.is_empty() {
    let s = format!("{lead}{}{}{}", mode_label::text(mode.0), if mode.1.is_empty() { "" } else { "  " }, mode.1);
    let mut spans = mode_label::spans(mode.0);
    for sp in &mut spans {
      sp.range = sp.range.start + lead.len()..sp.range.end + lead.len();
    }
    put(text, &s, &spans, &mut x, &mut y);
  }
  hits
}

// a filled round status dot of diameter `d` at `r` (the one round shape the design allows)
fn dot(c: &mut Canvas, r: Rect, d: i32, ink: u32) {
  let rad = d as f32 / 2.0;
  for y in 0..r.h.min(d) {
    for x in 0..r.w.min(d) {
      let (dx, dy) = (x as f32 + 0.5 - rad, y as f32 + 0.5 - rad);
      if dx * dx + dy * dy <= rad * rad {
        c.fill(Rect::new(r.x + x, r.y + y, 1, 1), ink);
      }
    }
  }
}

// one glyph bold in the middle of `r` by its drawn pixels, not its font's line box. hover reverses it
fn icon(c: &mut Canvas, text: &mut Text, r: Rect, glyph: &str, on: bool) {
  let (bg, ink) = if on { (theme::WHITE, theme::BLACK) } else { (theme::BLACK, theme::WHITE) };
  let l = text.layout(glyph, &[Span { range: 0..glyph.len(), ink: Ink(ink), bold: true }], &[], f32::MAX);
  let mut s = Canvas::new(r.w.max(l.width().ceil() as i32), r.h);
  s.fill(s.bounds(), bg);
  let sb = s.bounds();
  text.draw(&l, &mut s, 0.0, 0.0, sb, |_, _| {});
  c.fill(r, bg);
  // a pixel counts once it is a quarter of the way to the ink: faint antialiasing doesn't pull the middle
  let ink_at = |x: i32, y: i32| {
    let p = s.px[(y * s.w + x) as usize];
    [16, 8, 0].iter().any(|sh| ((p >> sh) & 0xff).abs_diff((bg >> sh) & 0xff) > 0x40)
  };
  let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, -1, -1);
  for y in 0..s.h {
    for x in 0..s.w {
      if ink_at(x, y) {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
      }
    }
  }
  if x1 < 0 {
    return;
  }
  let (dx, dy) = (r.x + (r.w - (x1 - x0 + 1)) / 2 - x0, r.y + (r.h - (y1 - y0 + 1)) / 2 - y0);
  for y in y0..=y1 {
    for x in x0..=x1 {
      let p = s.px[(y * s.w + x) as usize];
      if r.contains(x + dx, y + dy) && c.bounds().contains(x + dx, y + dy) {
        c.px[((y + dy) * c.w + x + dx) as usize] = p;
      }
    }
  }
}
