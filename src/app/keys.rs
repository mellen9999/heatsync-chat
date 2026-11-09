// a key went down: the open layer takes it, else the box's mode (insert types, normal is vim on the box or the chat's own keys)

use super::{App, screen};
use crate::model::message::Platform;
use crate::model::tabs::Kind;
use crate::model::vi::{Action, Mode, Vi};
use crate::net::{Cmd, Reply};
use winit::keyboard::{Key, NamedKey};

impl App {
  pub(super) fn key_insert(&mut self, key: Key<&str>, text: Option<&str>) {
    let ctrl = self.mods.control_key();
    if self.global_key(key.clone()) {
      return;
    }
    if self.row_key(&key) {
      return;
    }
    self.sync_suggest();
    if self.suggest_key(&key) {
      return;
    }
    if key == Key::Named(NamedKey::Tab) {
      return self.tab_complete(self.mods.shift_key());
    }
    // any other key commits the current candidate; an emote pick joins the recents. esc only ends
    // the cycle (and a reply), keeping the pick
    let cycling = self.complete.is_some();
    if let Some(name) = self.complete.take().and_then(|c| c.emote().map(str::to_owned)) {
      self.used_emote(&name);
    }
    if cycling && key == Key::Named(NamedKey::Escape) {
      self.reply = None;
      self.input_dirty = true;
      return self.wake();
    }
    let recall = matches!(key, Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown));
    let space = key == Key::Named(NamedKey::Space) || text == Some(" ");
    // an edit leaves a ctrl-z step (a run of typed letters is one); anything else ends the run
    let alt = self.mods.alt_key();
    match &key {
      Key::Named(NamedKey::Backspace | NamedKey::Delete) => self.line.checkpoint(false),
      Key::Character("w" | "u" | "k") if ctrl => self.line.checkpoint(false),
      Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => self.line.checkpoint(!space),
      _ => self.line.seal(),
    }
    if self.set.vi && key == Key::Named(NamedKey::Escape) {
      self.sync_overlays();
    }
    let l = &mut self.line;
    match key {
      Key::Named(NamedKey::Escape) if self.set.vi => {
        let empty = l.text.is_empty();
        if empty {
          self.reply = None;
          self.edit_end();
        }
        return self.leave_insert();
      }
      Key::Named(NamedKey::Escape) => {
        self.reply = None;
        self.edit_end();
        self.find_clear();
        if let Some(v) = self.view() {
          v.deselect();
        }
      }
      Key::Named(NamedKey::Enter) => self.send(),
      Key::Named(NamedKey::Backspace) if ctrl => l.delete_word_back(),
      Key::Named(NamedKey::Backspace) => l.backspace(),
      Key::Named(NamedKey::Delete) => l.delete(),
      Key::Named(NamedKey::ArrowLeft) if ctrl => l.word_left(),
      Key::Named(NamedKey::ArrowRight) if ctrl => l.word_right(),
      Key::Named(NamedKey::ArrowLeft) => l.left(),
      Key::Named(NamedKey::ArrowRight) => l.right(),
      Key::Named(NamedKey::Home) => l.home(),
      Key::Named(NamedKey::End) => l.end(),
      Key::Named(NamedKey::ArrowUp) => l.older(),
      Key::Named(NamedKey::ArrowDown) => l.newer(),
      Key::Character("a") if ctrl => l.home(),
      Key::Character("e") if ctrl => l.end(),
      Key::Character("w") if ctrl => l.delete_word_back(),
      Key::Character("u") if ctrl => l.clear_to_start(),
      Key::Character("k") if ctrl => l.clear_to_end(),
      Key::Character("z") if ctrl => l.undo(),
      Key::Character("Z" | "y") if ctrl => l.redo(),
      Key::Character("n") if ctrl => self.cycle_send(),
      Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
        // mid-composition the input method owns the text; its commit arrives as an ime event
        if let Some(t) = text.filter(|_| self.ime.accepts_key_text()) {
          l.insert(t);
        }
      }
      _ => return,
    }
    if recall {
      self.chip_recalled();
    } else if space {
      self.chip_spaced();
    }
    self.input_dirty = true;
    self.wake();
  }

  // keys that work in every mode: login, tab switching, page scroll
  pub(super) fn global_key(&mut self, key: Key<&str>) -> bool {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    let half = self.chat_area().h / 2;
    match key {
      Key::Character("o") if ctrl => self.pick_file(),
      Key::Character("f") if ctrl => self.find_open(),
      Key::Character("g") if ctrl && self.find_on() => self.find_step(1),
      Key::Character("G") if ctrl && self.find_on() => self.find_step(-1),
      Key::Character("t") if ctrl => self.open_form(None),
      Key::Character("p") if ctrl => self.inv_open(),
      Key::Character("l") if ctrl => self.login(),
      Key::Character("L") if ctrl => self.net.send(Cmd::Logout),
      Key::Named(NamedKey::Tab) if ctrl => self.select(self.step_tab(self.active, self.mods.shift_key())),
      Key::Character(d) if alt && d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
        let n = (d.as_bytes()[0] - b'0') as usize;
        if let Some(i) = self.nth_visible(if n == 0 { 9 } else { n - 1 }) {
          self.select(i);
        }
      }
      Key::Character(c @ ("[" | "]")) if alt && !ctrl => self.select(self.step_tab(self.active, c == "[")),
      Key::Character("r") if alt && !ctrl => self.row_mode(),
      Key::Character(c @ ("t" | "k" | "y")) if alt && !ctrl => {
        let p = match c {
          "t" => Platform::Twitch,
          "k" => Platform::Kick,
          _ => Platform::Youtube,
        };
        return self.toggle_send(p);
      }
      Key::Named(NamedKey::PageDown) => self.scroll(-half * 2),
      Key::Named(NamedKey::PageUp) => self.scroll(half * 2),
      _ => return false,
    }
    true
  }

  // a key went down: the open layer first (form, screen, picture, prompt, picker, menus, the chat logs),
  // then the box's mode
  pub(super) fn on_key(&mut self, k: Key<&str>, t: Option<&str>) {
    if self.zoom_key(&k) {
      return;
    }
    if self.form.is_some() {
      if self.paste_key(&k) {
        return;
      }
      return self.form_key(k, t);
    }
    if self.mods.control_key() && k == Key::Character(",") {
      return self.toggle_screen(screen::Page::Settings);
    }
    if self.mods.control_key() && k == Key::Character("b") {
      return self.toggle_following();
    }
    if k == Key::Named(NamedKey::F1) {
      return self.toggle_screen(screen::Page::Keys);
    }
    if self.screen.is_some() {
      return self.screen_key(k, t);
    }
    if self.img_view.is_some() {
      return self.img_key(k);
    }
    if self.mods.control_key() && k == Key::Character(";") {
      return self.toggle_picker();
    }
    if self.paste_key(&k) {
      return;
    }
    if self.prompt.is_some() {
      return self.prompt_key(k, t);
    }
    if self.picker.is_some() {
      return self.picker_key(k, t);
    }
    if self.pop().is_some() {
      return self.pop_key(k);
    }
    if self.card().is_some() {
      return self.card_key(k);
    }
    if self.tab_menu().is_some() {
      return self.tab_menu_key(k);
    }
    if self.rover.is_some() {
      return self.rover_key(k);
    }
    if self.menu().is_some() {
      return self.menu_key(k);
    }
    if self.logs_key(k.clone()) {
      return;
    }
    if self.login_esc(&k) {
      return;
    }
    match self.mode {
      Mode::Insert => self.key_insert(k, t),
      Mode::Normal => self.key_normal(k),
    }
  }

  // vi normal mode: a picked line's keys first, then the space leader's chat commands, then vim on the box
  pub(super) fn key_normal(&mut self, key: Key<&str>) {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    if self.global_key(key.clone()) {
      return;
    }
    if self.chord.take() == Some(' ') {
      self.leader_key(key);
      return self.wake();
    }
    if self.selecting() && self.pick_key(key.clone()) {
      return;
    }
    let idle = self.vi.idle();
    let half = self.chat_area().h / 2;
    match key {
      Key::Named(NamedKey::Space) if idle && !ctrl && !alt => {
        self.chord = Some(' ');
        return self.wake();
      }
      Key::Character("t" | "T") if self.vi.g_pending() => {
        self.vi.abort();
        return self.select(self.step_tab(self.active, key == Key::Character("T")));
      }
      Key::Character("v") if idle && !ctrl => return self.select_start(),
      Key::Character("?") if idle => return self.open_screen(screen::Page::Keys),
      // vim's own / n N search the box, which is one line: here they search the chat
      Key::Character("/") if idle && !ctrl => return self.find_open(),
      Key::Character("n") if idle && !ctrl && self.find_on() => return self.find_step(1),
      Key::Character("N") if idle && !ctrl && self.find_on() => return self.find_step(-1),
      Key::Character("d") if ctrl => return self.scroll(-half),
      Key::Character("u") if ctrl => return self.scroll(half),
      Key::Character("w") if ctrl => return self.close_tab(),
      Key::Named(NamedKey::Escape) if idle && self.feed_back() => return,
      _ => {}
    }
    let name = match key {
      Key::Character("r") if ctrl => "<c-r>",
      _ if ctrl || alt => return,
      Key::Character(c) => c,
      Key::Named(NamedKey::Space) => " ",
      Key::Named(NamedKey::Escape) => "Esc",
      Key::Named(NamedKey::Enter) => "Enter",
      Key::Named(NamedKey::Backspace) => "Backspace",
      Key::Named(NamedKey::Delete) => "Delete",
      Key::Named(NamedKey::ArrowUp) => "Up",
      Key::Named(NamedKey::ArrowDown) => "Down",
      Key::Named(NamedKey::ArrowLeft) => "Left",
      Key::Named(NamedKey::ArrowRight) => "Right",
      Key::Named(NamedKey::Home) => "Home",
      Key::Named(NamedKey::End) => "End",
      _ => return,
    };
    self.sync_overlays();
    let act = self.vi.key(&mut self.line, name);
    if let Some(y) = self.vi.take_fresh() {
      self.copy(&y, "");
    }
    match act {
      Action::None => {}
      Action::Insert => self.mode = Mode::Insert,
      Action::Send => {
        self.mode = Mode::Insert;
        self.send();
      }
      Action::HistoryOlder => {
        self.line.older();
        Vi::settle(&mut self.line);
        self.chip_recalled();
      }
      Action::HistoryNewer => {
        self.line.newer();
        Vi::settle(&mut self.line);
        self.chip_recalled();
      }
      Action::Leave => {
        if !self.find_clear() {
          self.reply = None;
          self.edit_end();
        }
      }
    }
    self.complete = None;
    self.input_dirty = true;
    self.wake();
  }

  // `r` on the selected line: reply to it on its own platform
  pub(super) fn reply_selected(&mut self) {
    self.reply_to(None);
    // the hover button is off until asked for: said once per run, after a reply from the menu or the key
    if self.reply.is_some() && !self.set.hover_reply && !std::mem::replace(&mut self.reply_nudged, true) {
      self.say("replying \u{b7} want a reply button on hover? /hoverreply turns it on");
    }
  }

  // reply to the line `seq` (None: the picked one) on its own platform: the line menu's reply, and the
  // hover button's
  pub(super) fn reply_to(&mut self, seq: Option<i64>) {
    let Some(t) = self.tabs.get_mut(self.active).filter(|t| t.kind == Kind::Chat) else { return };
    let Some(m) = (match seq {
      Some(s) => t.view.message(s),
      None => t.view.selected(),
    }) else {
      return;
    };
    if m.id.is_empty() {
      return self.say("that line can't be replied to");
    }
    let p = m.platform;
    let reply = Reply { platform: p, id: m.id.clone(), nick: m.nick.clone(), text: m.body.clone() };
    if let Some(i) = t.sources.iter().position(|s| s.0 == p) {
      t.target = i;
    }
    self.reply = Some(reply);
    self.enter_insert();
  }

  pub(super) fn select_start(&mut self) {
    let h = self.chat_area().h;
    let res = &mut self.res;
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.select_start(res, h);
    }
    self.wake();
  }

  pub(super) fn select_move(&mut self, by: i32) {
    let h = self.chat_area().h;
    let res = &mut self.res;
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.select_move(by, res, h);
    }
    self.wake();
  }

  pub(super) fn selecting(&self) -> bool {
    self.tabs.get(self.active).is_some_and(|t| t.view.selected().is_some() || t.view.picked_run())
  }

  // alt-r: the pick cursor without vi (the ext's row mode). the keys are a picked line's own (`pick_key`); letters
  // that mean nothing there are swallowed so they never land in the box; esc, or any way of letting go, ends it
  pub(super) fn row_mode(&mut self) {
    if self.set.vi && self.mode == Mode::Insert {
      self.leave_insert();
    }
    self.select_start();
    self.rows = self.selecting() && !self.set.vi;
  }

  fn row_key(&mut self, key: &Key<&str>) -> bool {
    if !self.rows {
      return false;
    }
    if !self.selecting() {
      self.rows = false;
      return false;
    }
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    // g / G / home / end: first and last line, as the ext's row mode
    let edge = match key {
      Key::Character("g") | Key::Named(NamedKey::Home) => Some(true),
      Key::Character("G") | Key::Named(NamedKey::End) => Some(false),
      _ => None,
    };
    if let Some(first) = edge.filter(|_| !ctrl && !alt) {
      let h = self.chat_area().h;
      let res = &mut self.res;
      if let Some(t) = self.tabs.get_mut(self.active) {
        t.view.select_edge(first, res, h);
      }
      self.wake();
      return true;
    }
    let mut took = self.pick_key(key.clone());
    // enter on a plain chat line opens its menu, as in the ext
    if !took && *key == Key::Named(NamedKey::Enter) && self.kind() == Kind::Chat {
      self.line_menu_key();
      took = true;
    }
    self.rows = self.selecting();
    took || (matches!(key, Key::Character(_) | Key::Named(NamedKey::Space)) && !ctrl && !alt)
  }

  // a picked line's keys: j/k move it, r replies, esc lets go. false = not one of them (vi gets it)
  pub(super) fn pick_key(&mut self, key: Key<&str>) -> bool {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) if !ctrl => self.select_move(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) if !ctrl => self.select_move(-1),
      Key::Named(NamedKey::Enter) if self.view().is_some_and(|v| v.picked_run()) => {
        if let Some(v) = self.view() {
          v.toggle_run_picked();
        }
        self.wake();
      }
      Key::Named(NamedKey::Enter) if self.kind() == Kind::Mentions => self.jump_to_mention(),
      Key::Named(NamedKey::Enter) if self.feedish() => self.feed_activate(),
      Key::Character("u") if !ctrl && self.feedish() => self.feed_vote(),
      Key::Character("r") if !ctrl && self.feedish() => self.feed_reply(),
      Key::Character("h") | Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Backspace)
        if !ctrl && self.feed_back() => {}
      Key::Named(NamedKey::Enter) if self.kind() == Kind::Whispers => self.whisper_activate(),
      Key::Character("r") if !ctrl && self.kind() == Kind::Whispers => self.whisper_reply_selected(),
      Key::Character("x") if !ctrl && self.kind() == Kind::Whispers => self.dm_delete_ask(),
      Key::Character("t") if self.chain_wanted(&key) => self.chain_key(),
      Key::Character("r") if !ctrl => self.reply_selected(),
      Key::Character("m") if !ctrl => self.line_menu_key(),
      Key::Character("c") if !ctrl => self.card_selected(),
      Key::Character("o") if !ctrl && !alt => self.open_selected_link(),
      Key::Character("c") if ctrl => self.copy_selected(),
      Key::Named(NamedKey::Escape) | Key::Character("v") if !ctrl => {
        if let Some(v) = self.view() {
          v.deselect();
        }
        self.wake();
      }
      _ => return false,
    }
    true
  }

  // space then a key (the ext's leader): chat commands that vim's keys would otherwise take
  pub(super) fn leader_key(&mut self, key: Key<&str>) {
    let Key::Character(c) = key else { return };
    match c {
      "/" => self.find_open(),
      "n" if self.find_on() => self.find_step(1),
      "N" if self.find_on() => self.find_step(-1),
      "]" => self.select(self.step_tab(self.active, false)),
      "[" => self.select(self.step_tab(self.active, true)),
      "s" => self.streamer_reveal(),
      "x" => self.pin_x(),
      "p" => self.pred_here(),
      "t" => self.rover_open(),
      "w" => self.watch_here(),
      "g" => {
        let res = &mut self.res;
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.scroll_to_top(res);
        }
      }
      "G" => {
        if let Some(v) = self.view() {
          v.scroll_to_bottom();
        }
      }
      "r" => {
        let res = &mut self.res;
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.jump_read(res);
        }
      }
      "R" => {
        if let Some(v) = self.view() {
          v.clear_read();
        }
      }
      d if d.len() == 1 && (b'1'..=b'9').contains(&d.as_bytes()[0]) => {
        if let Some(i) = self.nth_visible((d.as_bytes()[0] - b'1') as usize) {
          self.select(i);
        }
      }
      _ => {}
    }
  }
}
