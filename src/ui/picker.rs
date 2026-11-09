// the emote picker over the chat area: filter row, a virtualized grid by section (rows of
// aspect-true cells, wrapping), a hint row. only cells in view ask for their image, and animated
// ones repaint alone at <=30fps. selected cell is reversed
use std::time::{Duration, Instant};

use super::canvas::{Canvas, Rect};
use super::input_bar::underline;
use super::res::{EMOTE_PX, Res};
use super::text::{Ink, Span};
use super::theme;
use crate::i18n::t;
use crate::images::{Images, ImgId};
use crate::model::picker::{Focus, Metrics, Picker, Tab};

const GAP: i32 = 4;
// the ext's `.hs-mc-picker-emote` clamp: 28px min width, 96px max (logical px)
const MAX_W: u16 = 96;
// repaint ceiling for animated cells, same as chat
const ANIM_MIN_MS: u32 = 33;
// a gif cell: a 16:9-ish box, bigger than an emote, still a dense grid (logical px)
pub const GIF_PX: u16 = 64;
const GIF_W: u16 = 112;

pub struct Geo {
  pub search: Rect,
  // the provider chips under the filter row (emotes tab only; zero height elsewhere)
  pub chips: Rect,
  pub grid: Rect,
  pub foot: Rect,
  pub m: Metrics,
}

pub fn geo(area: Rect, line: i32, ih: i32, min_w: i32, max_w: i32, chips: bool) -> Geo {
  let search = Rect::new(area.x, area.y, area.w, line);
  let foot = Rect::new(area.x, area.bottom() - line, area.w, line);
  let chips = Rect::new(area.x, search.bottom() + 1, area.w, if chips { line } else { 0 });
  let grid = Rect::new(area.x, chips.bottom(), area.w, (area.h - 2 * line - 1 - chips.h).max(0));
  let m = Metrics { w: grid.w, head: line, ih, min_w, max_w, gap: GAP };
  Geo { search, chips, grid, foot, m }
}

pub fn geo_of(area: Rect, res: &Res, tab: Tab) -> Geo {
  let line = res.text.line_px as i32;
  match tab {
    Tab::Emotes => {
      let ih = res.images.px(EMOTE_PX) as i32;
      geo(area, line, ih, ih, res.images.px(MAX_W) as i32, true)
    }
    Tab::Gifs => {
      let w = res.images.px(GIF_W) as i32;
      geo(area, line, res.images.px(GIF_PX) as i32, w, w, false)
    }
    // square cells just big enough for a glyph at EMOJI_K times the font
    Tab::Emoji => {
      let side = emoji_side(res);
      geo(area, line, side, side, side, false)
    }
  }
}

// the emoji tab's glyphs draw this many times the font size (the chat's own emoji size)
const EMOJI_K: u16 = 2;

// a cell's side: the glyph's line height at that size
fn emoji_side(res: &Res) -> i32 {
  (res.text.font_px * EMOJI_K as f32 * 1.2).ceil() as i32
}

// an animated cell on screen: repainted alone when its frame flips
struct Anim {
  inner: Rect,
  clip: Rect,
  id: ImgId,
  on: bool,
}

// what the last full paint looked like; same key + a due animation = repaint only the anims
type Key = (u64, usize, i32, Focus, usize, [i32; 4], usize, usize, u32);

// per-picker paint memory
#[derive(Default)]
pub struct View {
  key: Option<Key>,
  anims: Vec<Anim>,
  // every image in view, touched each pass so the cache never evicts what's on screen
  visible: Vec<ImgId>,
  due: Option<Instant>,
  // where the tab names were painted, for clicks
  pub tabs: [Rect; 3],
  // the chips: 7tv, ffz, hs, exact (zero width = not drawn)
  pub chips: [Rect; 4],
}

impl View {
  // the next draw repaints everything (something drawn over the picker went away)
  pub fn forget(&mut self) {
    self.key = None;
  }

  // when the next animated frame should be painted, if any are on screen
  pub fn next_anim(&self) -> Option<Instant> {
    self.due
  }
}

const TAB_PAD: i32 = 6;

// the tab names, right-aligned in the filter row, each padded and square (reversed when current)
fn tab_rects(text: &mut super::text::Text, row: Rect) -> [Rect; 3] {
  let mut x = row.right();
  let mut out = [Rect::new(0, 0, 0, 0); 3];
  for (n, t) in Tab::ALL.into_iter().enumerate().rev() {
    let w = text.layout(t.label(), &[], &[], 100_000.0).full_width().ceil() as i32 + 2 * TAB_PAD;
    x -= w;
    out[n] = Rect::new(x, row.y, w, row.h);
  }
  out
}

// the source chips: three brand names and the exact-match switch
fn chip_name(i: usize) -> &'static str {
  match i {
    0 => "7tv",
    1 => "ffz",
    2 => "hs",
    _ => t("ui.picker.chip_exact"),
  }
}

// the chips, left to right under the filter row: on = white with black text, off = dim on black
fn draw_chips(c: &mut Canvas, text: &mut super::text::Text, p: &Picker, row: Rect) -> [Rect; 4] {
  let mut out = [Rect::new(0, 0, 0, 0); 4];
  if row.h == 0 {
    return out;
  }
  let mut x = row.x + theme::PAD_X as i32;
  for i in 0..4 {
    let name = chip_name(i);
    let w = text.layout(name, &[], &[], 100_000.0).full_width().ceil() as i32 + 2 * TAB_PAD;
    let r = Rect::new(x, row.y, w, row.h);
    let on = p.prefs.is_on(i);
    c.fill(r, if on { theme::WHITE } else { theme::BLACK });
    let ink = Ink(if on { theme::BLACK } else { theme::DIM });
    let l = text.layout(name, &[Span { range: 0..name.len(), ink, bold: on }], &[], 100_000.0);
    text.draw(&l, c, (r.x + TAB_PAD) as f32, r.y as f32, r, |_, _| {});
    out[i] = r;
    x += w + GAP;
  }
  out
}

fn blit(c: &mut Canvas, images: &mut Images, a: &Anim) -> Option<u32> {
  c.fill(a.inner.intersect(a.clip), if a.on { theme::WHITE } else { theme::BLACK });
  let (f, w, h, ms) = images.frame(a.id)?;
  let (w, h) = (w as i32, h as i32);
  c.rgba(a.inner.x + (a.inner.w - w) / 2, a.inner.y + (a.inner.h - h) / 2, w, h, &f.rgba, a.inner.intersect(a.clip));
  ms
}

// paints the picker; returns the damaged rect (None = nothing changed this wake)
pub fn draw(c: &mut Canvas, res: &mut Res, area: Rect, p: &mut Picker, preedit: &str) -> Option<Rect> {
  let g = geo_of(area, res, p.tab);
  // images.bytes() moves when an emote loads or is evicted, which needs a full repaint
  let key: Key = (
    p.epoch(),
    p.sel,
    p.scroll,
    p.focus,
    p.query.len(),
    [area.x, area.y, area.w, area.h],
    preedit.len(),
    res.images.bytes(),
    p.rev(),
  );
  let now = Instant::now();
  if p.view.key == Some(key) && !p.view.anims.is_empty() {
    // nothing but time moved: keep every visible image warm, repaint only the animated cells
    if p.view.due.is_some_and(|t| t > now) {
      return None;
    }
    for &id in &p.view.visible {
      res.images.frame(id);
    }
    let (mut dirty, mut next): (Option<Rect>, Option<u32>) = (None, None);
    for a in &p.view.anims {
      if let Some(ms) = blit(c, &mut res.images, a) {
        next = Some(next.map_or(ms, |n: u32| n.min(ms)));
      }
      let r = a.inner.intersect(a.clip);
      dirty = Some(dirty.map_or(r, |d| super::union(d, r)));
    }
    p.view.due = next.map(|ms| now + Duration::from_millis(ms.max(ANIM_MIN_MS) as u64));
    return dirty;
  }
  let Res { text, images, blocked, .. } = res;
  let gifs = p.tab == Tab::Gifs;
  let emoji = p.tab == Tab::Emoji;
  let wpx = images.px(GIF_W);
  let (mut anims, mut visible) = (std::mem::take(&mut p.view.anims), std::mem::take(&mut p.view.visible));
  anims.clear();
  visible.clear();
  let mut next: Option<u32> = None;
  c.fill(area, theme::BLACK);

  // filter row
  // the upload form, while it is open, has this row: its question (dim), then what is typed
  let ask = p.up.as_ref().map_or("", |u| u.prompt());
  let mut s = String::from(p.up.as_ref().map_or(&*p.query, |u| u.text.as_str()));
  let pre = ask.len() + s.len()..ask.len() + s.len() + preedit.len();
  s.push_str(preedit);
  let (shown, spans) = if !ask.is_empty() {
    let t = format!("{ask}{s}");
    let dim = Span { range: 0..ask.len(), ink: Ink(theme::DIM), bold: false };
    (t, vec![dim])
  } else if s.is_empty() {
    let hint = t(if p.tab == Tab::Gifs { "ui.picker.hint_gifs" } else { "ui.picker.hint_filter" });
    (hint.to_owned(), vec![Span { range: 0..hint.len(), ink: Ink(theme::DIM), bold: false }])
  } else {
    (s.clone(), Vec::new())
  };
  let l = text.layout(&shown, &spans, &[], 100_000.0);
  let (ox, oy) = (g.search.x as f32 + theme::PAD_X, g.search.y as f32);
  // the tab names sit at the right end of the filter row; the text gives way to them
  let tabs = tab_rects(text, g.search);
  let room = Rect::new(g.search.x, g.search.y, (tabs[0].x - g.search.x - 4).max(0), g.search.h);
  text.draw(&l, c, ox, oy, room, |_, _| {});
  if !preedit.is_empty() {
    underline(c, &l, pre, ox, oy, room);
  }
  let shown_is_hint = ask.is_empty() && s.is_empty();
  if p.focus == Focus::Search && p.up.as_ref().is_none_or(|u| u.typing()) {
    let x = ox as i32 + if shown_is_hint { 0 } else { l.full_width().ceil() as i32 };
    c.fill(Rect::new(x, g.search.y, 2, g.search.h).intersect(room), theme::WHITE);
  }
  for (t, r) in Tab::ALL.into_iter().zip(tabs) {
    let on = t == p.tab;
    c.fill(r, if on { theme::WHITE } else { theme::BLACK });
    let ink = Ink(if on { theme::BLACK } else { theme::DIM });
    let tl = text.layout(t.label(), &[Span { range: 0..t.label().len(), ink, bold: on }], &[], 100_000.0);
    text.draw(&tl, c, (r.x + TAB_PAD) as f32, r.y as f32, r, |_, _| {});
  }
  c.fill(Rect::new(g.search.x, g.search.bottom(), g.search.w, 1), theme::DIM);
  let chips = draw_chips(c, text, p, g.chips);

  // grid: only rows intersecting the view are walked
  let row_h = g.m.row_h();
  let mut empty = true;
  {
    let lay = p.layout(g.m);
    for b in &lay.bands {
      empty = false;
      let top = g.grid.y + b.top - p.scroll;
      let cells_y = top + g.m.head;
      if cells_y + b.rows as i32 * row_h < g.grid.y || top > g.grid.bottom() {
        continue;
      }
      let label = format!("{} {}", b.sec.label(), b.range.len());
      let span = Span { range: 0..label.len(), ink: Ink(theme::WHITE), bold: true };
      let hl = text.layout(&label, &[span], &[], 100_000.0);
      text.draw(&hl, c, ox, top as f32, g.grid, |_, _| {});
      let r0 = ((g.grid.y - cells_y) / row_h).max(0) as usize;
      let r1 = (((g.grid.bottom() - cells_y) / row_h + 1).max(0) as usize).min(b.rows);
      for r in r0..r1 {
        let y = cells_y + r as i32 * row_h;
        for i in b.row(r) {
          let (cx, cw) = lay.cells[i];
          let cell = Rect::new(g.grid.x + cx, y, cw, row_h);
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
          if emoji {
            // the glyph centered in its cell; the font draws its own colors, a plain-text fallback goes dark on the pick
            let ink = Ink(if on { theme::BLACK } else { theme::WHITE });
            // U+FE0F asks for the color font even for a glyph the monospace font has a text form of
            let glyph = if it.url.contains('\u{fe0f}') { it.url.to_string() } else { format!("{}\u{fe0f}", it.url) };
            let span = Span { range: 0..glyph.len(), ink, bold: false };
            let gl = text.layout_at(&glyph, &[span], &[], 100_000.0, row_h as f32, (0..glyph.len(), EMOJI_K));
            let (gx, gy) = (
              inner.x + (inner.w - gl.full_width().ceil() as i32) / 2,
              inner.y + (inner.h - gl.height().ceil() as i32) / 2,
            );
            text.draw(&gl, c, gx as f32, gy as f32, clip, |_, _| {});
            continue;
          }
          if !gifs && blocked.contains(&it.name, &it.url) {
            c.dashed(inner, clip, if on { theme::BLACK } else { theme::WHITE });
            continue;
          }
          // a gif holds its first frame; only the picked one, once it has rested, plays
          let (mut id, mut play) =
            if gifs { (images.id_fit(&it.url, GIF_PX, wpx), false) } else { (images.id(&it.url, EMOTE_PX), true) };
          visible.push(id);
          if gifs
            && on
            && p.hot
            && let Some(g) = p.gif(i)
          {
            let moving = images.id_fit(&crate::ui::embed::proxied(&g.animated), GIF_PX, wpx);
            visible.push(moving);
            if images.frame(moving).is_some() {
              (id, play) = (moving, true);
            }
          }
          if gifs && images.failed(id) {
            c.dashed(inner, clip, if on { theme::BLACK } else { theme::WHITE });
            continue;
          }
          let a = Anim { inner, clip, id, on };
          if let Some((f, w, h, ms)) = images.frame_with(a.id, play) {
            let (w, h) = (w as i32, h as i32);
            c.rgba(inner.x + (inner.w - w) / 2, inner.y + (inner.h - h) / 2, w, h, &f.rgba, inner.intersect(clip));
            if let Some(ms) = ms {
              next = Some(next.map_or(ms, |n: u32| n.min(ms)));
              anims.push(a);
            }
          }
        }
      }
    }
  }
  if empty {
    let msg = match (gifs, p.note.is_empty()) {
      (true, false) => &*p.note,
      (true, true) => t("ui.picker.empty_gifs"),
      (false, false) => &*p.note,
      _ if emoji => t("ui.picker.empty_emoji"),
      _ => t("ui.picker.empty_emotes"),
    };
    let span = Span { range: 0..msg.len(), ink: Ink(theme::DIM), bold: false };
    let l = text.layout(msg, &[span], &[], 100_000.0);
    text.draw(&l, c, ox, g.grid.y as f32, g.grid, |_, _| {});
  }

  // hint row: what's selected, then the keys that work now
  let keys = match p.focus {
    _ if p.up.as_ref().is_some_and(|u| u.typing()) => t("ui.picker.keys_upload_typing"),
    _ if p.up.is_some() => t("ui.picker.keys_upload_wait"),
    Focus::Search if emoji => t("ui.picker.keys_search_emoji"),
    Focus::Search if !gifs => t("ui.picker.keys_search_emotes"),
    Focus::Search => t("ui.picker.keys_search_gifs"),
    Focus::Grid => t("ui.picker.keys_grid"),
  };
  let picked = p.selected().map(|i| if emoji { format!(":{}:", i.name) } else { i.name.to_string() });
  let name = if p.note.is_empty() || empty { picked.as_deref().unwrap_or("") } else { &*p.note };
  // one line: the name first, the keys in what room is left, cut with `…` (translations run long)
  let room = g.foot.w - ox as i32;
  let name = text.elide(name, room);
  let name = name.as_str();
  let keys = if name.is_empty() {
    text.elide(keys, room)
  } else {
    let used = text.layout(&format!("{name}  "), &[], &[], f32::MAX).full_width().ceil() as i32;
    if room - used < 3 * g.foot.h { String::new() } else { text.elide(keys, room - used) }
  };
  let t = if name.is_empty() {
    keys
  } else if keys.is_empty() {
    name.to_owned()
  } else {
    format!("{name}  {keys}")
  };
  let spans = [
    Span { range: 0..name.len(), ink: Ink(theme::WHITE), bold: true },
    Span { range: name.len()..t.len(), ink: Ink(theme::DIM), bold: false },
  ];
  let l = text.layout(&t, &spans, &[], 100_000.0);
  c.fill(g.foot, theme::BLACK);
  text.draw(&l, c, ox, g.foot.y as f32, g.foot, |_, _| {});
  p.view = View {
    key: Some(key),
    anims,
    visible,
    due: next.map(|ms| now + Duration::from_millis(ms.max(ANIM_MIN_MS) as u64)),
    tabs,
    chips,
  };
  Some(area)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::picker::{Item, Recents, Sec};

  fn item(n: usize, w: u16) -> Item {
    Item {
      sec: Sec::Global,
      name: format!("e{n}").into(),
      url: format!("https://cdn.7tv.app/emote/e{n}/1x.webp").into(),
      w,
      h: 28,
    }
  }

  #[test]
  fn only_visible_cells_ask_for_images() {
    let mut res = Res::new(1.0);
    // mixed widths, so rows hold different counts
    let global: Vec<Item> = (0..3000).map(|n| item(n, [28, 56, 84][n % 3])).collect();
    let mut p = Picker::new(Vec::new(), Vec::new(), global, &Recents::default());
    let mut c = Canvas::new(400, 300);
    let area = Rect::new(0, 0, 400, 300);
    assert!(draw(&mut c, &mut res, area, &mut p, "").is_some());
    let first = res.images.take_wanted().len();
    assert!(first > 0 && first <= 12 * 10, "{first}");
    assert!(c.px.iter().any(|&p| p != theme::BLACK));
    // scrolled deep: a fresh window of cells, still bounded
    let g = geo_of(area, &res, Tab::Emotes);
    p.scroll_by(10_000, g.m, g.grid.h);
    draw(&mut c, &mut res, area, &mut p, "");
    let second = res.images.take_wanted().len();
    assert!(second > 0 && second <= 12 * 10, "{second}");
    // a filter that matches nothing draws the empty note, no images
    p.type_str("zzzz");
    draw(&mut c, &mut res, area, &mut p, "");
    assert!(res.images.take_wanted().is_empty());
  }

  // the chips sit under the filter row: on = a white tile, off = black; the grid starts below them, and the
  // gifs tab has none
  #[test]
  fn chips_show_which_providers_are_on() {
    let mut res = Res::new(1.0);
    let mut p = Picker::new(Vec::new(), Vec::new(), vec![item(0, 28)], &Recents::default());
    p.flip(1);
    p.flip(3);
    let mut c = Canvas::new(400, 300);
    let area = Rect::new(0, 0, 400, 300);
    draw(&mut c, &mut res, area, &mut p, "");
    let g = geo_of(area, &res, Tab::Emotes);
    let [stv, ffz, hs, exact] = p.view.chips;
    assert!(stv.w > 0 && stv.y == g.chips.y && stv.bottom() <= g.grid.y && ffz.x >= stv.right());
    let at = |r: Rect| c.px[(r.y * c.w + r.x) as usize];
    assert_eq!((at(stv), at(ffz), at(hs), at(exact)), (theme::WHITE, theme::BLACK, theme::WHITE, theme::WHITE));
    assert_eq!(geo_of(area, &res, Tab::Gifs).chips.h, 0);
  }

  #[test]
  fn animated_cells_repaint_alone_and_stay_warm() {
    use crate::images::decode::{Decoded, Frame};
    let mut res = Res::new(1.0);
    let mut p = Picker::new(Vec::new(), Vec::new(), vec![item(0, 28), item(1, 28)], &Recents::default());
    let mut c = Canvas::new(300, 200);
    let area = Rect::new(0, 0, 300, 200);
    draw(&mut c, &mut res, area, &mut p, "");
    // hand the first emote a 2-frame animation (red, green), the second a still
    let wanted = res.images.take_wanted();
    assert_eq!(wanted.len(), 2);
    let px = |rgb: u32| Frame {
      rgba: [rgb.to_le_bytes()[0], rgb.to_le_bytes()[1], rgb.to_le_bytes()[2], 255].repeat(28 * 28).into(),
      delay_ms: 50,
    };
    for (id, _, h) in wanted {
      let frames = if id == 0 { vec![px(0xff), px(0xff00)] } else { vec![px(0xff0000)] };
      res.images.loaded(id, h, Ok(Decoded { frames, w: 28, h: 28 }));
    }
    // loading moved the cache: a full repaint, which now registers the animation
    let r = draw(&mut c, &mut res, area, &mut p, "").unwrap();
    assert_eq!(r, area);
    assert!(p.view.next_anim().is_some());
    // before the next flip is due nothing is repainted
    assert!(draw(&mut c, &mut res, area, &mut p, "").is_none());
    // once due, only the animated cell's rect is damaged, not the whole picker
    std::thread::sleep(std::time::Duration::from_millis(120));
    let r = draw(&mut c, &mut res, area, &mut p, "").unwrap();
    assert!(r.w <= 32 && r.h <= 32, "{r:?}");
    assert!(p.view.next_anim().is_some());
  }
}
