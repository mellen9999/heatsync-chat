// adding, editing and removing tabs while running: the + button / ctrl-t form, /join, /part, the
// right-click menu on a tab. every change joins or leaves live and is saved at once
use winit::keyboard::{Key, NamedKey};

use super::{App, SCROLLBACK, Tab};
use crate::model::chan::{Cmd as ChanCmd, Form, Spec};
use crate::model::message::Platform;
use crate::model::tabs::Kind;
use crate::net::Cmd;
use crate::net::hs_ws::Cmd as HsCmd;
use crate::ui::canvas::Rect;
use crate::ui::chat_view::ChatView;
use crate::ui::form::{self, Hit};
use crate::ui::menu::Menu;
use crate::ui::union;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabAct {
  Edit,
  Remove,
  // a built-in tab leaves the bar
  Hide,
}

pub struct TabMenu {
  menu: Menu<TabAct>,
  tab: usize,
}

// start listening to one feed (what boot and the form both do)
pub(super) fn join(net: &crate::net::Net, p: Platform, key: &str) {
  let key: Box<str> = key.into();
  match p {
    Platform::Twitch => net.send(Cmd::Join(key)),
    Platform::Kick => {
      net.send(Cmd::Hs(HsCmd::Join(Platform::Kick, key.clone())));
      net.send(Cmd::Backfill(Platform::Kick, key.clone(), key));
    }
    Platform::Youtube => net.send(Cmd::Hs(HsCmd::YtSub(key))),
  }
}

pub(super) fn sources_of(s: &Spec) -> Vec<(Platform, Box<str>)> {
  let mut v = Vec::with_capacity(3);
  if !s.twitch.is_empty() {
    v.push((Platform::Twitch, s.twitch.as_str().into()));
  }
  if !s.kick.is_empty() {
    v.push((Platform::Kick, s.kick.as_str().into()));
  }
  if !s.youtube.is_empty() {
    v.push((Platform::Youtube, s.youtube.as_str().into()));
  }
  v
}

pub(super) fn spec_of(t: &Tab) -> Spec {
  let key = |p| t.sources.iter().find(|s| s.0 == p).map_or("", |s| &*s.1);
  Spec::lenient(key(Platform::Twitch), key(Platform::Kick), key(Platform::Youtube))
}

pub(super) fn make_tab(net: &crate::net::Net, spec: &Spec) -> Tab {
  let sources = sources_of(spec);
  for (p, key) in &sources {
    join(net, *p, key);
  }
  let first = sources.first().map_or(Box::from(""), |s| s.1.clone());
  let mut view = ChatView::new(SCROLLBACK, first);
  view.tags = tags(&sources);
  view.filtered = true;
  Tab { name: spec.name().into(), kind: Kind::Chat, view, unread: false, sources, target: 0 }
}

// source tags show on merged tabs and on anything not plain twitch
fn tags(s: &[(Platform, Box<str>)]) -> bool {
  s.len() > 1 || s.first().is_some_and(|f| f.0 != Platform::Twitch)
}

// pull the last key out of a line editor: the keys every text box shares
pub(super) fn edit_line(l: &mut crate::model::line::Line, key: &Key<&str>, ctrl: bool) -> bool {
  match key {
    Key::Named(NamedKey::Backspace) if ctrl => l.delete_word_back(),
    Key::Named(NamedKey::Backspace) => l.backspace(),
    Key::Named(NamedKey::Delete) => l.delete(),
    Key::Named(NamedKey::ArrowLeft) => l.left(),
    Key::Named(NamedKey::ArrowRight) => l.right(),
    Key::Named(NamedKey::Home) => l.home(),
    Key::Named(NamedKey::End) => l.end(),
    Key::Character("a") if ctrl => l.home(),
    Key::Character("e") if ctrl => l.end(),
    Key::Character("w") if ctrl => l.delete_word_back(),
    Key::Character("u") if ctrl => l.clear_to_start(),
    Key::Character("k") if ctrl => l.clear_to_end(),
    _ => return false,
  }
  true
}

impl App {
  // every other real tab's feeds (the tab being edited, if any, is left out)
  fn other_specs(&self, skip: Option<usize>) -> Vec<Spec> {
    (0..self.tabs.len())
      .filter(|&i| Some(i) != skip && !self.tabs[i].sources.is_empty())
      .map(|i| spec_of(&self.tabs[i]))
      .collect()
  }

  pub(super) fn save_tabs(&self) {
    let lines: Vec<Box<str>> =
      self.tabs.iter().map(|t| spec_of(t).line()).filter(|l| !l.is_empty()).map(Into::into).collect();
    super::persist(&lines);
  }

  fn activate(&mut self, i: usize) {
    self.find_clear();
    self.active = i;
    if let Some(t) = self.tabs.get_mut(i) {
      t.unread = false;
      t.view.invalidate();
    }
    self.bar_dirty = true;
    self.input_dirty = true;
    self.wake();
  }

  // stop listening to a feed, unless another tab still merges it
  pub(super) fn leave(&mut self, p: Platform, key: Box<str>) {
    if self.tabs.iter().any(|o| o.sources.iter().any(|s| s.0 == p && s.1 == key)) {
      return;
    }
    self.pins.forget(p, &key);
    match p {
      Platform::Twitch => {
        self.res.emotes.drop_channel(&key);
        self.res.badges.drop_channel(&key);
        self.rooms.retain(|r| r.0 != key);
        self.mod_in.remove(&key);
        self.net.send(Cmd::Part(key));
      }
      Platform::Kick => self.net.send(Cmd::Hs(HsCmd::Leave(Platform::Kick, key))),
      Platform::Youtube => self.net.send(Cmd::Hs(HsCmd::YtUnsub(key))),
    }
  }

  pub(super) fn remove_tab(&mut self, i: usize) {
    if i >= self.tabs.len() {
      return;
    }
    let t = self.tabs.remove(i);
    for (p, key) in t.sources {
      self.leave(p, key);
    }
    if i < self.active {
      self.active -= 1;
    }
    self.active = self.active.min(self.tabs.len().saturating_sub(1));
    self.save_tabs();
    if !self.tabs.iter().any(|t| t.kind == Kind::Chat) {
      self.status = "no channels — click + or press ctrl-t to add one".into();
    }
    self.fix_active();
    self.activate(self.active);
  }

  fn add_tab(&mut self, spec: &Spec) {
    let tab = make_tab(&self.net, spec);
    self.tabs.push(tab);
    self.status.clear();
    self.save_tabs();
    self.activate(self.tabs.len() - 1);
  }

  fn edit_tab(&mut self, i: usize, spec: &Spec) {
    let new = sources_of(spec);
    for (p, key) in &new {
      if !self.tabs[i].sources.iter().any(|s| s.0 == *p && s.1 == *key) {
        join(&self.net, *p, key);
      }
    }
    let old = std::mem::replace(&mut self.tabs[i].sources, new.clone());
    let t = &mut self.tabs[i];
    let first = new.first().map_or(Box::from(""), |s| s.1.clone());
    t.name = spec.name().into();
    t.target = 0;
    if t.view.channel != first {
      t.view = ChatView::new(SCROLLBACK, first);
      t.view.filtered = true;
    }
    t.view.tags = tags(&new);
    t.view.relayout();
    t.view.invalidate();
    for (p, key) in old {
      if !new.iter().any(|s| s.0 == p && s.1 == key) {
        self.leave(p, key);
      }
    }
    self.save_tabs();
    self.bar_dirty = true;
    self.input_dirty = true;
    self.wake();
  }

  // /join and /part typed in the box
  pub(super) fn chan_cmd(&mut self, cmd: ChanCmd) {
    match cmd {
      ChanCmd::Join(None) => self.open_form(None),
      ChanCmd::Join(Some(words)) => {
        match Spec::from_line(&words).and_then(|s| s.check(&self.other_specs(None)).map(|_| s)) {
          Ok(s) => self.add_tab(&s),
          Err(e) => self.say(e.msg()),
        }
      }
      ChanCmd::Part if self.tabs.get(self.active).is_some_and(|t| !t.sources.is_empty()) => {
        self.remove_tab(self.active)
      }
      ChanCmd::Part => self.say("no channel to leave"),
    }
  }

  // ---- the form ----

  pub(super) fn open_form(&mut self, edit: Option<usize>) {
    let spec = match edit {
      Some(i) => match self.tabs.get(i) {
        Some(t) if !t.sources.is_empty() => spec_of(t),
        _ => return,
      },
      None => Spec::default(),
    };
    if self.picker.is_some() {
      self.close_picker();
    }
    self.close_prompt();
    self.menu = None;
    self.tab_menu = None;
    self.form = Some(Form::new(edit, &spec));
    self.form_hover = None;
    self.bar_dirty = true;
    self.input_dirty = true;
    self.wake();
  }

  pub(super) fn close_form(&mut self) {
    self.form = None;
    self.ime.clear();
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.input_dirty = true;
    self.wake();
  }

  fn submit_form(&mut self) {
    let Some(f) = &self.form else { return };
    let (edit, res) = (f.edit, f.submit(&self.other_specs(f.edit)));
    match res {
      Ok(spec) => {
        self.close_form();
        match edit {
          Some(i) if i < self.tabs.len() => self.edit_tab(i, &spec),
          _ => self.add_tab(&spec),
        }
      }
      Err(e) => {
        if let Some(f) = &mut self.form {
          f.err = Some(e.msg());
        }
        self.wake();
      }
    }
  }

  pub(super) fn form_key(&mut self, key: Key<&str>, text: Option<&str>) {
    let (ctrl, shift, alt) = (self.mods.control_key(), self.mods.shift_key(), self.mods.alt_key());
    let Some(f) = self.form.as_mut() else { return };
    match key {
      Key::Named(NamedKey::Escape) => return self.close_form(),
      Key::Named(NamedKey::Enter) => return self.submit_form(),
      Key::Named(NamedKey::Tab) => f.cycle(shift),
      Key::Named(NamedKey::ArrowDown) => f.cycle(false),
      Key::Named(NamedKey::ArrowUp) => f.cycle(true),
      _ if edit_line(&mut f.fields[f.focus], &key, ctrl) => {}
      Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
        if let Some(t) = text.filter(|_| self.ime.accepts_key_text()) {
          f.fields[f.focus].insert(t);
        }
      }
      _ => return,
    }
    f.err = None;
    self.wake();
  }

  fn form_geo(&mut self) -> form::Geo {
    let em = self.res.text.layout("0", &[], &[], f32::MAX).full_width().ceil() as i32;
    form::geo(self.chat_area(), self.res.text.line_px as i32, em)
  }

  pub(super) fn form_at(&mut self) -> Option<Hit> {
    let g = self.form_geo();
    form::hit(&g, self.pointer.0, self.pointer.1)
  }

  pub(super) fn form_hover_move(&mut self) {
    if self.form.is_none() {
      return;
    }
    let h = self.form_at();
    if h != self.form_hover {
      self.form_hover = h;
      self.wake();
    }
  }

  pub(super) fn form_click(&mut self) {
    match self.form_at() {
      Some(Hit::Field(i)) => {
        if let Some(f) = &mut self.form {
          f.focus = i;
        }
        self.wake();
      }
      Some(Hit::Ok) => self.submit_form(),
      Some(Hit::Cancel) => self.close_form(),
      None => {}
    }
  }

  pub(super) fn draw_form(&mut self) -> Option<Rect> {
    let area = self.chat_area();
    let f = self.form.as_ref()?;
    let title = match f.edit.and_then(|i| self.tabs.get(i)) {
      Some(t) => format!("edit {}", t.name),
      None => "add channel".to_owned(),
    };
    let (caret, _) = form::draw(&mut self.canvas, &mut self.res.text, area, f, &title, self.form_hover);
    self.ime_area = Some(caret);
    Some(area)
  }

  // ---- the tab bar ----

  // position in the drawn bar: shown tabs in order, then the + button, then the gear
  fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
    self.bar_hits.iter().position(|r| r.contains(x, y))
  }

  pub(super) fn gear_at(&self) -> bool {
    self.tab_at(self.pointer.0, self.pointer.1) == Some(self.bar_map.len() + 1)
  }

  pub(super) fn bar_hover(&mut self) {
    let (x, y) = self.pointer;
    let at = self.tab_at(x, y);
    let (plus, gear) = (at == Some(self.bar_map.len()), at == Some(self.bar_map.len() + 1));
    if (plus, gear) != (self.plus_hover, self.gear_hover) {
      (self.plus_hover, self.gear_hover) = (plus, gear);
      self.bar_dirty = true;
      self.wake();
    }
  }

  pub(super) fn bar_click(&mut self) {
    match self.tab_at(self.pointer.0, self.pointer.1) {
      Some(p) if p == self.bar_map.len() => self.open_form(None),
      Some(p) if p == self.bar_map.len() + 1 => self.open_screen(super::screen::Page::Settings),
      Some(p) => self.select(self.bar_map[p]),
      None => {}
    }
  }

  pub(super) fn bar_right_click(&mut self) {
    let (x, y) = self.pointer;
    let Some(p) = self.tab_at(x, y).filter(|&p| p < self.bar_map.len()) else { return };
    let i = self.bar_map[p];
    let at = (self.bar_hits[p].x, self.bar().bottom());
    let items = match self.tabs[i].kind {
      Kind::Chat if self.tabs[i].sources.is_empty() => return,
      Kind::Chat => vec![(TabAct::Edit, "edit"), (TabAct::Remove, "remove")],
      _ => vec![(TabAct::Hide, "hide")],
    };
    self.tab_menu = Some(TabMenu { menu: Menu::new(items, at), tab: i });
    self.tab_menu_changed();
  }

  pub(super) fn tab_menu_changed(&mut self) {
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.bar_dirty = true;
    self.wake();
  }

  fn tab_menu_pick(&mut self) {
    let Some(m) = self.tab_menu.take() else { return };
    self.tab_menu_changed();
    match m.menu.chosen() {
      Some(TabAct::Edit) => self.open_form(Some(m.tab)),
      Some(TabAct::Remove) => self.remove_tab(m.tab),
      Some(TabAct::Hide) => {
        if let Some(k) = self.tabs.get(m.tab).map(|t| t.kind) {
          self.set_shown(k, false);
        }
      }
      None => {}
    }
  }

  pub(super) fn tab_menu_key(&mut self, key: Key<&str>) {
    let Some(m) = &mut self.tab_menu else { return };
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => m.menu.step(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => m.menu.step(-1),
      Key::Named(NamedKey::Enter) => return self.tab_menu_pick(),
      Key::Named(NamedKey::Escape) => self.tab_menu = None,
      _ => return,
    }
    self.tab_menu_changed();
  }

  pub(super) fn tab_menu_click(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    let row = self.tab_menu.as_ref().and_then(|m| m.menu.row_at(x, y, line));
    match (row, &mut self.tab_menu) {
      (Some(i), Some(m)) => {
        m.menu.at = i;
        self.tab_menu_pick();
      }
      _ => {
        self.tab_menu = None;
        self.tab_menu_changed();
      }
    }
  }

  pub(super) fn tab_menu_hover(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    if let Some(m) = &mut self.tab_menu
      && let Some(i) = m.menu.row_at(x, y, line)
      && i != m.menu.at
    {
      m.menu.at = i;
      self.wake();
    }
  }

  pub(super) fn draw_tab_menu(&mut self) -> Option<Rect> {
    let bounds = Rect::new(0, 0, self.canvas.w, (self.canvas.h - self.input_h).max(0));
    let m = self.tab_menu.as_mut()?;
    let r = m.menu.draw(&mut self.canvas, &mut self.res.text, bounds);
    Some(union(r, r))
  }
}
