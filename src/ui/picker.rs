// the emote picker over the chat area: filter row, a virtualized grid by section (rows of
// aspect-true cells, wrapping), a hint row. only cells in view ask for their image, and animated
// ones repaint alone at <=30fps. selected cell is reversed
use std::time::{Duration, Instant};

use super::canvas::{Canvas, Rect};
use super::input_bar::underline;
use super::res::{EMOTE_PX, Res};
use super::text::{Ink, Span};
use super::theme;
use crate::images::{Images, ImgId};
use crate::model::picker::{Focus, Metrics, Picker};

const GAP: i32 = 4;
// the ext's `.hs-mc-picker-emote` clamp: 28px min width, 96px max (logical px)
const MAX_W: u16 = 96;
// repaint ceiling for animated cells, same as chat
const ANIM_MIN_MS: u32 = 33;

pub struct Geo {
  pub search: Rect,
  pub grid: Rect,
  pub foot: Rect,
  pub m: Metrics,
}

pub fn geo(area: Rect, line: i32, ih: i32, max_w: i32) -> Geo {
  let search = Rect::new(area.x, area.y, area.w, line);
  let foot = Rect::new(area.x, area.bottom() - line, area.w, line);
  let grid = Rect::new(area.x, search.bottom() + 1, area.w, (area.h - 2 * line - 1).max(0));
  let m = Metrics { w: grid.w, head: line, ih, min_w: ih, max_w, gap: GAP };
  Geo { search, grid, foot, m }
}

pub fn geo_of(area: Rect, res: &Res) -> Geo {
  geo(area, res.text.line_px as i32, res.images.px(EMOTE_PX) as i32, res.images.px(MAX_W) as i32)
}

// an animated cell on screen: repainted alone when its frame flips
struct Anim {
  inner: Rect,
  clip: Rect,
  id: ImgId,
  on: bool,
}

// what the last full paint looked like; same key + a due animation = repaint only the anims
type Key = (u64, usize, i32, Focus, usize, [i32; 4], usize, usize);

// per-picker paint memory
#[derive(Default)]
pub struct View {
  key: Option<Key>,
  anims: Vec<Anim>,
  // every image in view, touched each pass so the cache never evicts what's on screen
  visible: Vec<ImgId>,
  due: Option<Instant>,
}

impl View {
  // when the next animated frame should be painted, if any are on screen
  pub fn next_anim(&self) -> Option<Instant> {
    self.due
  }
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
  let g = geo_of(area, res);
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
  let (mut anims, mut visible) = (std::mem::take(&mut p.view.anims), std::mem::take(&mut p.view.visible));
  anims.clear();
  visible.clear();
  let mut next: Option<u32> = None;
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
          if blocked.contains(&it.name, &it.url) {
            c.dashed(inner, clip, if on { theme::BLACK } else { theme::WHITE });
            continue;
          }
          let a = Anim { inner, clip, id: images.id(&it.url, EMOTE_PX), on };
          visible.push(a.id);
          if let Some((f, w, h, ms)) = images.frame(a.id) {
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
  p.view = View {
    key: Some(key),
    anims,
    visible,
    due: next.map(|ms| now + Duration::from_millis(ms.max(ANIM_MIN_MS) as u64)),
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
    let g = geo_of(area, &res);
    p.scroll_by(10_000, g.m, g.grid.h);
    draw(&mut c, &mut res, area, &mut p, "");
    let second = res.images.take_wanted().len();
    assert!(second > 0 && second <= 12 * 10, "{second}");
    // a filter that matches nothing draws the empty note, no images
    p.type_str("zzzz");
    draw(&mut c, &mut res, area, &mut p, "");
    assert!(res.images.take_wanted().is_empty());
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
