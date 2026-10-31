// the emote picker over the chat area: filter row, a virtualized grid of square cells by
// section, a hint row. only cells in view ask for their image. selected cell is reversed
use super::canvas::{Canvas, Rect};
use super::input_bar::underline;
use super::res::{EMOTE_PX, Res};
use super::text::{Ink, Span};
use super::theme;
use crate::model::picker::{Focus, Picker};

const GAP: i32 = 4;

pub struct Geo {
  pub search: Rect,
  pub grid: Rect,
  pub foot: Rect,
  pub cols: usize,
  pub head: i32,
  pub cell: i32,
}

pub fn geo(area: Rect, line: i32, emote_px: i32) -> Geo {
  let cell = emote_px + GAP;
  let search = Rect::new(area.x, area.y, area.w, line);
  let foot = Rect::new(area.x, area.bottom() - line, area.w, line);
  let grid = Rect::new(area.x, search.bottom() + 1, area.w, (area.h - 2 * line - 1).max(0));
  Geo { search, grid, foot, cols: ((area.w / cell.max(1)) as usize).max(1), head: line, cell }
}

pub fn draw(c: &mut Canvas, res: &mut Res, area: Rect, p: &Picker, preedit: &str) {
  let Res { text, images, blocked, .. } = res;
  let g = geo(area, text.line_px as i32, images.px(EMOTE_PX) as i32);
  c.fill(area, theme::BLACK);

  // filter row
  let mut s = String::from(&*p.query);
  let pre = s.len()..s.len() + preedit.len();
  s.push_str(preedit);
  let (shown, spans) = if s.is_empty() {
    let hint = "type to filter";
    (hint.to_owned(), vec![Span { range: 0..hint.len(), ink: Ink(theme::DIM), bold: false }])
  } else {
    (s.clone(), Vec::new())
  };
  let l = text.layout(&shown, &spans, &[], 100_000.0);
  let (ox, oy) = (g.search.x as f32 + theme::PAD_X, g.search.y as f32);
  text.draw(&l, c, ox, oy, g.search, |_, _| {});
  if !s.is_empty() {
    underline(c, &l, pre, ox, oy, g.search);
  }
  if p.focus == Focus::Search {
    let x = ox as i32 + if s.is_empty() { 0 } else { l.full_width().ceil() as i32 };
    c.fill(Rect::new(x, g.search.y, 2, g.search.h).intersect(g.search), theme::WHITE);
  }
  c.fill(Rect::new(g.search.x, g.search.bottom(), g.search.w, 1), theme::DIM);

  // grid
  let bands = p.bands(g.cols, g.head, g.cell);
  for b in &bands {
    let top = g.grid.y + b.top - p.scroll;
    if top + g.head + b.rows as i32 * g.cell < g.grid.y || top > g.grid.bottom() {
      continue;
    }
    let label = format!("{} {}", b.sec.label(), b.range.len());
    let span = Span { range: 0..label.len(), ink: Ink(theme::WHITE), bold: true };
    let hl = text.layout(&label, &[span], &[], 100_000.0);
    text.draw(&hl, c, ox, top as f32, g.grid, |_, _| {});
    for (n, i) in b.range.clone().enumerate() {
      let (row, col) = ((n / g.cols) as i32, (n % g.cols) as i32);
      let cell = Rect::new(g.grid.x + col * g.cell, top + g.head + row * g.cell, g.cell, g.cell);
      if cell.bottom() <= g.grid.y || cell.y >= g.grid.bottom() {
        continue;
      }
      let Some(it) = p.item(i) else { continue };
      let clip = cell.intersect(g.grid);
      let on = i == p.sel;
      if on {
        c.fill(clip, theme::WHITE);
      }
      let inner = Rect::new(cell.x + GAP / 2, cell.y + GAP / 2, cell.w - GAP, cell.h - GAP);
      if blocked.contains(&it.name, &it.url) {
        c.dashed(inner, clip, if on { theme::BLACK } else { theme::WHITE });
        continue;
      }
      let id = images.id(&it.url, EMOTE_PX);
      if let Some((f, w, h, _)) = images.frame(id) {
        let (w, h) = (w as i32, h as i32);
        c.rgba(inner.x + (inner.w - w) / 2, inner.y + (inner.h - h) / 2, w, h, &f.rgba, clip);
      }
    }
  }
  if bands.is_empty() {
    let msg = "no emotes match";
    let span = Span { range: 0..msg.len(), ink: Ink(theme::DIM), bold: false };
    let l = text.layout(msg, &[span], &[], 100_000.0);
    text.draw(&l, c, ox, g.grid.y as f32, g.grid, |_, _| {});
  }

  // hint row: what's selected, then the keys that work now
  let keys = match p.focus {
    Focus::Search => "enter insert · tab hjkl · esc close",
    Focus::Grid => "hjkl move · / filter · enter insert · esc close",
  };
  let name = p.selected().map_or("", |i| &i.name);
  let t = if name.is_empty() { keys.to_owned() } else { format!("{name}  {keys}") };
  let spans = [
    Span { range: 0..name.len(), ink: Ink(theme::WHITE), bold: true },
    Span { range: name.len()..t.len(), ink: Ink(theme::DIM), bold: false },
  ];
  let l = text.layout(&t, &spans, &[], 100_000.0);
  c.fill(g.foot, theme::BLACK);
  text.draw(&l, c, ox, g.foot.y as f32, g.foot, |_, _| {});
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::picker::{Item, Recents, Sec};

  #[test]
  fn only_visible_cells_ask_for_images() {
    let mut res = Res::new(1.0);
    let global: Vec<Item> = (0..3000)
      .map(|n| Item {
        sec: Sec::Global,
        name: format!("e{n}").into(),
        url: format!("https://cdn.7tv.app/emote/e{n}/1x.webp").into(),
        w: 28,
        h: 28,
      })
      .collect();
    let mut p = Picker::new(Vec::new(), Vec::new(), global, &Recents::default());
    let mut c = Canvas::new(400, 300);
    let area = Rect::new(0, 0, 400, 300);
    draw(&mut c, &mut res, area, &p, "");
    let first = res.images.take_wanted().len();
    // 400/32 = 12 cols; ~8 visible rows at most
    assert!(first > 0 && first <= 12 * 10, "{first}");
    assert!(c.px.iter().any(|&p| p != theme::BLACK));
    // scrolled deep: a fresh window of cells, still bounded
    let g = geo(area, res.text.line_px as i32, 28);
    p.scroll_by(10_000, g.cols, g.head, g.cell, g.grid.h);
    draw(&mut c, &mut res, area, &p, "");
    let second = res.images.take_wanted().len();
    assert!(second > 0 && second <= 12 * 10, "{second}");
    // a filter that matches nothing draws the empty note, no images
    p.type_str("zzzz");
    draw(&mut c, &mut res, area, &p, "");
    assert!(res.images.take_wanted().is_empty());
  }
}
