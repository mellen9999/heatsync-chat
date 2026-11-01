// one row of tabs: active reversed (white bg, black text), unread bold, status dim at the end.
// a `+` after the last tab adds one; hover reverses it like the active tab
use super::canvas::{Canvas, Rect};
use super::mode_label::{self, Seg};
use super::text::{Ink, Span, Text};
use super::theme;

pub struct TabLabel<'a> {
  pub name: &'a str,
  pub active: bool,
  pub unread: bool,
  // the color of an unread tab: white for chat, red for mentions, cyan for whispers (the ext's)
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
  hits.push(cell(c, text, &mut x, " + ", theme::WHITE, true, hot.0));
  hits.push(cell(c, text, &mut x, " ⚙ ", theme::WHITE, true, hot.1));
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
