// the user card (ext profile-card.js): a square box over the chat with the person's name, what heatsync
// knows about them, what they said here lately, and a menu of things to do. text is drawn as one block;
// the actions are a Menu inside the box so keys, mouse and hover work like every other menu
use super::canvas::{Canvas, Rect};
use super::menu::Menu;
use super::text::{Ink, Span, Text};
use super::theme;
use crate::i18n::t;

const PAD: i32 = 8;
// the label column is at least this many cells wide, and as wide as its longest label (translations run long)
const LABEL_W: usize = 10;
const RECENT_MAX: usize = 140;
// a note is up to 2000 chars; the card shows the first of it (it wraps)
const NOTE_SHOWN: usize = 600;

pub struct View<'a> {
  pub nick: &'a str,
  pub color: u32,
  // "he/they" beside the name (ext .hs-card-pronouns, dim); None = unknown or switched off
  pub pronouns: Option<&'a str>,
  // the plus `+` after them, in its tenure color (ext .hs-plus-tenure); None = not a plus member
  pub plus: Option<u32>,
  // dim words after the name ("esc closes"); empty = none
  pub hint: &'a str,
  // "[T] " style platform tag and its color
  pub tag: (&'static str, u32),
  pub rows: &'a [(&'static str, String)],
  // loading / failed / not on heatsync
  pub note: Option<&'a str>,
  // your private note on them (ext card notes section)
  pub mine: Option<&'a str>,
  // (time, text), oldest first
  pub recent: &'a [(String, &'a str)],
}

fn one_line(s: &str) -> String {
  let t: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(RECENT_MAX).collect();
  t
}

// `label` padded with spaces out to `col` px of real laid-out width (at least one space after it)
fn padded(text: &mut Text, label: &str, col: f32, cell: f32) -> String {
  let w = text.layout(label, &[], &[], f32::MAX).full_width();
  format!("{label}{}", " ".repeat((((col - w) / cell).round() as i64).max(1) as usize))
}

// the card's words: name line, then the label: value rows, then a note when there is one
pub fn head(text: &mut Text, v: &View) -> (String, Vec<Span>) {
  let cell = text.layout(" ", &[], &[], f32::MAX).full_width().max(1.0);
  let mut col = cell * LABEL_W as f32;
  let mine = t("ui.card.note");
  for l in v.rows.iter().map(|r| r.0).chain(v.mine.map(|_| mine)) {
    col = col.max(text.layout(l, &[], &[], f32::MAX).full_width() + cell);
  }
  let mut s = String::new();
  let mut spans: Vec<Span> = Vec::new();
  let mut put = |s: &mut String, t: &str, ink: u32, bold: bool| {
    let a = s.len();
    s.push_str(t);
    spans.push(Span { range: a..s.len(), ink: Ink(ink), bold });
  };
  put(&mut s, v.nick, v.color, true);
  if let Some(p) = v.pronouns {
    put(&mut s, " ", theme::WHITE, false);
    put(&mut s, p, theme::DIM, false);
  }
  if let Some(ink) = v.plus {
    put(&mut s, " ", theme::WHITE, false);
    put(&mut s, "+", ink, true);
  }
  put(&mut s, "  ", theme::WHITE, false);
  put(&mut s, v.tag.0.trim_end(), v.tag.1, true);
  if !v.hint.is_empty() {
    put(&mut s, "   ", theme::WHITE, false);
    put(&mut s, v.hint, theme::DIM, false);
  }
  s.push('\n');
  for (label, value) in v.rows {
    put(&mut s, &padded(text, label, col, cell), theme::DIM, false);
    put(&mut s, value, theme::WHITE, false);
    s.push('\n');
  }
  if let Some(n) = v.note {
    put(&mut s, n, theme::DIM, false);
    s.push('\n');
  }
  if let Some(n) = v.mine {
    put(&mut s, &padded(text, mine, col, cell), theme::DIM, false);
    let shown: String = n.chars().take(NOTE_SHOWN).collect();
    put(&mut s, &shown, theme::WHITE, false);
    if shown.len() < n.len() {
      put(&mut s, "…", theme::DIM, false);
    }
    s.push('\n');
  }
  (s, spans)
}

// the whole card; returns its rect so the caller can tell a click outside it
pub fn draw<T: Copy>(c: &mut Canvas, text: &mut Text, area: Rect, v: &View, menu: &mut Menu<T>) -> Rect {
  let (mut s, mut spans) = head(text, v);
  let mut put = |s: &mut String, t: &str, ink: u32, bold: bool| {
    let a = s.len();
    s.push_str(t);
    spans.push(Span { range: a..s.len(), ink: Ink(ink), bold });
  };
  s.push('\n');
  put(&mut s, if v.recent.is_empty() { t("ui.card.none_seen") } else { t("ui.card.this_session") }, theme::DIM, false);
  s.push('\n');
  for (when, body) in v.recent {
    put(&mut s, &format!("{when} "), theme::DIM, false);
    put(&mut s, &one_line(body), theme::WHITE, false);
    s.push('\n');
  }
  let line = text.line_px as i32;
  let w = ((text.font_px * 0.62) as i32 * 72).min(area.w - 16).max(120);
  let layout = text.layout(&s, &spans, &[], (w - 2 * PAD) as f32);
  let menu_h = line * menu.items.len() as i32 + 2;
  let h = (layout.height().ceil() as i32 + 2 * PAD + 2 + menu_h).min(area.h - 8);
  let r = Rect::new(area.x + (area.w - w) / 2, area.y + 8, w, h);
  c.fill(r, theme::WHITE);
  let inner = Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2);
  c.fill(inner, theme::BLACK);
  text.draw(&layout, c, (inner.x + PAD) as f32, (inner.y + PAD) as f32, inner, |_, _| {});
  // the actions sit under the text, inside the box
  let top = inner.y + PAD + layout.height().ceil() as i32;
  menu.anchor_to((inner.x + PAD, top));
  menu.draw(c, text, inner);
  r
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn your_note_is_a_line_of_the_card() {
    let v = View {
      hint: "",
      nick: "bob",
      color: 0,
      pronouns: None,
      plus: None,
      tag: ("[T] ", theme::TWITCH),
      rows: &[],
      note: None,
      mine: Some("likes cats"),
      recent: &[],
    };
    let mut text = Text::new(1.0);
    let got = head(&mut text, &v).0;
    assert!(got.contains("note      likes cats\n"), "{got:?}");
    let none = View { mine: None, ..v };
    assert!(!head(&mut text, &none).0.contains("note"));
  }

  #[test]
  fn pronouns_sit_dim_right_after_the_name() {
    let v = View {
      hint: "",
      nick: "bob",
      color: theme::GREEN,
      pronouns: Some("she/they"),
      plus: None,
      tag: ("[T] ", theme::TWITCH),
      rows: &[],
      note: None,
      mine: None,
      recent: &[],
    };
    let mut text = Text::new(1.0);
    let (s, spans) = head(&mut text, &v);
    assert!(s.starts_with("bob she/they  [T]\n"), "{s:?}");
    let p = spans.iter().find(|sp| &s[sp.range.clone()] == "she/they").expect("a span of its own");
    assert_eq!((p.ink.0, p.bold), (theme::DIM, false));
    assert!(head(&mut text, &View { pronouns: None, ..v }).0.starts_with("bob  [T]\n"));
    // a plus member: a bold `+` in their tenure color after the pronouns (ext card-render plusBadge)
    let (s, spans) = head(&mut text, &View { plus: Some(theme::DIM), ..v });
    assert!(s.starts_with("bob she/they +  [T]\n"), "{s:?}");
    let p = spans.iter().find(|sp| &s[sp.range.clone()] == "+").expect("a span of its own");
    assert_eq!((p.ink.0, p.bold), (theme::DIM, true));
  }

  #[test]
  fn the_card_draws_a_square_box_with_clickable_rows() {
    let mut text = Text::new(1.0);
    let mut c = Canvas::new(800, 600);
    let area = Rect::new(0, 24, 800, 500);
    let rows = [("age", "4y".to_owned()), ("heat", "1.2k".to_owned())];
    let recent = [("12:00".to_owned(), "hello there")];
    let v = View {
      hint: "esc closes",
      nick: "Bob",
      color: theme::GREEN,
      pronouns: Some("he/they"),
      plus: None,
      tag: ("[T] ", theme::TWITCH),
      rows: &rows,
      note: None,
      mine: Some("likes cats"),
      recent: &recent,
    };
    let mut menu = Menu::new(vec![(1, "mention"), (2, "mute")], (0, 0));
    let r = draw(&mut c, &mut text, area, &v, &mut menu);
    assert!(r.x >= 0 && r.right() <= 800 && r.y >= area.y && r.bottom() <= area.bottom());
    // 1px white border, black inside, nothing outside
    assert_eq!(c.px[(r.y * 800 + r.x) as usize], theme::WHITE);
    assert_eq!(c.px[((r.y + 1) * 800 + r.x + 1) as usize], theme::BLACK);
    assert_eq!(c.px[(area.y * 800 + 2) as usize], theme::BLACK);
    // the action rows sit inside the box, last row near its bottom
    let line = text.line_px as i32;
    let last = r.bottom() - 2 - line / 2;
    assert_eq!(menu.row_at(r.x + 20, last, line), Some(1));
    assert_eq!(menu.row_at(r.x + 20, r.y + 2, line), None);
  }
}
