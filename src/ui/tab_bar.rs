// one row of tabs: active reversed (white bg, black text), unread bold in its call color, status dim at the end.
// a `+` after the last tab adds one; hover reverses it like the active tab
use super::canvas::{Canvas, Rect};
use super::mode_label::{self, Seg};
use super::text::{Ink, Span, Text};
use super::theme;

pub struct TabLabel<'a> {
  pub name: &'a str,
  pub active: bool,
  pub unread: bool,
  // the color of an unread tab: white for chat, yellow after a stream line, red for mentions, cyan
  // for whispers (the ext's)
  pub hot: u32,
}

// returns each tab's rect, then the `+` rect, then the gear (for clicks). hot: pointer on the `+`, gear on or hovered
pub fn draw(
  c: &mut Canvas,
  text: &mut Text,
  area: Rect,
  tabs: &[TabLabel],
  status: &str,
  // the vi mode line when auto-hide folded the input bar away
  mode: (&[Seg], &str),
  hot: (bool, bool),
) -> Vec<Rect> {
  c.fill(area, theme::BLACK);
  let mut x = area.x;
  let mut hits = Vec::with_capacity(tabs.len() + 1);
  let cell = |c: &mut Canvas, text: &mut Text, x: &mut i32, label: &str, ink: u32, bold: bool, on: bool| {
    let l = text.layout(
      label,
      &[Span { range: 0..label.len(), ink: Ink(if on { theme::BLACK } else { ink }), bold }],
      &[],
      f32::MAX,
    );
    let w = l.width().ceil() as i32;
    if on {
      c.fill(Rect::new(*x, area.y, w, area.h), theme::WHITE);
    }
    text.draw(&l, c, *x as f32, area.y as f32, area, |_, _| {});
    let r = Rect::new(*x, area.y, w, area.h);
    *x += w;
    r
  };
  for t in tabs {
    let ink = if t.unread { t.hot } else { theme::DIM };
    hits.push(cell(c, text, &mut x, &format!(" {} ", t.name), ink, t.unread || t.active, t.active));
  }
  // the two buttons: same width, each glyph centered on its own ink (the gear comes from a fallback
  // font that sits lower than the `+`)
  let w = text.layout(" + ", &[], &[], f32::MAX).width().ceil() as i32;
  for (glyph, on) in [("+", hot.0), ("⚙", hot.1)] {
    let r = Rect::new(x, area.y, w, area.h);
    icon(c, text, r, glyph, on);
    hits.push(r);
    x += w;
  }
  if !status.is_empty() {
    let s = format!("  {status}");
    let l = text.layout(&s, &[Span { range: 0..s.len(), ink: Ink(theme::DIM), bold: false }], &[], f32::MAX);
    text.draw(&l, c, x as f32, area.y as f32, area, |_, _| {});
    x += l.width().ceil() as i32;
  }
  if !mode.0.is_empty() {
    let s = format!("  {}{}{}", mode_label::text(mode.0), if mode.1.is_empty() { "" } else { "  " }, mode.1);
    let mut spans = mode_label::spans(mode.0);
    for sp in &mut spans {
      sp.range = sp.range.start + 2..sp.range.end + 2;
    }
    let l = text.layout(&s, &spans, &[], f32::MAX);
    text.draw(&l, c, x as f32, area.y as f32, area, |_, _| {});
  }
  hits
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
