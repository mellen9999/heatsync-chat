// reply threads (ext main.js reply stack): `t` on a picked line, or a click on a reply's `↳ @name` line,
// covers the tab with that line's chain, oldest first, from the tab's own buffer (which keeps filling
// underneath, so new replies land at the bottom). esc / h / the `← back` tab / pressing again goes back.
// it is a takeover like the chat logs: the view is a sibling of the tab's (same room, look, filters) and
// the data lives in model::chain
use crate::i18n::t;
use std::collections::HashSet;

use winit::keyboard::Key;

use super::App;
use crate::model::chain;
use crate::model::message::Message;
use crate::model::tabs::Kind;
use crate::model::vi::Mode;

// lines the covering view holds: the two sides, the picked line, and room for replies landing live
const KEEP: usize = chain::SIDE * 4;

// the chain on screen: its first line, and the ids it holds so far (a live reply joins by answering one)
pub(super) struct Open {
  root: Box<str>,
  ids: HashSet<Box<str>>,
}

impl App {
  pub(super) fn chain_shown(&self) -> bool {
    self.chain.is_some() && self.takeover_tab() == Some(self.active)
  }

  // the tab's own lines, wherever they are right now
  fn chain_buf(&self) -> Option<&crate::ui::chat_view::ChatView> {
    self.tabs.get(self.active).filter(|t| t.kind == Kind::Chat).map(|t| t.under.as_ref().unwrap_or(&t.view))
  }

  // the chain of the line with this id in the active tab (oldest first)
  pub(super) fn chain_of(&self, id: &str) -> Option<chain::Found> {
    if id.is_empty() {
      return None;
    }
    let buf: Vec<&Message> = self.chain_buf()?.msgs().collect();
    chain::find(&buf, id)
  }

  // t: the picked line's chain opens, or the open one closes
  pub(super) fn chain_key(&mut self) {
    if self.chain_shown() {
      return self.chain_close();
    }
    let Some(id) = self.tabs.get(self.active).and_then(|t| t.view.selected()).map(|m| m.id.clone()) else { return };
    self.chain_open(&id);
  }

  // a click on a reply's `↳` line: its chain opens, or the open one closes. true = the click was spent
  pub(super) fn reply_press(&mut self) -> bool {
    if self.kind() != Kind::Chat || self.logs_shown().is_some() {
      return false;
    }
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    let res = &mut self.res;
    let Some(id) = self.tabs.get_mut(self.active).and_then(|t| t.view.reply_at(chat, x, y, res)) else {
      return false;
    };
    if self.chain_shown() {
      self.chain_close();
    } else {
      self.chain_open(&id);
      // the picked line's keys are normal mode's
      if self.chain_shown() && self.set.vi && self.mode == Mode::Insert {
        self.leave_insert();
      }
    }
    self.wake();
    true
  }

  pub(super) fn chain_close(&mut self) {
    self.takeover_close();
  }

  fn chain_open(&mut self, id: &str) {
    if self.kind() != Kind::Chat || self.logs_shown().is_some() || self.tabs[self.active].under.is_some() {
      return;
    }
    // one takeover at a time: the chain left on another tab goes back first
    if self.takeover_tab().is_some() {
      self.takeover_close();
    }
    let Some(f) = self.chain_of(id) else { return self.say(t("app.chain.cant_thread")) };
    if f.lines.len() < 2 {
      return self.say(t("app.chain.no_thread"));
    }
    let (h, w) = (self.chat_area().h, self.body().w);
    let Some(t) = self.tabs.get_mut(self.active) else { return };
    let mut v = t.view.sibling(KEEP);
    v.resize(w);
    v.reload(f.lines);
    t.under = Some(std::mem::replace(&mut t.view, v));
    t.view.select_id(id, &mut self.res, h);
    self.chain = Some(Open { root: f.root, ids: f.ids });
    self.bar_dirty = true;
    self.input_dirty = true;
  }

  // a line landed in tab `i`: it joins the open chain when it belongs there
  pub(super) fn chain_live(&mut self, i: usize, m: &Message) {
    let Some(c) = self.chain.as_mut() else { return };
    if self.tabs.get(i).is_none_or(|t| t.under.is_none()) || !chain::joins(m, &c.root, &c.ids) {
      return;
    }
    c.ids.insert(m.id.clone());
    self.tabs[i].view.push(m.clone());
    if i == self.active {
      self.wake();
    }
  }

  // copy thread: the picked line's chain, oldest first, one `name: text` each
  pub(super) fn copy_thread(&mut self) {
    let Some(id) = self.tabs.get(self.active).and_then(|t| t.view.selected()).map(|m| m.id.clone()) else { return };
    match self.chain_of(&id).and_then(|f| chain::copy_text(&f.lines)) {
      Some(text) => self.copy(&text, t("app.chain.copied")),
      None => self.say(t("app.chain.no_thread")),
    }
  }

  // is `k` the key that opens / closes a chain (t, with nothing half-typed in front of it)
  pub(super) fn chain_wanted(&self, k: &Key<&str>) -> bool {
    *k == Key::Character("t") && !self.mods.control_key() && !self.mods.alt_key() && !self.vi.g_pending()
  }
}
