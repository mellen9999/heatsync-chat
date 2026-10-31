// the one floating menu for choices that are not about an emote or a tab: the login chooser (ctrl-l)
// and, further on, the actions on a chat line. keyboard j/k/enter/esc and the mouse both drive it
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::filter::{self, DEFAULT_MUTE_MS};
use crate::model::message::{Kind as MsgKind, Platform};
use crate::model::modcmd::{DURATIONS, Verb};
use crate::model::tabs::Kind;
use crate::net::Cmd;
use crate::net::auth::Provider;
use crate::ui::canvas::Rect;
use crate::ui::menu::Menu;
use crate::util::time::now_ms;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
  Login(Provider),
  // a mod action on the person the menu was opened for
  Do(Verb),
  // the answers to "ban bob?"
  Yes,
  No,
  Reply,
  CopyLine,
  // the person's card, and the things a card (or a line menu) does to them
  Card,
  Mention,
  Whisper,
  Mute,
  Unmute,
  Block,
  Unblock,
}

// the person a line menu acts on, from the line it was opened on
#[derive(Clone, Debug)]
pub struct Subject {
  pub platform: Platform,
  // twitch login / kick slug of the room
  pub chan: Box<str>,
  pub login: Box<str>,
  pub nick: Box<str>,
  pub user_id: Box<str>,
  pub msg_id: Box<str>,
  pub color: u32,
  // one of our own accounts: no mod rows
  pub own: bool,
}

// a mod action waiting for "yes"
pub struct Todo {
  pub verb: Verb,
  pub who: Vec<Subject>,
  pub reason: Option<String>,
}

pub struct Pop {
  menu: Menu<Act>,
  subject: Option<Subject>,
  todo: Option<Todo>,
}

impl Pop {
  pub fn confirm(head: String, at: (i32, i32), todo: Todo) -> Self {
    let rows = vec![(Act::Yes, "yes, ban".to_owned()), (Act::No, "cancel".to_owned())];
    Pop { menu: Menu::new(rows, at).titled(head), subject: None, todo: Some(todo) }
  }
}

// the chooser rows, plain words
fn login_rows() -> Vec<(Act, String)> {
  [Provider::Twitch, Provider::Kick, Provider::Youtube]
    .into_iter()
    .map(|p| (Act::Login(p), p.name().to_owned()))
    .collect()
}

impl App {
  // ctrl-l: logged in just says who; logged out asks which platform
  pub(super) fn login(&mut self) {
    if let Some(me) = &self.me {
      let on = me.linked();
      let on = if on.is_empty() { String::new() } else { format!(" · {on}") };
      self.status = format!("logged in as {}{on} (ctrl-shift-l logs out)", me.username);
      self.bar_dirty = true;
      return self.wake();
    }
    let at = (self.canvas.w / 2 - 80, self.bar().bottom());
    self.pop = Some(Pop { menu: Menu::new(login_rows(), at).titled("log in with"), subject: None, todo: None });
    self.pop_changed();
  }

  pub(super) fn pop_changed(&mut self) {
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.bar_dirty = true;
    self.wake();
  }

  fn pop_pick(&mut self) {
    let Some(p) = self.pop.take() else { return };
    self.pop_changed();
    if let Some(act) = p.menu.chosen() {
      self.act(act, p.subject, p.todo);
    }
  }

  // what a chosen menu row does; shared by the line menu, the card and the login chooser
  pub(super) fn act(&mut self, act: Act, who: Option<Subject>, todo: Option<Todo>) {
    match (act, who, todo) {
      (Act::Login(provider), ..) => {
        self.status = format!("finish logging in with {} in your browser…", provider.name());
        self.net.send(Cmd::Login(provider));
      }
      (Act::Do(verb), Some(who), _) => self.mod_run(verb, vec![who], None),
      (Act::Yes, _, Some(t)) => self.mod_send(t.verb, t.who, t.reason),
      (Act::No, ..) => self.say("cancelled"),
      (Act::Reply, ..) => self.reply_selected(),
      (Act::CopyLine, ..) => self.copy_selected(),
      (Act::Card, Some(who), _) => self.open_card(who),
      (Act::Mention, Some(who), _) => self.type_into_line(&format!("@{} ", who.nick)),
      (Act::Whisper, Some(who), _) => self.type_into_line(&format!("/w {} ", who.login)),
      (Act::Mute, Some(who), _) => self.mute(&who.login, Some(now_ms() + DEFAULT_MUTE_MS)),
      (Act::Unmute, Some(who), _) => self.unmute(&who.login),
      (Act::Block, Some(who), _) => self.block(&who.login),
      (Act::Unblock, Some(who), _) => self.unblock(&who.login),
      _ => {}
    }
  }

  // start (or add to) the message with some text and put the cursor in the box
  fn type_into_line(&mut self, text: &str) {
    self.line.insert(text);
    self.complete = None;
    self.enter_insert();
    self.input_dirty = true;
    self.wake();
  }

  // the rows for a person: the filters first, then what a moderator can do. shared by the line menu and the card
  pub(super) fn person_rows(&self, w: &Subject) -> Vec<(Act, String)> {
    let mut rows: Vec<(Act, String)> = Vec::new();
    if w.own {
      return rows;
    }
    rows.push((Act::Mention, "mention".into()));
    if w.platform == Platform::Twitch && self.me.is_some() {
      rows.push((Act::Whisper, "whisper".into()));
    }
    let key = filter::key(&w.login);
    let (muted, blocked) = filter::with(|f| (f.muted(&key, now_ms()), f.blocked(&key)));
    rows.push(if muted { (Act::Unmute, "unmute".into()) } else { (Act::Mute, "mute".into()) });
    rows.push(if blocked { (Act::Unblock, "unblock".into()) } else { (Act::Block, "block".into()) });
    if self.can_mod(w.platform, &w.chan) {
      if !w.msg_id.is_empty() {
        rows.push((Act::Do(Verb::Delete), "delete message".into()));
      }
      for (label, secs) in DURATIONS {
        rows.push((Act::Do(Verb::Timeout(secs)), format!("timeout {label}")));
      }
      rows.push((Act::Do(Verb::Purge), "purge".into()));
      rows.push((Act::Do(Verb::Ban), "ban".into()));
      rows.push((Act::Do(Verb::Unban), "unban".into()));
    }
    rows
  }

  // the menu for the selected line: its own actions, then mod actions when we moderate the room
  pub(super) fn open_line_menu(&mut self, at: (i32, i32)) {
    let Some(t) = self.tabs.get(self.active).filter(|t| matches!(t.kind, Kind::Chat | Kind::Mentions)) else { return };
    let Some(m) = t.view.selected().filter(|m| m.kind != MsgKind::System) else { return };
    let who = self.subject_of(m);
    let mut rows: Vec<(Act, String)> = Vec::new();
    if who.is_some() {
      rows.push((Act::Card, "user card".into()));
    }
    if !m.id.is_empty() && t.kind == Kind::Chat {
      rows.push((Act::Reply, "reply".into()));
    }
    rows.push((Act::CopyLine, "copy line".into()));
    if let Some(w) = &who {
      rows.extend(self.person_rows(w));
    }
    let head = m.nick.to_string();
    self.pop = Some(Pop { menu: Menu::new(rows, at).titled(head), subject: who, todo: None });
    self.pop_changed();
  }

  // right-click on a line (not an emote): select it and open its menu
  pub(super) fn line_right_click(&mut self) {
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    if !chat.contains(x, y) {
      return;
    }
    let res = &mut self.res;
    if self.tabs.get_mut(self.active).is_some_and(|t| t.view.select_at(chat, y, res)) {
      self.open_line_menu((x, y));
    }
  }

  // `m` on a selected line
  pub(super) fn line_menu_key(&mut self) {
    let chat = self.chat_area();
    self.open_line_menu((chat.x + 24, chat.y + chat.h / 4));
  }

  pub(super) fn pop_key(&mut self, key: Key<&str>) {
    let Some(p) = &mut self.pop else { return };
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => p.menu.step(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => p.menu.step(-1),
      Key::Named(NamedKey::Enter) => return self.pop_pick(),
      Key::Named(NamedKey::Escape) => self.pop = None,
      _ => return,
    }
    self.pop_changed();
  }

  pub(super) fn pop_click(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    let row = self.pop.as_ref().and_then(|p| p.menu.row_at(x, y, line));
    match (row, &mut self.pop) {
      (Some(i), Some(p)) => {
        p.menu.at = i;
        self.pop_pick();
      }
      _ => {
        self.pop = None;
        self.pop_changed();
      }
    }
  }

  pub(super) fn pop_hover(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    if let Some(p) = &mut self.pop
      && let Some(i) = p.menu.row_at(x, y, line)
      && i != p.menu.at
    {
      p.menu.at = i;
      self.wake();
    }
  }

  pub(super) fn draw_pop(&mut self) -> Option<Rect> {
    let bounds = Rect::new(0, 0, self.canvas.w, (self.canvas.h - self.input_h).max(0));
    let p = self.pop.as_mut()?;
    Some(p.menu.draw(&mut self.canvas, &mut self.res.text, bounds))
  }
}
