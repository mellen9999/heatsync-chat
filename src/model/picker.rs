// emote picker state + grid geometry, no drawing. sections in order: recent, your heatsync
// emotes, this channel, globals. filter is a case-insensitive substring over names already
// loaded locally — never a catalog search, so nothing here can reach the render path
use std::cell::{Ref, RefCell};
use std::ops::Range;

use crate::store;

pub const RECENT_CAP: usize = 40;
const RECENT_FILE: &str = "recent";
const MAX_QUERY: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sec {
  Recent,
  Mine,
  Channel,
  Global,
}

impl Sec {
  pub fn label(self) -> &'static str {
    match self {
      Sec::Recent => "recent",
      Sec::Mine => "my emotes",
      Sec::Channel => "channel",
      Sec::Global => "global",
    }
  }
}

pub struct Item {
  pub sec: Sec,
  pub name: Box<str>,
  pub url: Box<str>,
  pub w: u16,
  pub h: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
  // typing filters; arrows still move
  Search,
  // hjkl move, `/` or `i` goes back to search
  Grid,
}

// most-recent-first emote names, persisted
#[derive(Default)]
pub struct Recents(Vec<Box<str>>);

impl Recents {
  pub fn load() -> Self {
    let names = store::read(RECENT_FILE).unwrap_or_default();
    Self(names.lines().filter(|n| crate::heatsync::valid_name(n)).take(RECENT_CAP).map(Into::into).collect())
  }

  // move `name` to the front (deduped, capped). false = not a legal emote name
  fn bump(&mut self, name: &str) -> bool {
    if !crate::heatsync::valid_name(name) {
      return false;
    }
    self.0.retain(|n| **n != *name);
    self.0.insert(0, name.into());
    self.0.truncate(RECENT_CAP);
    true
  }

  pub fn record(&mut self, name: &str) {
    if !self.bump(name) {
      return;
    }
    let mut s = self.0.join("\n");
    s.push('\n');
    if let Err(e) = store::write(RECENT_FILE, &s) {
      eprintln!("save recent: {e}");
    }
  }

  pub fn names(&self) -> &[Box<str>] {
    &self.0
  }
}

fn ci_cmp(a: &str, b: &str) -> std::cmp::Ordering {
  a.bytes().map(|c| c.to_ascii_lowercase()).cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
}

pub fn contains_ci(hay: &str, needle: &str) -> bool {
  let (h, n) = (hay.as_bytes(), needle.as_bytes());
  n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

// pixel sizes the grid lays out with. a cell is as wide as its emote at row height (aspect-true),
// clamped to [min_w, max_w] like the ext's `.hs-mc-picker-emote`, plus `gap`. all px, already scaled
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Metrics {
  // grid width available to rows
  pub w: i32,
  // section header height
  pub head: i32,
  // emote image height (the row's content height)
  pub ih: i32,
  pub min_w: i32,
  pub max_w: i32,
  pub gap: i32,
}

impl Metrics {
  pub fn row_h(&self) -> i32 {
    self.ih + self.gap
  }

  // cell width for an emote of stored size w x h. unknown dims = square. never wider than a row
  pub fn cell_w(&self, w: u16, h: u16) -> i32 {
    let img = if w == 0 || h == 0 { self.ih } else { (w as i32 * self.ih + h as i32 / 2) / h as i32 };
    (img.clamp(self.min_w, self.max_w.max(self.min_w)) + self.gap).min(self.w.max(1))
  }
}

// one section's place in the scrolled column: a header line, then wrapped rows of cells
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Band {
  pub sec: Sec,
  pub range: Range<usize>,
  pub top: i32,
  pub rows: usize,
  // shown index where each row begins
  pub starts: Vec<usize>,
}

impl Band {
  pub fn cells_top(&self, head: i32) -> i32 {
    self.top + head
  }

  pub fn bottom(&self, m: &Metrics) -> i32 {
    self.cells_top(m.head) + self.rows as i32 * m.row_h()
  }

  pub fn row_of(&self, i: usize) -> usize {
    self.starts.partition_point(|&s| s <= i).saturating_sub(1)
  }

  // shown indices of row `r`
  pub fn row(&self, r: usize) -> Range<usize> {
    let end = self.starts.get(r + 1).copied().unwrap_or(self.range.end);
    self.starts[r]..end
  }
}

// every shown cell's x and width, plus the bands, for one set of metrics
pub struct Layout {
  key: (u64, Metrics),
  pub bands: Vec<Band>,
  // (x, w) per shown index
  pub cells: Vec<(i32, i32)>,
  pub height: i32,
}

impl Layout {
  // x center of cell `i`
  fn mid(&self, i: usize) -> i32 {
    let (x, w) = self.cells[i];
    x + w / 2
  }

  // the cell in `row` whose span holds x, else the nearest by center
  fn nearest(&self, row: Range<usize>, x: i32) -> usize {
    row
      .clone()
      .find(|&i| x >= self.cells[i].0 && x < self.cells[i].0 + self.cells[i].1)
      .unwrap_or_else(|| row.min_by_key(|&i| (self.mid(i) - x).abs()).unwrap_or(0))
  }
}

pub struct Picker {
  items: Vec<Item>,
  // indices into `items` that pass the filter, still in section order
  shown: Vec<u32>,
  // bumps whenever `shown` changes, to invalidate the cached layout
  epoch: u64,
  layout: RefCell<Option<Layout>>,
  // what the ui last painted, so animation can repaint cells alone
  pub view: crate::ui::picker::View,
  pub query: String,
  pub focus: Focus,
  // position within `shown`
  pub sel: usize,
  pub scroll: i32,
}

impl Picker {
  // `groups` are (section, items) already gathered from local sets. recents resolve against them
  pub fn new(mut mine: Vec<Item>, mut channel: Vec<Item>, mut global: Vec<Item>, recents: &Recents) -> Self {
    for v in [&mut mine, &mut channel, &mut global] {
      v.sort_by(|a, b| ci_cmp(&a.name, &b.name));
    }
    let mut items = Vec::with_capacity(RECENT_CAP + mine.len() + channel.len() + global.len());
    for name in recents.names() {
      // the same name from your inventory wins, then the channel, then globals
      let src = [&mine, &channel, &global].into_iter().find_map(|v| v.iter().find(|i| i.name == *name));
      if let Some(i) = src {
        items.push(Item { sec: Sec::Recent, name: i.name.clone(), url: i.url.clone(), w: i.w, h: i.h });
      }
    }
    items.extend(mine);
    items.extend(channel);
    items.extend(global);
    let mut p = Self {
      items,
      shown: Vec::new(),
      epoch: 0,
      layout: RefCell::new(None),
      view: Default::default(),
      query: String::new(),
      focus: Focus::Search,
      sel: 0,
      scroll: 0,
    };
    p.refilter();
    p
  }

  fn refilter(&mut self) {
    self.epoch += 1;
    self.shown.clear();
    let q = &self.query;
    self.shown.extend(self.items.iter().enumerate().filter(|(_, i)| contains_ci(&i.name, q)).map(|(n, _)| n as u32));
    self.sel = 0;
    self.scroll = 0;
  }

  pub fn epoch(&self) -> u64 {
    self.epoch
  }

  #[cfg(test)]
  pub fn len(&self) -> usize {
    self.shown.len()
  }

  pub fn item(&self, shown_idx: usize) -> Option<&Item> {
    self.items.get(*self.shown.get(shown_idx)? as usize)
  }

  pub fn selected(&self) -> Option<&Item> {
    self.item(self.sel)
  }

  // typed or committed text goes to the filter (control bytes dropped, bounded)
  pub fn type_str(&mut self, s: &str) {
    let room = MAX_QUERY.saturating_sub(self.query.chars().count());
    self.query.extend(s.chars().filter(|c| !c.is_control()).take(room));
    self.refilter();
  }

  pub fn backspace(&mut self) {
    if self.query.pop().is_some() {
      self.refilter();
    }
  }

  pub fn clear_query(&mut self) {
    if !self.query.is_empty() {
      self.query.clear();
      self.refilter();
    }
  }

  // sections with something to show, in order
  fn ranges(&self) -> Vec<(Sec, Range<usize>)> {
    let mut v: Vec<(Sec, Range<usize>)> = Vec::with_capacity(4);
    for (n, &i) in self.shown.iter().enumerate() {
      let sec = self.items[i as usize].sec;
      match v.last_mut() {
        Some((s, r)) if *s == sec => r.end = n + 1,
        _ => v.push((sec, n..n + 1)),
      }
    }
    v
  }

  fn build(&self, m: Metrics) -> Layout {
    let mut cells = Vec::with_capacity(self.shown.len());
    let mut bands = Vec::with_capacity(4);
    let mut y = 0;
    for (sec, range) in self.ranges() {
      let (mut x, mut starts) = (0, Vec::new());
      for n in range.clone() {
        let it = &self.items[self.shown[n] as usize];
        let cw = m.cell_w(it.w, it.h);
        if starts.is_empty() || x + cw > m.w {
          starts.push(n);
          x = 0;
        }
        cells.push((x, cw));
        x += cw;
      }
      let b = Band { sec, range, top: y, rows: starts.len(), starts };
      y = b.bottom(&m);
      bands.push(b);
    }
    Layout { key: (self.epoch, m), bands, cells, height: y }
  }

  // cached per (filter, metrics): rebuilt only when the filter or the window/scale changes
  pub fn layout(&self, m: Metrics) -> Ref<'_, Layout> {
    if self.layout.borrow().as_ref().is_none_or(|l| l.key != (self.epoch, m)) {
      *self.layout.borrow_mut() = Some(self.build(m));
    }
    Ref::map(self.layout.borrow(), |l| l.as_ref().expect("built above"))
  }

  pub fn content_h(&self, m: Metrics) -> i32 {
    self.layout(m).height
  }

  // move the selection: left/right walk the cells in order (wrapping rows); up/down go to the
  // nearest-by-center cell in the adjacent row, hopping between sections
  pub fn step(&mut self, dx: i32, dy: i32, m: Metrics) {
    if self.shown.is_empty() {
      return;
    }
    if dx != 0 {
      self.sel = (self.sel as i64 + dx as i64).clamp(0, self.shown.len() as i64 - 1) as usize;
      return;
    }
    let l = self.layout(m);
    let Some(bi) = l.bands.iter().position(|b| b.range.contains(&self.sel)) else { return };
    let (b, r) = (&l.bands[bi], l.bands[bi].row_of(self.sel));
    let x = l.mid(self.sel);
    let row = if dy > 0 {
      if r + 1 < b.rows { Some(b.row(r + 1)) } else { l.bands.get(bi + 1).map(|n| n.row(0)) }
    } else if dy < 0 {
      if r > 0 {
        Some(b.row(r - 1))
      } else if bi > 0 {
        let p = &l.bands[bi - 1];
        Some(p.row(p.rows - 1))
      } else {
        None
      }
    } else {
      None
    };
    if let Some(row) = row {
      let to = l.nearest(row, x);
      drop(l);
      self.sel = to;
    }
  }

  pub fn home(&mut self) {
    self.sel = 0;
  }

  pub fn end(&mut self) {
    self.sel = self.shown.len().saturating_sub(1);
  }

  // scroll just enough that the selected cell (and its header, on a section's first row) shows
  pub fn reveal(&mut self, m: Metrics, view_h: i32) {
    let l = self.layout(m);
    let Some(b) = l.bands.iter().find(|b| b.range.contains(&self.sel)) else { return };
    let row = b.row_of(self.sel);
    let top = if row == 0 { b.top } else { b.cells_top(m.head) + row as i32 * m.row_h() };
    let bottom = b.cells_top(m.head) + (row as i32 + 1) * m.row_h();
    drop(l);
    if top < self.scroll {
      self.scroll = top;
    } else if bottom > self.scroll + view_h {
      self.scroll = bottom - view_h;
    }
  }

  pub fn scroll_by(&mut self, px: i32, m: Metrics, view_h: i32) {
    let max = (self.content_h(m) - view_h).max(0);
    self.scroll = (self.scroll + px).clamp(0, max);
  }

  // the shown index of the cell at (x, y) in grid coordinates (origin = grid top-left, unscrolled
  // by `scroll`), if there is one
  pub fn cell_at(&self, x: i32, y: i32, m: Metrics) -> Option<usize> {
    if x < 0 || m.row_h() <= 0 {
      return None;
    }
    let (l, y) = (self.layout(m), y + self.scroll);
    let b = l.bands.iter().find(|b| y >= b.cells_top(m.head) && y < b.bottom(&m))?;
    let r = ((y - b.cells_top(m.head)) / m.row_h()) as usize;
    b.row(r).find(|&i| x >= l.cells[i].0 && x < l.cells[i].0 + l.cells[i].1)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const M: Metrics = Metrics { w: 128, head: 20, ih: 28, min_w: 28, max_w: 96, gap: 4 };

  fn dim(sec: Sec, name: &str, w: u16, h: u16) -> Item {
    Item { sec, name: name.into(), url: format!("https://cdn.7tv.app/emote/{name}/1x.webp").into(), w, h }
  }

  fn it(sec: Sec, name: &str) -> Item {
    dim(sec, name, 28, 28)
  }

  fn sample() -> Picker {
    let mine = vec![it(Sec::Mine, "zed"), it(Sec::Mine, "Alpha")];
    let chan = (0..7).map(|n| it(Sec::Channel, &format!("c{n}"))).collect();
    let glob = vec![it(Sec::Global, "Kappa"), it(Sec::Global, "kappaPride")];
    let r = Recents(vec!["Kappa".into(), "gone".into()]);
    Picker::new(mine, chan, glob, &r)
  }

  fn names(p: &Picker) -> Vec<&str> {
    (0..p.len()).map(|i| &*p.item(i).unwrap().name).collect()
  }

  #[test]
  fn cell_width_follows_aspect_and_clamps() {
    assert_eq!(M.cell_w(28, 28), 32);
    // 2:1 -> twice the row height
    assert_eq!(M.cell_w(56, 28), 56 + 4);
    // narrow floors at 28, very wide caps at 96
    assert_eq!(M.cell_w(10, 28), 28 + 4);
    assert_eq!(M.cell_w(1000, 28), 96 + 4);
    // unknown dims are square; never wider than the row itself
    assert_eq!(M.cell_w(0, 0), 32);
    assert_eq!(Metrics { w: 50, ..M }.cell_w(1000, 28), 50);
    // dims scale with the row height (hidpi)
    let big = Metrics { ih: 56, min_w: 56, max_w: 192, w: 1000, ..M };
    assert_eq!(big.cell_w(56, 28), 112 + 4);
  }

  #[test]
  fn rows_wrap_by_width_not_count() {
    // widths: 32, 60, 100(wide), 32, 32 in a 128px row
    let g = vec![
      dim(Sec::Global, "a", 28, 28),
      dim(Sec::Global, "b", 56, 28),
      dim(Sec::Global, "c", 400, 28),
      dim(Sec::Global, "d", 28, 28),
      dim(Sec::Global, "e", 0, 0),
    ];
    let p = Picker::new(Vec::new(), Vec::new(), g, &Recents::default());
    let l = p.layout(M);
    let b = &l.bands[0];
    // a+b = 92; c (100) can't join; d (32) can't join c (132 > 128); e joins d
    assert_eq!(b.starts, vec![0, 2, 3]);
    assert_eq!(b.rows, 3);
    assert_eq!(l.cells, vec![(0, 32), (32, 60), (0, 100), (0, 32), (32, 32)]);
    assert_eq!(l.height, 3 * M.row_h() + M.head);
  }

  #[test]
  fn sections_in_order_and_recents_resolve() {
    let p = sample();
    let secs: Vec<_> = p.layout(M).bands.iter().map(|b| (b.sec, b.range.len())).collect();
    // "gone" resolves nowhere and is dropped; mine sorts case-insensitively
    assert_eq!(secs, vec![(Sec::Recent, 1), (Sec::Mine, 2), (Sec::Channel, 7), (Sec::Global, 2)]);
    assert_eq!(&*p.item(1).unwrap().name, "Alpha");
  }

  #[test]
  fn filter_is_case_insensitive_substring_and_bounded() {
    let mut p = sample();
    p.type_str("KAPPA");
    assert_eq!(names(&p), vec!["Kappa", "Kappa", "kappaPride"]);
    p.type_str(&"x".repeat(500));
    assert!(p.query.chars().count() <= MAX_QUERY);
    p.clear_query();
    assert_eq!(p.len(), 12);
    p.type_str("a\u{7}\n");
    assert_eq!(p.query, "a");
  }

  #[test]
  fn filter_invalidates_the_cached_layout() {
    let mut p = sample();
    assert_eq!(p.layout(M).cells.len(), 12);
    p.type_str("c1");
    assert_eq!(p.layout(M).cells.len(), 1);
  }

  #[test]
  fn vertical_moves_pick_the_nearest_cell_by_center() {
    // row 0: a1(60) a2(32) b1(32) = 124; row 1: b2 b3 b4 (32 each)
    let g = vec![
      dim(Sec::Global, "a1", 56, 28),
      dim(Sec::Global, "a2", 28, 28),
      dim(Sec::Global, "b1", 28, 28),
      dim(Sec::Global, "b2", 28, 28),
      dim(Sec::Global, "b3", 28, 28),
      dim(Sec::Global, "b4", 28, 28),
    ];
    let mut p = Picker::new(Vec::new(), Vec::new(), g, &Recents::default());
    let sel_name = |p: &Picker| p.selected().unwrap().name.to_string();
    assert_eq!(p.layout(M).bands[0].starts, vec![0, 3]);
    // a1 center 30 sits over b2 (0..32)
    p.step(0, 1, M);
    assert_eq!(sel_name(&p), "b2");
    p.step(0, -1, M);
    assert_eq!(sel_name(&p), "a1");
    // a2 center 76 sits over b4 (64..96)
    p.step(1, 0, M);
    p.step(0, 1, M);
    assert_eq!(sel_name(&p), "b4");
    // b4 center 80 is inside a2 (60..92); b3 center 48 is inside a1 (0..60)
    p.step(0, -1, M);
    assert_eq!(sel_name(&p), "a2");
    p.sel = 4;
    p.step(0, -1, M);
    assert_eq!(sel_name(&p), "a1");
    // b1 (center 108) has no cell under it below: nothing below at all, stays put
    p.sel = 2;
    p.step(0, 1, M);
    assert_eq!(sel_name(&p), "b4");
    // no row above the first / below the last: stays put
    p.end();
    p.step(0, 1, M);
    assert_eq!(sel_name(&p), "b4");
    p.home();
    p.step(0, -1, M);
    assert_eq!(p.sel, 0);
  }

  #[test]
  fn arrows_hop_between_sections() {
    let mut p = sample();
    // 128px rows hold 4 square cells. recent has 1; down lands in mine
    p.step(0, 1, M);
    assert_eq!(p.selected().unwrap().sec, Sec::Mine);
    p.step(0, 1, M);
    assert_eq!(p.selected().unwrap().sec, Sec::Channel);
    p.step(1, 0, M);
    p.step(1, 0, M);
    p.step(0, 1, M);
    // c2 -> second channel row, same x
    assert_eq!(&*p.selected().unwrap().name, "c6");
    p.step(0, -1, M);
    p.step(0, -1, M);
    assert_eq!(p.selected().unwrap().sec, Sec::Mine);
    p.step(-1, 0, M);
    p.step(-1, 0, M);
    p.step(-1, 0, M);
    assert_eq!(p.sel, 0);
  }

  #[test]
  fn scrolling_reveals_and_clamps() {
    let mut p = sample();
    let view = 100;
    p.end();
    p.reveal(M, view);
    let total = p.content_h(M);
    assert!(p.scroll > 0 && p.scroll + view >= total - M.row_h());
    p.scroll_by(100_000, M, view);
    assert_eq!(p.scroll, total - view);
    p.scroll_by(-100_000, M, view);
    assert_eq!(p.scroll, 0);
    p.home();
    p.reveal(M, view);
    assert_eq!(p.scroll, 0);
  }

  #[test]
  fn hit_testing_cells() {
    let p = sample();
    // recent band: header 0..20, cells 20..52. second band (mine) header 52..72
    assert_eq!(p.cell_at(5, 25, M), Some(0));
    assert_eq!(p.cell_at(40, 25, M), None);
    assert_eq!(p.cell_at(5, 60, M), None);
    assert_eq!(p.cell_at(5, 75, M), Some(1));
    assert_eq!(p.cell_at(40, 75, M), Some(2));
    assert_eq!(p.cell_at(70, 75, M), None);
    // variable widths: x decides the cell
    let g = vec![dim(Sec::Global, "a", 28, 28), dim(Sec::Global, "b", 56, 28)];
    let p = Picker::new(Vec::new(), Vec::new(), g, &Recents::default());
    assert_eq!(p.cell_at(31, 25, M), Some(0));
    assert_eq!(p.cell_at(32, 25, M), Some(1));
    assert_eq!(p.cell_at(91, 25, M), Some(1));
    assert_eq!(p.cell_at(92, 25, M), None);
  }

  #[test]
  fn recents_are_mru_deduped_and_capped() {
    let mut r = Recents::default();
    for n in 0..(RECENT_CAP + 10) {
      assert!(r.bump(&format!("e{n}")));
    }
    assert!(!r.bump("two words") && !r.bump("/me"));
    assert!(r.bump("e5"));
    assert_eq!(&*r.names()[0], "e5");
    assert_eq!(r.names().len(), RECENT_CAP);
    assert_eq!(r.names().iter().filter(|n| ***n == *"e5").count(), 1);
  }
}
