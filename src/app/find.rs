// search in a channel tab (ext live-search.js): `/` in vi normal mode or ctrl-f anywhere opens a prompt in the
// input bar; the tab shows only matching lines as you type, hits in reverse video. enter keeps the filter
// (n / N, or ctrl-g / ctrl-shift-g, walk the matches), esc clears it. `@name` keeps one person's lines.
// leaving the tab drops it, like the ext
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::line::Line;
use crate::model::tabs::Kind;

impl App {
  pub(super) fn find_on(&self) -> bool {
    self.tabs.get(self.active).is_some_and(|t| t.view.searching())
  }

  pub(super) fn find_open(&mut self) {
    if self.kind() != Kind::Chat {
      return self.say("search works in channel tabs");
    }
    if self.picker.is_some() {
      self.close_picker();
    }
    // asking again edits what you asked before
    let mut l = Line::default();
    l.insert(&self.find_q);
    self.prompt = Some(l);
    self.finding = true;
    self.input_dirty = true;
    self.wake();
  }

  // filter to one person: the tab search open on `@login`, already applied (enter keeps it, esc clears)
  pub(super) fn find_for(&mut self, login: &str) {
    self.find_q = format!("@{}", login.to_ascii_lowercase());
    self.find_open();
    self.find_live();
  }

  fn find_set(&mut self, q: &str) {
    let (h, res) = (self.chat_area().h, &mut self.res);
    let Some(t) = self.tabs.get_mut(self.active) else { return };
    let n = t.view.set_search(q, res, h);
    self.find_note = match (t.view.searching(), n) {
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
    if let Some(q) = self.prompt.as_ref().map(|l| l.text.clone()) {
      self.find_set(&q);
    }
  }

  // esc: no search, no prompt. true when there was one
  pub(super) fn find_clear(&mut self) -> bool {
    let was = self.find_on() || self.finding;
    if self.find_on() {
      self.find_set("");
    }
    self.find_q.clear();
    if self.finding {
      self.close_prompt();
    }
    was
  }

  // n / N
  pub(super) fn find_step(&mut self, dir: i32) {
    let (h, res) = (self.chat_area().h, &mut self.res);
    let Some(t) = self.tabs.get_mut(self.active) else { return };
    match t.view.search_step(dir, res, h) {
      Some((at, of)) => self.find_note = format!("{at}/{of}"),
      None => self.find_note = "no match".into(),
    }
    self.input_dirty = true;
    self.wake();
  }

  pub(super) fn find_key(&mut self, key: Key<&str>, text: Option<&str>) {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    let accepts = self.ime.accepts_key_text();
    let Some(l) = self.prompt.as_mut() else { return };
    match key {
      Key::Named(NamedKey::Escape) => {
        self.find_clear();
        return;
      }
      // keep the filter, give the box back
      Key::Named(NamedKey::Enter) => {
        self.find_q = l.text.clone();
        self.close_prompt();
        if !self.find_on() {
          self.find_q.clear();
        }
        return;
      }
      Key::Character("f") if ctrl => {
        self.find_q = l.text.clone();
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
