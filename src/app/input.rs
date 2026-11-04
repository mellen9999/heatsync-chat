// the message box: its hint, its mode, sending what is typed, and the slash commands that are ours

use super::{App, LOGIN_HINT, Tag, screen};
use crate::model::message::Platform;
use crate::model::settings::{Local, local_cmd};
use crate::model::tabs::Kind;
use crate::model::vi::Mode;
use crate::model::{chan, whisper};
use crate::net::Cmd;

impl App {
  pub(super) fn hint(&self) -> String {
    if !self.tabs.iter().any(|t| t.kind == Kind::Chat) {
      return "no channels — click + or press ctrl-t to add one".into();
    }
    if self.kind() == Kind::Mentions {
      return "enter or a click opens the line in its channel".into();
    }
    if let Some(l) = self.logs_shown() {
      return l.hint();
    }
    if self.feedish() {
      return self.feed_hint();
    }
    if self.kind() == Kind::Whispers {
      return match self.whisper_label() {
        Some(to) => format!("type to {to}· /w user msg · /dm user msg"),
        None => "/w user msg to whisper · /dm user msg to dm".into(),
      };
    }
    let lead = if self.set.vi { "i type" } else { "type to chat" };
    let Some(me) = &self.me else { return format!("{LOGIN_HINT} · {lead}") };
    let Some((p, ..)) = self.target() else { return lead.into() };
    // locked out of this room: the box says why (and for how long) instead of inviting you to type
    if let Some(l) = self.lock_here() {
      return l.text(super::now_ms());
    }
    match me.name_on(p) {
      Some(name) => format!("{lead} as {name}"),
      None => format!("{} isn't linked · ctrl-l to link it", p.name()),
    }
  }

  // auto-hide: an empty bar takes no room (mode and reply stay as they are)
  pub(super) fn hidden(&self) -> bool {
    self.set.auto_hide && self.line.text.is_empty() && !self.ime.composing() && self.prompt.is_none()
  }

  // where the input sends: the tab's target source, tagged when the tab merges platforms
  pub(super) fn target(&self) -> Option<(Platform, &str, Option<Tag>)> {
    let t = self.tabs.get(self.active)?;
    let (p, key) = t.sources.get(t.target)?;
    let tag = match p {
      Platform::Twitch => ("[T] ", crate::ui::theme::TWITCH),
      Platform::Kick => ("[K] ", crate::ui::theme::GREEN),
      Platform::Youtube => ("[Y] ", crate::ui::theme::RED),
    };
    Some((*p, key, (t.sources.len() > 1).then_some(tag)))
  }

  pub(super) fn set_mode(&mut self, m: Mode) {
    self.mode = m;
    self.input_dirty = true;
    self.wake();
  }

  // vi: back to normal, the cursor stepping back onto a char
  pub(super) fn leave_insert(&mut self) {
    self.sync_overlays();
    self.vi.end_insert(&mut self.line);
    self.set_mode(Mode::Normal);
  }

  pub(super) fn enter_insert(&mut self) {
    if self.mode != Mode::Insert {
      self.vi.begin_insert(&self.line);
    }
    self.set_mode(Mode::Insert);
  }

  // after a send or local command: fresh undo history
  pub(super) fn after_send(&mut self) {
    self.vi.reset();
    if self.set.vi && self.mode == Mode::Insert {
      self.vi.begin_insert(&self.line);
    }
    self.input_dirty = true;
    self.wake();
  }

  // slash commands that are ours: switches from the schema table, zoom, tabs, the two screens. never sent anywhere
  pub(super) fn local(&mut self, cmd: Local) {
    match cmd {
      Local::Toggle(id) => {
        if let Some(d) = crate::model::schema::def(id) {
          self.flip(d);
        }
      }
      Local::Tab(Some(k), shown) => self.set_shown(k, shown),
      Local::Tab(None, shown) => {
        self.say(format!("usage: /{} feed, whispers or mentions", if shown { "show" } else { "hide" }))
      }
      Local::Zoom(Some(pct)) => self.zoom_to(pct),
      Local::Zoom(None) => self.say("usage: /zoom 1.5 (anything from 0.5 to 3)"),
      Local::Side(Some(side)) => self.side_to(side),
      Local::Side(None) => self.say("usage: /tabs top, bottom, left or right"),
      Local::Pick(id, Some(i)) => {
        if let Some(d) = crate::model::schema::def(id) {
          self.pick_to(d, i);
        }
      }
      Local::Pick(id, None) => {
        if let Some(d) = crate::model::schema::def(id)
          && let (Some(cmd), crate::model::schema::Ctl::Pick(c, ..)) = (d.cmd, d.ctl)
        {
          self.say(format!("usage: {cmd} {}", c.usage));
        }
      }
      Local::Counter(Some(c)) => self.counter_to(c),
      Local::Counter(None) => self.say("usage: /tabcounter unread, heat or off"),
      Local::Settings => self.open_screen(screen::Page::Settings),
      Local::Keys => self.open_screen(screen::Page::Keys),
      Local::Login(arg) => self.login_cmd(&arg),
      Local::Logout => self.logout_cmd(),
      Local::Account => self.account_open(),
      Local::Blocked => self.blocked_here(),
      Local::Summary => self.summary_here(),
      Local::Status => self.status_here(),
      Local::Pred => self.pred_here(),
      Local::ModTools => self.tools_here(),
      Local::Lclear => self.lclear(),
      Local::User(name) => self.user_cmd(&name),
      Local::Goto(q) => self.goto_tab(&q),
      Local::Set(k, v) => self.set_cmd(&k, &v),
      Local::Follow(name, want) => self.follow_slash(&name, want),
    }
  }

  pub(super) fn send(&mut self) {
    if self.logs_shown().is_some() {
      return self.say("you're reading chat logs: esc goes back");
    }
    if self.edit.is_some() {
      return self.edit_send();
    }
    self.faces();
    if let Some(cmd) = local_cmd(&self.line.text) {
      self.line.take();
      self.local(cmd);
      return self.after_send();
    }
    if let Some(cmd) = chan::command(&self.line.text) {
      self.line.take();
      self.chan_cmd(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::filtercmd::command(&self.line.text) {
      self.line.take();
      self.filter_cmd(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::notes::command(&self.line.text) {
      self.line.take();
      self.note_cmd(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::modes::command(&self.line.text) {
      self.line.take();
      self.modes_slash(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::modsuite::command(&self.line.text) {
      self.line.take();
      self.suite_slash(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::chantools::command(&self.line.text) {
      self.line.take();
      self.chan_tools_slash(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::modcmd::command(&self.line.text) {
      self.line.take();
      self.mod_slash(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::postcmd::command(&self.line.text) {
      self.line.take();
      self.post_slash(cmd);
      return self.after_send();
    }
    if self.feedish() && !self.line.text.trim().is_empty() {
      self.feed_post();
      return self.after_send();
    }
    if let Some(cmd) = whisper::command(&self.line.text) {
      self.line.take();
      self.whisper_slash(cmd);
      return self.after_send();
    }
    if self.kind() == Kind::Whispers {
      if let Some(text) = self.line.take() {
        self.reply_whisper(text);
      }
      return self.after_send();
    }
    if self.kind() == Kind::Mentions {
      return self.say("mentions is read-only. open a line to answer it");
    }
    let Some((platform, chan)) = self.target().map(|(p, k, _)| (p, Box::<str>::from(k))) else { return };
    // a timeout has an end we know: say how long instead of sending a line that is sure to bounce (words stay)
    if let Some(l) = self.lock_here().filter(|l| l.blocks()) {
      return self.say(l.text(super::now_ms()));
    }
    if let Some(text) = self.line.take() {
      let reply = self.reply.take();
      if let Some(v) = self.view() {
        v.deselect();
      }
      let text = crate::model::emoji::convert(&text).into_owned();
      self.auto_add(&text);
      self.net.send(Cmd::Send { platform, chan, text, reply });
      if let Some(v) = self.view() {
        v.scroll_to_bottom();
      }
      self.after_send();
    }
  }
}
