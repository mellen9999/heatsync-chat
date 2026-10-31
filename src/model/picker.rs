// emote picker state + grid geometry, no drawing. sections in order: recent, your heatsync
// emotes, this channel, globals. filter is a case-insensitive substring over names already
// loaded locally — never a catalog search, so nothing here can reach the render path
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

// one section's place in the scrolled column: a header line, then ceil(len/cols) rows of cells
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Band {
  pub sec: Sec,
  pub range: Range<usize>,
  pub top: i32,
  pub rows: usize,
}

impl Band {
  pub fn cells_top(&self, head: i32) -> i32 {
    self.top + head
  }

  pub fn bottom(&self, head: i32, cell: i32) -> i32 {
    self.cells_top(head) + self.rows as i32 * cell
  }
}

pub struct Picker {
  items: Vec<Item>,
  // indices into `items` that pass the filter, still in section order
  shown: Vec<u32>,
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
    let mut p = Self { items, shown: Vec::new(), query: String::new(), focus: Focus::Search, sel: 0, scroll: 0 };
    p.refilter();
    p
  }

  fn refilter(&mut self) {
    self.shown.clear();
    let q = &self.query;
    self.shown.extend(self.items.iter().enumerate().filter(|(_, i)| contains_ci(&i.name, q)).map(|(n, _)| n as u32));
    self.sel = 0;
    self.scroll = 0;
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

  pub fn bands(&self, cols: usize, head: i32, cell: i32) -> Vec<Band> {
    let mut y = 0;
    self
      .ranges()
      .into_iter()
      .map(|(sec, range)| {
        let rows = range.len().div_ceil(cols.max(1));
        let b = Band { sec, range, top: y, rows };
        y = b.bottom(head, cell);
        b
      })
      .collect()
  }

  pub fn content_h(&self, cols: usize, head: i32, cell: i32) -> i32 {
    self.bands(cols, head, cell).last().map_or(0, |b| b.bottom(head, cell))
  }

  // move the selection one cell: sections are separate grids, so vertical moves hop between them
  pub fn step(&mut self, dx: i32, dy: i32, cols: usize) {
    let cols = cols.max(1);
    if self.shown.is_empty() {
      return;
    }
    let bands = self.bands(cols, 0, 0);
    let Some(bi) = bands.iter().position(|b| b.range.contains(&self.sel)) else { return };
    let b = &bands[bi];
    let p = self.sel - b.range.start;
    if dx != 0 {
      self.sel = (self.sel as i64 + dx as i64).clamp(0, self.shown.len() as i64 - 1) as usize;
    } else if dy > 0 {
      if p + cols < b.range.len() {
        self.sel += cols;
      } else if let Some(n) = bands.get(bi + 1) {
        self.sel = n.range.start + (p % cols).min(n.range.len() - 1);
      }
    } else if dy < 0 {
      if p >= cols {
        self.sel -= cols;
      } else if bi > 0 {
        let pr = &bands[bi - 1];
        let last_row = (pr.range.len() - 1) / cols * cols;
        self.sel = pr.range.start + (last_row + p % cols).min(pr.range.len() - 1);
      }
    }
  }

  pub fn home(&mut self) {
    self.sel = 0;
  }

  pub fn end(&mut self) {
    self.sel = self.shown.len().saturating_sub(1);
  }

  // scroll just enough that the selected cell (and its header, on a section's first row) shows
  pub fn reveal(&mut self, cols: usize, head: i32, cell: i32, view_h: i32) {
    let bands = self.bands(cols, head, cell);
    let Some(b) = bands.iter().find(|b| b.range.contains(&self.sel)) else { return };
    let row = (self.sel - b.range.start) / cols.max(1);
    let top = if row == 0 { b.top } else { b.cells_top(head) + row as i32 * cell };
    let bottom = b.cells_top(head) + (row as i32 + 1) * cell;
    if top < self.scroll {
      self.scroll = top;
    } else if bottom > self.scroll + view_h {
      self.scroll = bottom - view_h;
    }
  }

  pub fn scroll_by(&mut self, px: i32, cols: usize, head: i32, cell: i32, view_h: i32) {
    let max = (self.content_h(cols, head, cell) - view_h).max(0);
    self.scroll = (self.scroll + px).clamp(0, max);
  }

  // the shown index of the cell at (x, y) in grid coordinates (origin = grid top-left, unscrolled
  // by `scroll`), if there is one
  pub fn cell_at(&self, x: i32, y: i32, cols: usize, head: i32, cell: i32) -> Option<usize> {
    if x < 0 || cell <= 0 {
      return None;
    }
    let (col, y) = ((x / cell) as usize, y + self.scroll);
    if col >= cols {
      return None;
    }
    let b = self.bands(cols, head, cell).into_iter().find(|b| y >= b.cells_top(head) && y < b.bottom(head, cell))?;
    let i = b.range.start + ((y - b.cells_top(head)) / cell) as usize * cols + col;
    b.range.contains(&i).then_some(i)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn it(sec: Sec, name: &str) -> Item {
    Item { sec, name: name.into(), url: format!("https://cdn.7tv.app/emote/{name}/1x.webp").into(), w: 28, h: 28 }
  }

  fn sample() -> Picker {
    let mine = vec![it(Sec::Mine, "zed"), it(Sec::Mine, "Alpha")];
    let chan = (0..7).map(|n| it(Sec::Channel, &format!("c{n}"))).collect();
    let glob = vec![it(Sec::Global, "Kappa"), it(Sec::Global, "kappaPride")];
    let r = Recents(vec!["Kappa".into(), "gone".into()]);
    Picker::new(mine, chan, glob, &r)
  }

  #[test]
  fn sections_in_order_and_recents_resolve() {
    let p = sample();
    let secs: Vec<_> = p.bands(4, 20, 32).iter().map(|b| (b.sec, b.range.len())).collect();
    // "gone" resolves nowhere and is dropped; mine sorts case-insensitively
    assert_eq!(secs, vec![(Sec::Recent, 1), (Sec::Mine, 2), (Sec::Channel, 7), (Sec::Global, 2)]);
    assert_eq!(&*p.item(1).unwrap().name, "Alpha");
  }

  #[test]
  fn filter_is_case_insensitive_substring_and_bounded() {
    let mut p = sample();
    p.type_str("KAPPA");
    let names: Vec<_> = (0..p.len()).map(|i| &*p.item(i).unwrap().name).collect();
    assert_eq!(names, vec!["Kappa", "Kappa", "kappaPride"]);
    p.type_str(&"x".repeat(500));
    assert!(p.query.chars().count() <= MAX_QUERY);
    p.clear_query();
    assert_eq!(p.len(), 12);
    p.type_str("a\u{7}\n");
    assert_eq!(p.query, "a");
  }

  #[test]
  fn arrows_hop_between_sections() {
    let mut p = sample();
    // recent has 1; down lands in mine
    p.step(0, 1, 4);
    assert_eq!(p.selected().unwrap().sec, Sec::Mine);
    p.step(0, 1, 4);
    assert_eq!(p.selected().unwrap().sec, Sec::Channel);
    p.step(1, 0, 4);
    p.step(1, 0, 4);
    p.step(0, 1, 4);
    // c2 -> second channel row, col 2
    assert_eq!(&*p.selected().unwrap().name, "c6");
    p.step(0, -1, 4);
    p.step(0, -1, 4);
    assert_eq!(p.selected().unwrap().sec, Sec::Mine);
    p.step(-1, 0, 4);
    p.step(-1, 0, 4);
    p.step(-1, 0, 4);
    assert_eq!(p.sel, 0);
  }

  #[test]
  fn scrolling_reveals_and_clamps() {
    let mut p = sample();
    let (head, cell, view) = (20, 32, 100);
    p.end();
    p.reveal(4, head, cell, view);
    let total = p.content_h(4, head, cell);
    assert!(p.scroll > 0 && p.scroll + view >= total - cell);
    p.scroll_by(100_000, 4, head, cell, view);
    assert_eq!(p.scroll, total - view);
    p.scroll_by(-100_000, 4, head, cell, view);
    assert_eq!(p.scroll, 0);
    p.home();
    p.reveal(4, head, cell, view);
    assert_eq!(p.scroll, 0);
  }

  #[test]
  fn hit_testing_cells() {
    let p = sample();
    let (head, cell) = (20, 32);
    // recent band: header 0..20, cells 20..52. second band (mine) header 52..72
    assert_eq!(p.cell_at(5, 25, 4, head, cell), Some(0));
    assert_eq!(p.cell_at(40, 25, 4, head, cell), None);
    assert_eq!(p.cell_at(5, 60, 4, head, cell), None);
    assert_eq!(p.cell_at(5, 75, 4, head, cell), Some(1));
    assert_eq!(p.cell_at(40, 75, 4, head, cell), Some(2));
    assert_eq!(p.cell_at(4 * 32 + 1, 75, 4, head, cell), None);
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
