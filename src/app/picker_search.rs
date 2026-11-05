// the picker's catalog search (ext emotes.js mcTriggerProviderSearches): the word in the filter box is also
// asked of 7tv, ffz and heatsync after a short rest, and more pages follow when the pick nears the end of
// what is shown. what comes back is tiles in the picker and nothing more: a click registers the name the
// way tab-complete does (`catalog_reg`), and only a line SENT with it puts it in your inventory
use std::time::{Duration, Instant};

use super::App;
use crate::model::catalog::{Page, picker_query};
use crate::model::picker::Tab;
use crate::net::Cmd;

// the pause in typing before it asks
const REST: Duration = Duration::from_millis(200);
// ask for the next page this many tiles before the last one
const AHEAD: usize = 12;

impl App {
  // after any key, move or chip: a changed word starts the rest, a pick near the end asks for more
  pub(super) fn picker_search_sync(&mut self) {
    let Some(p) = self.picker.as_mut().filter(|p| p.tab == Tab::Emotes) else {
      self.pick_due = None;
      return;
    };
    let want = picker_query(&p.query).filter(|_| p.prefs.any());
    if want.as_deref().unwrap_or("") != &*p.search.q {
      p.want(want.as_deref());
      p.set_note(String::new());
      self.pick_due = want.map(|_| Instant::now() + REST);
      return self.wake();
    }
    if want.is_none() {
      self.pick_due = None;
    }
    if p.search.asked() && !p.search.pending() && p.sel + AHEAD >= p.len() {
      self.picker_search_ask(true);
    }
  }

  // the rest is over: ask for the first page
  pub(super) fn picker_search_tick(&mut self) {
    self.pick_due = None;
    self.picker_search_ask(true);
  }

  // the next page (a trigger resets the chase budget; a chase does not)
  fn picker_search_ask(&mut self, fresh: bool) {
    let Some(p) = self.picker.as_mut().filter(|p| p.tab == Tab::Emotes) else { return };
    self.catalog_epoch = self.catalog_epoch.wrapping_add(1);
    let prefs = p.prefs;
    let Some(a) = p.search.ask(self.catalog_epoch, &prefs, fresh) else { return };
    if a.page == 1 {
      p.set_note("searching…".into());
    }
    let q = p.search.q.clone();
    self.net.send(Cmd::PickerSearch { q, epoch: self.catalog_epoch, page: a.page, ask: a.which });
    self.wake();
  }

  // a page came back. one for any ask but the newest is dropped. tiles are added and re-ranked; the pick and
  // scroll stay on what they were on
  pub(super) fn on_picker_search(&mut self, epoch: u32, pages: [Option<Page>; 3]) {
    let Some(p) = self.picker.as_mut() else { return };
    let Some(step) = p.search.absorb(epoch, pages) else { return };
    p.set_note(if step.failed { "can't reach the catalogs".into() } else { String::new() });
    p.rerank();
    if step.chase {
      self.picker_search_ask(false);
    } else if !self.picker_filled() {
      // a short list asks for more until the grid is full or the catalogs run out
      self.picker_search_ask(true);
    }
    self.picker_changed_quiet();
  }

  // does what is shown reach past the bottom of the grid (more pages are only for a grid that scrolls)
  fn picker_filled(&self) -> bool {
    let g = self.picker_geo();
    self.picker.as_ref().is_none_or(|p| p.content_h(g.m) > g.grid.h)
  }

  // the grid scrolled to its end: the next page is wanted
  pub(super) fn picker_search_scrolled(&mut self) {
    let g = self.picker_geo();
    let Some(p) = self.picker.as_ref().filter(|p| p.tab == Tab::Emotes && p.search.asked()) else { return };
    if p.scroll + g.grid.h + 2 * g.m.row_h() >= p.content_h(g.m) {
      self.picker_search_ask(true);
    }
  }

  fn picker_changed_quiet(&mut self) {
    self.input_dirty = true;
    self.wake();
  }

  // chip `i` (7tv, ffz, hs, exact) flipped by key or click
  pub(super) fn picker_chip(&mut self, i: usize) {
    let Some(p) = self.picker.as_mut().filter(|p| p.tab == Tab::Emotes) else { return };
    p.flip(i);
    p.set_note(String::new());
    self.picker_changed();
  }
}
