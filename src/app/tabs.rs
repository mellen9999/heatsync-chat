// adding, editing and removing tabs while running: the + button / ctrl-t form, /join, /part, the
// right-click menu on a tab. every change joins or leaves live and is saved at once
use winit::keyboard::{Key, NamedKey};

use super::overlay::Overlay;
use super::screen::Page;
use super::{App, SCROLLBACK, Tab};
use crate::model::chan::{Act, Cmd as ChanCmd, Form, Spec};
use crate::model::layout_link;
use crate::model::message::Platform;
use crate::model::settings::Side;
use crate::model::tabs::Kind;
use crate::net::Cmd;
use crate::net::hs_ws::Cmd as HsCmd;
use crate::ui::canvas::Rect;
use crate::ui::chat_view::ChatView;
use crate::ui::form::{self, Hit};
use crate::ui::menu::Menu;
use crate::ui::union;

// the buttons after the tabs, in the order they are drawn
#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
  Plus,
  Gear,
  Keys,
  Update,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabAct {
  Edit,
  Remove,
  // a built-in tab leaves the bar
  Hide,
}

impl TabMenu {
  #[cfg(test)]
  pub fn labels(&self) -> Vec<String> {
    self.menu.items.iter().map(|i| i.1.clone()).collect()
  }
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

// a twitch channel's owner is someone you can name in it, spoken or not
pub(super) fn seat_hosts(c: &mut crate::model::chatters::Chatters, sources: &[(Platform, Box<str>)]) {
  for (p, key) in sources {
    c.host(*p, key);
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
  Tab {
    name: spec.name().into(),
    kind: Kind::Chat,
    view,
    unread: false,
    event: false,
    sources,
    target: 0,
    under: None,
    count: Default::default(),
    stats: Default::default(),
  }
}

// source tags show on merged tabs and on anything not plain twitch
// the buttons of the edit form: watch (one per channel when merged), status and predictions for a
// twitch channel, summary
fn form_acts(src: &[(Platform, Box<str>)]) -> Vec<(Act, String)> {
  let mut v: Vec<(Act, String)> = match src.len() {
    1 => vec![(Act::Watch(0), "watch".into())],
    _ => src.iter().enumerate().map(|(n, (p, _))| (Act::Watch(n), format!("watch {}", p.name()))).collect(),
  };
  if src.iter().any(|s| s.0 == Platform::Twitch) {
    v.push((Act::Status, "status".into()));
    v.push((Act::Pred, "predictions".into()));
  }
  v.push((Act::Summary, "summary".into()));
  v
}

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

  // a change you made: saved, and sent to your account
  pub(super) fn save_tabs(&mut self) {
    self.store_tabs();
    self.tab_sync_push();
  }

  pub(super) fn store_tabs(&self) {
    let lines: Vec<Box<str>> =
      self.tabs.iter().map(|t| spec_of(t).line()).filter(|l| !l.is_empty()).map(Into::into).collect();
    super::persist(&lines);
  }

  pub(super) fn activate(&mut self, i: usize) {
    self.find_clear();
    self.active = i;
    if let Some(t) = self.tabs.get_mut(i) {
      t.unread = false;
      t.event = false;
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
    self.pin_bar.pins.forget(p, &key);
    self.pacer.flush(&key);
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
    if self.tabs[i].under.is_some() {
      self.logs = None;
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
    self.fix_active();
    self.activate(self.active);
  }

  fn add_tab(&mut self, spec: &Spec) {
    let mut tab = make_tab(&self.net, spec);
    tab.view.resize(self.body().w);
    seat_hosts(&mut self.res.chatters, &tab.sources);
    self.tabs.push(tab);
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
    seat_hosts(&mut self.res.chatters, &new);
    let old = std::mem::replace(&mut self.tabs[i].sources, new.clone());
    let w = self.body().w;
    let t = &mut self.tabs[i];
    let first = new.first().map_or(Box::from(""), |s| s.1.clone());
    t.name = spec.name().into();
    t.target = 0;
    let v = t.chat();
    if v.channel != first {
      *v = ChatView::new(SCROLLBACK, first);
      v.filtered = true;
      v.resize(w);
    }
    v.tags = tags(&new);
    v.relayout();
    v.invalidate();
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
      // a pasted /m/ link where a channel goes opens its lineup
      ChanCmd::Join(Some(words)) if layout_link::extract(&words).is_some_and(|_| words.contains("/m/")) => {
        self.open_layout(&words)
      }
      ChanCmd::Join(Some(words)) => {
        match Spec::from_line(&words).and_then(|s| s.check(&self.other_specs(None)).map(|_| s)) {
          Ok(s) => self.add_tab(&s),
          Err(e) => self.say(e.msg()),
        }
      }
      ChanCmd::Share => self.share_lineup(),
      ChanCmd::Open(Some(link)) => self.open_layout(&link),
      ChanCmd::Open(None) => self.say("paste a heatsync.org/m/ link after /layout"),
      ChanCmd::Part if self.tabs.get(self.active).is_some_and(|t| !t.sources.is_empty()) => {
        self.remove_tab(self.active)
      }
      ChanCmd::Part => self.say("no channel to leave"),
      ChanCmd::Watch => match self.tabs.get(self.active).filter(|t| !t.sources.is_empty()) {
        Some(t) => self.watch(self.active, t.target),
        None => self.say("no channel to watch here"),
      },
    }
  }

  // /share: every channel tab as one heatsync.org/m/ link, on the clipboard
  fn share_lineup(&mut self) {
    let specs = self.other_specs(None);
    let Some(l) = layout_link::link(&specs) else { return self.say("no channel tabs to share") };
    // the copy runs off the ui thread and reports itself; the line says how much of the lineup fit
    let n = layout_link::parse(l.rsplit('/').next().unwrap_or("")).len();
    let cut = if n < specs.len() { format!(" (first {n} of {})", specs.len()) } else { String::new() };
    self.copy(&l, format!("copied your lineup, {n} tab{}{cut}", if n == 1 { "" } else { "s" }));
  }

  // /open <link>: add the lineup's tabs, minus whatever is already open
  pub(super) fn open_layout(&mut self, input: &str) {
    let Some(layout) = layout_link::extract(input) else {
      return self.say("that isn't a heatsync.org/m/ link");
    };
    let (specs, skipped) = layout_link::to_specs(&layout_link::parse(&layout));
    let mut have = self.other_specs(None);
    let first = self.tabs.len();
    let mut names: Vec<String> = Vec::new();
    for spec in &specs {
      if let Some(s) = layout_link::fresh(spec, &have) {
        names.push(s.name());
        have.push(s.clone());
        let mut tab = make_tab(&self.net, &s);
        tab.view.resize(self.body().w);
        seat_hosts(&mut self.res.chatters, &tab.sources);
        self.tabs.push(tab);
      }
    }
    let note = if skipped > 0 { format!(", skipped {skipped} it can't open") } else { String::new() };
    if names.is_empty() {
      return self.say(if specs.is_empty() { format!("nothing to open there{note}") } else { "already open".into() });
    }
    self.save_tabs();
    self.activate(first);
    let n = names.len();
    self.say(format!("added {n} tab{}: {}{note}", if n == 1 { "" } else { "s" }, names.join(", ")));
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
    self.close_menu();
    self.close_tab_menu();
    let mut form = Form::new(edit, &spec);
    if let Some(t) = edit.and_then(|i| self.tabs.get(i)) {
      form.acts = form_acts(&t.sources);
    }
    self.form = Some(form);
    self.form_hover = None;
    self.bar_dirty = true;
    self.input_dirty = true;
    self.wake();
  }

  // the first window of a run with no channels asks for one right away, instead of an empty window
  pub(super) fn first_run(&mut self) {
    if !std::mem::take(&mut self.welcome) {
      return;
    }
    self.open_form(None);
    if let Some(f) = &mut self.form {
      f.welcome = true;
    }
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

  // run the focused button on the tab being edited; the form closes first, nothing is saved
  fn form_act(&mut self) {
    let Some(f) = &self.form else { return };
    let (Some(act), Some(tab)) = (f.act(), f.edit) else { return };
    let chan = self.tabs.get(tab).and_then(|t| t.sources.iter().find(|s| s.0 == Platform::Twitch)).map(|s| s.1.clone());
    self.close_form();
    match act {
      Act::Watch(n) => self.watch(tab, n),
      Act::Summary => {
        if let Some(t) = self.tabs.get(tab) {
          let name = t.name.clone();
          self.summary_open(name);
        }
      }
      Act::Pred => chan.into_iter().for_each(|c| self.pred_open(c)),
      Act::Status => chan.into_iter().for_each(|c| self.status_open(c)),
    }
  }

  pub(super) fn form_key(&mut self, key: Key<&str>, text: Option<&str>) {
    let (ctrl, shift, alt) = (self.mods.control_key(), self.mods.shift_key(), self.mods.alt_key());
    let Some(f) = self.form.as_mut() else { return };
    // on a button the letters are keys, not text: j l next, k h back, enter or space runs it
    if let (Some(_), Key::Character(c)) = (f.act(), &key)
      && !ctrl
      && !alt
    {
      match *c {
        "j" | "l" => f.cycle(false),
        "k" | "h" => f.cycle(true),
        _ => return,
      }
      self.wake();
      return;
    }
    match key {
      Key::Named(NamedKey::Escape) => return self.close_form(),
      Key::Named(NamedKey::Space) if f.act().is_some() => return self.form_act(),
      Key::Named(NamedKey::Enter) if f.act().is_some() => return self.form_act(),
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
    let n = self.form.as_ref().map_or(0, |f| f.acts.len());
    form::geo(self.chat_area(), self.res.text.line_px as i32, em, n)
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
      Some(Hit::Act(i)) => {
        if let Some(f) = &mut self.form {
          f.focus = 3 + i;
        }
        self.form_act();
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
      None if f.welcome => "add your first channel".to_owned(),
      None => "add channel".to_owned(),
    };
    let (caret, _) = form::draw(&mut self.canvas, &mut self.res.text, area, f, &title, self.form_hover);
    self.ime_win.area = Some(caret);
    Some(area)
  }

  // ---- the tab bar ----

  // where a menu opens off a bar cell: under it on top, beside it in a column, above it at the bottom
  // (menus clamp into menu_room, which lifts or shifts them clear of the bar)
  fn off_bar(&self, r: Rect) -> (i32, i32) {
    match self.set.tab_side {
      Side::Top => (r.x, r.bottom()),
      Side::Left => (r.right(), r.y),
      Side::Bottom | Side::Right => (r.x, r.y),
    }
  }

  // menus float over the chat: the space under the tabs, above the input bar
  pub(super) fn menu_room(&self) -> Rect {
    let b = self.body();
    Rect { h: (b.h - self.input_h).max(0), ..b }
  }

  // position in the drawn bar: shown tabs in order, then the + button, the gear, the ? and the update label
  fn tab_at(&self, x: i32, y: i32) -> Option<usize> {
    self.bar_hits.iter().position(|r| r.contains(x, y))
  }

  // the button under the pointer
  fn bar_button(&self) -> Option<Button> {
    let p = self.tab_at(self.pointer.0, self.pointer.1)?.checked_sub(self.bar_map.len())?;
    [Button::Plus, Button::Gear, Button::Keys, Button::Update].get(p).copied()
  }

  // a click on the gear or the ? with a screen open: its own screen closes, the other one takes its place.
  // false = not a click on either
  pub(super) fn bar_screen_click(&mut self) -> bool {
    let page = match self.bar_button() {
      Some(Button::Gear) => Page::Settings,
      Some(Button::Keys) => Page::Keys,
      _ => return false,
    };
    self.toggle_screen(page);
    true
  }

  pub(super) fn bar_hover(&mut self) {
    let b = self.bar_button();
    let hover = (b == Some(Button::Plus), b == Some(Button::Gear), b == Some(Button::Keys), b == Some(Button::Update));
    if hover != (self.hover.plus, self.hover.gear, self.hover.keys, self.hover.update) {
      (self.hover.plus, self.hover.gear, self.hover.keys, self.hover.update) = hover;
      self.bar_dirty = true;
      self.wake();
    }
  }

  pub(super) fn bar_click(&mut self) {
    match (self.bar_button(), self.tab_at(self.pointer.0, self.pointer.1)) {
      (Some(Button::Plus), _) => self.open_form(None),
      (Some(Button::Gear), _) => self.open_screen(Page::Settings),
      (Some(Button::Keys), _) => self.open_screen(Page::Keys),
      (Some(Button::Update), _) => self.update_press(),
      // the `← back` tab: a click goes back (the picture first, then a thread)
      (_, Some(p)) if self.bar_map[p] == self.active && self.img_view.is_some() => self.img_close(),
      (_, Some(p)) if self.bar_map[p] == self.active && self.tabs[self.active].under.is_some() => {
        self.takeover_close();
      }
      (_, Some(p)) => self.select(self.bar_map[p]),
      (_, None) => {}
    }
  }

  pub(super) fn bar_right_click(&mut self) {
    let (x, y) = self.pointer;
    let Some(p) = self.tab_at(x, y).filter(|&p| p < self.bar_map.len()) else { return };
    let i = self.bar_map[p];
    if self.acknowledge(i) {
      return;
    }
    let at = self.off_bar(self.bar_hits[p]);
    let items: Vec<(TabAct, String)> = match self.tabs[i].kind {
      Kind::Chat if self.tabs[i].sources.is_empty() => return,
      Kind::Chat => vec![(TabAct::Edit, "edit".into()), (TabAct::Remove, "remove".into())],
      _ => vec![(TabAct::Hide, "hide".into())],
    };
    self.overlay = Some(Overlay::Tab(TabMenu { menu: Menu::new(items, at), tab: i }));
    self.tab_menu_changed();
  }

  // right-click on a tab calling for you (mentions red, whispers cyan, a stream line yellow) clears
  // the call without opening it; a plain unread chat tab keeps its menu (ext tab contextmenu)
  fn acknowledge(&mut self, i: usize) -> bool {
    let t = &mut self.tabs[i];
    match t.kind {
      // red and cyan go out everywhere you are logged in
      Kind::Mentions | Kind::Whispers if t.unread => {
        let s = super::unseen::surface(t.kind).expect("mentions and whispers are surfaces");
        self.unseen_clear(s);
      }
      Kind::Chat if std::mem::take(&mut t.event) => {
        self.bar_dirty = true;
        self.wake();
      }
      _ => return false,
    }
    true
  }

  pub(super) fn tab_menu_changed(&mut self) {
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.bar_dirty = true;
    self.wake();
  }

  fn tab_menu_pick(&mut self) {
    let Some(m) = self.take_tab_menu() else { return };
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
    let Some(Overlay::Tab(m)) = &mut self.overlay else { return };
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => m.menu.step(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => m.menu.step(-1),
      Key::Named(NamedKey::Enter) => return self.tab_menu_pick(),
      Key::Named(NamedKey::Escape) => self.close_tab_menu(),
      _ => return,
    }
    self.tab_menu_changed();
  }

  pub(super) fn tab_menu_click(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    let row = self.tab_menu().and_then(|m| m.menu.row_at(x, y, line));
    match (row, &mut self.overlay) {
      (Some(i), Some(Overlay::Tab(m))) => {
        m.menu.at = i;
        self.tab_menu_pick();
      }
      _ => {
        self.close_tab_menu();
        self.tab_menu_changed();
      }
    }
  }

  pub(super) fn tab_menu_hover(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    if let Some(Overlay::Tab(m)) = &mut self.overlay
      && let Some(i) = m.menu.row_at(x, y, line)
      && i != m.menu.at
    {
      m.menu.at = i;
      self.wake();
    }
  }

  pub(super) fn draw_tab_menu(&mut self) -> Option<Rect> {
    let bounds = self.menu_room();
    let Some(Overlay::Tab(m)) = &mut self.overlay else { return None };
    let r = m.menu.draw(&mut self.canvas, &mut self.res.text, bounds);
    Some(union(r, r))
  }
}
