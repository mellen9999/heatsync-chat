// the picker's gifs tab (ext gifs.js): the search box is the picker's filter row. typing asks heatsync
// after a short rest, answers are matched to the last ask (older ones are dropped), the box opens on
// the gifs you sent lately (a request only for a cold reader), and the picked gif starts to play once
// it has rested. picking inserts the gif's link
use std::time::{Duration, Instant};

use super::App;
use crate::model::gifs::{self, Cache, Gif, Page};
use crate::model::picker::Tab;
use crate::net::{Cmd, social};
use crate::util::time::now_ms;

// a pause in typing before it searches, and how long a pick rests before it plays
const REST: Duration = Duration::from_millis(250);
const DWELL: Duration = Duration::from_millis(400);

#[derive(Default)]
pub(super) struct State {
  cache: Cache,
  // the gifs you sent lately, read from disk on first use
  pub(super) recent: Option<Vec<Gif>>,
  // the search the box says now, and the one the grid shows or awaits
  want: Option<String>,
  asked: Option<String>,
  epoch: u32,
  ask_due: Option<Instant>,
  hot_due: Option<Instant>,
  sel: usize,
}

impl State {
  pub(super) fn due(&self) -> Option<Instant> {
    [self.ask_due, self.hot_due].into_iter().flatten().min()
  }
}

impl App {
  // the tab opened: show it at once, from recents or the cache or a request
  pub(super) fn gifs_enter(&mut self) {
    self.gifs.want = None;
    self.gifs.asked = None;
    self.gifs.ask_due = None;
    self.gifs.sel = usize::MAX;
    self.gifs_sync();
    self.gifs.ask_due = None;
    self.gifs_ask();
  }

  // the picker closed or went back to emotes: nothing pending
  pub(super) fn gifs_leave(&mut self) {
    self.gifs.ask_due = None;
    self.gifs.hot_due = None;
    self.gifs.want = None;
    self.gifs.epoch += 1;
  }

  // after any key or move: a changed box schedules a search, a moved pick restarts its rest
  pub(super) fn gifs_sync(&mut self) {
    let Some(p) = self.picker.as_mut().filter(|p| p.tab == Tab::Gifs) else {
      self.gifs.ask_due = None;
      self.gifs.hot_due = None;
      return;
    };
    let q = gifs::clean(&p.query);
    if self.gifs.want.as_deref() != Some(&q) {
      self.gifs.want = Some(q);
      self.gifs.ask_due = Some(Instant::now() + REST);
    }
    if p.sel != self.gifs.sel {
      self.gifs.sel = p.sel;
      p.set_hot(false);
      self.gifs.hot_due = Some(Instant::now() + DWELL);
    }
  }

  pub(super) fn gifs_tick(&mut self, now: Instant) {
    if self.gifs.ask_due.is_some_and(|t| t <= now) {
      self.gifs.ask_due = None;
      self.gifs_ask();
    }
    if self.gifs.hot_due.is_some_and(|t| t <= now) {
      self.gifs.hot_due = None;
      if let Some(p) = self.picker.as_mut().filter(|p| p.tab == Tab::Gifs) {
        p.set_hot(true);
      }
      self.wake();
    }
  }

  // put the grid in step with the box: recents for an empty box, the cache, else ask heatsync
  fn gifs_ask(&mut self) {
    let Some(p) = self.picker.as_mut().filter(|p| p.tab == Tab::Gifs) else { return };
    let q = gifs::clean(&p.query);
    if self.gifs.asked.as_deref() == Some(&q) {
      return;
    }
    self.gifs.asked = Some(q.clone());
    if q.is_empty() {
      let recent = self.gifs.recent.get_or_insert_with(gifs::load_recent);
      if !recent.is_empty() {
        p.set_gifs(recent.clone(), true);
        p.set_note(String::new());
        self.gifs.epoch += 1;
        return self.picker_changed();
      }
    }
    if let Some(page) = self.gifs.cache.get(&q, now_ms()).cloned() {
      self.gifs.epoch += 1;
      return self.gifs_apply(&q, page);
    }
    self.gifs.epoch += 1;
    p.set_note("searching…".into());
    self.net.send(Cmd::Social(social::Cmd::Gifs { q: q.into(), epoch: self.gifs.epoch }));
    self.picker_changed();
  }

  fn gifs_apply(&mut self, q: &str, page: Page) {
    let Some(p) = self.picker.as_mut().filter(|p| p.tab == Tab::Gifs) else { return };
    let note = if page.gifs.is_empty() { gifs::empty_note(q, page.library) } else { String::new() };
    p.set_gifs(page.gifs, false);
    p.set_note(note);
    self.picker_changed();
  }

  // an answer arrives; only the newest ask is shown, but any good answer is worth keeping
  pub(super) fn on_gifs(&mut self, epoch: u32, q: Box<str>, res: Result<Page, gifs::Fail>) {
    if let Ok(page) = &res {
      self.gifs.cache.put(&q, now_ms(), page.clone());
    }
    if epoch != self.gifs.epoch || self.gifs.asked.as_deref() != Some(&*q) {
      return;
    }
    match res {
      Ok(page) => self.gifs_apply(&q, page),
      Err(fail) => {
        // forget the ask so the same text tries again on the next change
        self.gifs.asked = None;
        let Some(p) = self.picker.as_mut().filter(|p| p.tab == Tab::Gifs) else { return };
        p.set_gifs(Vec::new(), false);
        p.set_note(fail.note());
        self.picker_changed();
      }
    }
  }

  // enter on a gif: its link goes in the box, spaced like an emote; the picker closes unless shift held
  pub(super) fn gif_pick(&mut self, keep: bool) {
    let Some(g) = self.picker.as_ref().and_then(|p| p.selected_gif()).cloned() else { return };
    if self.line.text.chars().count() + g.url.chars().count() + 2 > crate::model::line::MAX_CHARS {
      self.say("no room for that link in the box");
      self.bar_dirty = true;
      return self.wake();
    }
    let recent = self.gifs.recent.get_or_insert_with(gifs::load_recent);
    gifs::bump(recent, &g);
    gifs::save_recent(recent);
    self.line.insert_word(&g.url);
    self.complete = None;
    if keep {
      self.input_dirty = true;
      return self.wake();
    }
    self.close_picker();
    self.enter_insert();
  }
}
