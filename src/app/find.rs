// the filter bar at the top of a channel tab (ext live-search.js): `/` in vi normal mode, ctrl-f anywhere or a
// click on the bar gives it the keys; the tab shows only matching lines as you type, hits in reverse video.
// enter keeps the filter (n / N, or ctrl-g / ctrl-shift-g, walk the matches), esc clears it. `@name` keeps one
// person's lines, `/regex/` (or `/regex/i`) matches by pattern. leaving the tab drops it, like the ext.
// the typing goes through the same `prompt` line the path box uses (so paste and input methods just work); the
// chat logs search is still asked in the input bar
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::line::Line;
use crate::model::tabs::Kind;
use crate::ui::canvas::Rect;
use crate::ui::find_bar;

// the chat search: whether the box is the search (not a path prompt), the search as last kept with enter,
// and its count ("3/10", "no match") at the input bar's right
#[derive(Default)]
pub(super) struct FindUi {
  pub(super) on: bool,
  pub(super) query: String,
  pub(super) note: String,
  // the bar's height (0 = not shown) and what it was last painted with
  pub(super) h: i32,
  pub(super) sig: Option<Sig>,
}

#[derive(PartialEq)]
pub(super) struct Sig {
  text: Box<str>,
  cursor: usize,
  preedit: Box<str>,
  hint: &'static str,
  note: Box<str>,
  focused: bool,
  geo: (i32, i32, i32),
}

const FILTER_HINT: &str = "filter: a word, or @name";
const SEARCH_HINT: &str = "search messages…";

impl App {
  pub(super) fn find_on(&self) -> bool {
    self.tabs.get(self.active).is_some_and(|t| t.view.searching())
  }

  pub(super) fn find_open(&mut self) {
    if !matches!(self.kind(), Kind::Chat | Kind::Mentions) && self.logs_shown().is_none() {
      return self.say("search works in channel tabs");
    }
    if self.picker.is_some() {
      self.close_picker();
    }
    // asking again edits what you asked before
    let mut l = Line::default();
    // in the chat logs the search is the archive's: it starts from the one in effect
    l.insert(self.logs_query().unwrap_or(&self.find.query));
    self.prompt = Some(l);
    self.find.on = true;
    self.input_dirty = true;
    self.wake();
  }

  // the filter bar has the keys (not the chat logs' search, which asks in the input bar)
  pub(super) fn bar_find(&self) -> bool {
    self.find.on && self.prompt.is_some() && self.logs_shown().is_none()
  }

  fn find_wanted(&self) -> bool {
    matches!(self.kind(), Kind::Chat | Kind::Mentions)
      && self.screen.is_none()
      && self.img_view.is_none()
      && self.tabs.get(self.active).is_some_and(|t| t.under.is_none())
      && self.logs_shown().is_none()
  }

  pub(super) fn find_area(&self) -> Rect {
    let b = self.body();
    Rect::new(b.x, b.y + self.pin_bar.h + self.pred_bar.h, b.w, self.find.h)
  }

  // repaint the bar if what it shows changed; the chat below re-lays out when it appears or goes
  pub(super) fn sync_find(&mut self) -> Option<Rect> {
    let wanted = self.find_wanted();
    let focused = wanted && self.bar_find();
    let (text, cursor) = match (&self.prompt, focused) {
      (Some(l), true) => (l.text.as_str(), l.cursor),
      _ => (self.find.query.as_str(), self.find.query.len()),
    };
    let hint = if self.kind() == Kind::Mentions { SEARCH_HINT } else { FILTER_HINT };
    let sig = wanted.then(|| Sig {
      text: text.into(),
      cursor,
      preedit: if focused { self.ime.preedit().into() } else { "".into() },
      hint,
      note: self.find.note.as_str().into(),
      focused,
      geo: (self.body().w, self.pin_bar.h + self.pred_bar.h + self.body().y, self.res.text.line_px as i32),
    });
    if sig == self.find.sig {
      return None;
    }
    let old = self.find.h;
    self.find.h = if wanted { find_bar::height(self.res.text.line_px as i32) } else { 0 };
    let area = Rect { h: self.find.h.max(old), ..self.find_area() };
    self.canvas.fill(area, crate::ui::theme::BLACK);
    if wanted {
      let bar = find_bar::FindBar {
        text,
        cursor,
        preedit: if focused { self.ime.preedit() } else { "" },
        focused,
        hint,
        note: &self.find.note,
      };
      let area = self.find_area();
      let caret = find_bar::draw(&mut self.canvas, &mut self.res.text, area, &bar);
      if focused {
        self.ime_win.area = Some(caret);
      }
    }
    self.find.sig = sig;
    if self.find.h != old
      && let Some(t) = self.tabs.get_mut(self.active)
    {
      t.view.invalidate();
    }
    Some(area)
  }

  // true when the click landed on the bar: it takes the keys
  pub(super) fn find_click(&mut self) -> bool {
    let (x, y) = self.pointer;
    if self.find.h == 0 || !self.find_area().contains(x, y) {
      return false;
    }
    if !self.bar_find() {
      self.find_open();
    }
    true
  }

  // filter to one person: the tab search open on `@login`, already applied (enter keeps it, esc clears)
  pub(super) fn find_for(&mut self, login: &str) {
    self.find.query = format!("@{}", login.to_ascii_lowercase());
    self.find_open();
    self.find_live();
  }

  fn find_set(&mut self, q: &str) {
    let (h, res) = (self.chat_area().h, &mut self.res);
    let Some(t) = self.tabs.get_mut(self.active) else { return };
    let n = t.view.set_search(q, res, h);
    self.find.note = match (t.view.searching(), n) {
      (false, _) => String::new(),
      (true, 0) => "no match".into(),
      (true, 1) => "1 found".into(),
      (true, n) => format!("{n} found"),
    };
    self.input_dirty = true;
    self.wake();
  }

  // the prompt's text is the search, live
  pub(super) fn find_live(&mut self) {
    // the archive is asked once, on enter, not on every key
    if self.logs_shown().is_some() {
      return;
    }
    if let Some(q) = self.prompt.as_ref().map(|l| l.text.clone()) {
      self.find_set(&q);
    }
  }

  // esc: no search, no prompt. true when there was one
  pub(super) fn find_clear(&mut self) -> bool {
    let was = self.find_on() || self.find.on;
    if self.find_on() {
      self.find_set("");
    }
    self.find.query.clear();
    if self.find.on {
      self.close_prompt();
    }
    was
  }

  // n / N
  pub(super) fn find_step(&mut self, dir: i32) {
    let (h, res) = (self.chat_area().h, &mut self.res);
    let Some(t) = self.tabs.get_mut(self.active) else { return };
    match t.view.search_step(dir, res, h) {
      Some((at, of)) => self.find.note = format!("{at}/{of}"),
      None => self.find.note = "no match".into(),
    }
    self.input_dirty = true;
    self.wake();
  }

  pub(super) fn find_key(&mut self, key: Key<&str>, text: Option<&str>) {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    let accepts = self.ime.accepts_key_text();
    let logs = self.logs_shown().is_some();
    let Some(l) = self.prompt.as_mut() else { return };
    match key {
      Key::Named(NamedKey::Escape) => {
        self.find_clear();
        return;
      }
      // keep the filter, give the box back
      Key::Named(NamedKey::Enter) if logs => {
        let q = l.text.clone();
        self.close_prompt();
        self.logs_search(&q);
        return;
      }
      Key::Named(NamedKey::Enter) => {
        self.find.query = l.text.clone();
        self.close_prompt();
        // a pattern that will not compile stays up (flagged in the bar) until esc
        if !self.find_on() {
          self.find.query.clear();
        }
        return;
      }
      Key::Character("f") if ctrl => {
        self.find.query = l.text.clone();
        return self.close_prompt();
      }
      _ if super::tabs::edit_line(l, &key, ctrl) => {}
      Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
        if let Some(t) = text.filter(|_| accepts) {
          l.insert(t);
        }
      }
      _ => return,
    }
    self.find_live();
    self.input_dirty = true;
    self.wake();
  }
}
