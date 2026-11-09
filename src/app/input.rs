// the message box: its hint, its mode, sending what is typed, and the slash commands that are ours

use super::{App, LOGIN_HINT, screen};
use crate::model::message::Platform;
use crate::model::settings::{Local, local_cmd};
use crate::model::tabs::Kind;
use crate::model::vi::Mode;
use crate::model::{chan, whisper};

impl App {
  pub(super) fn hint(&self) -> String {
    if !self.tabs.iter().any(|t| t.kind == Kind::Chat) {
      let login = if self.me.is_none() { " · ctrl-l log in" } else { "" };
      return format!(
        "no channels — click + or press ctrl-t to add one, or /import to add the ones you follow on twitch{login}"
      );
    }
    if self.kind() == Kind::Mentions {
      return "enter or a click opens the line in its channel".into();
    }
    if let Some(l) = self.logs_shown() {
      return l.hint();
    }
    if self.chain_shown() {
      return "reply thread · t or esc goes back · r replies".into();
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
    let Some((p, _)) = self.target() else { return lead.into() };
    // locked out of this room: the box says why (and for how long) instead of inviting you to type
    if let Some(l) = self.lock_here() {
      return l.text(super::now_ms());
    }
    // every feed the send reaches needs your account on it
    if let Some((unlinked, _)) = self.legs().into_iter().find(|(l, _)| me.name_on(*l).is_none()) {
      return format!("{} isn't linked · ctrl-l to link it", unlinked.name());
    }
    match me.name_on(p) {
      Some(name) => format!("{lead} as {name}"),
      None => format!("{} isn't linked · ctrl-l to link it", p.name()),
    }
  }

  // auto-hide: an empty bar takes no room (mode and reply stay as they are)
  pub(super) fn hidden(&self) -> bool {
    self.set.auto_hide
      && self.line.text.is_empty()
      && !self.ime.composing()
      && (self.prompt.is_none() || self.bar_find())
  }

  // the feed the box speaks as (a send may reach more: `legs`)
  pub(super) fn target(&self) -> Option<(Platform, &str)> {
    let t = self.tabs.get(self.active)?;
    let (p, key) = t.sources.get(t.focus())?;
    Some((*p, key))
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
    self.sent_save();
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
      Local::Buttons(id, arg) => {
        if let Some(d) = crate::model::schema::def(id) {
          self.buttons_to(d, &arg);
        }
      }
      Local::Text(id, arg) => {
        if let Some(d) = crate::model::schema::def(id) {
          self.text_to(d, &arg);
        }
      }
      Local::Tab(Some(k), shown) => self.set_shown(k, shown),
      Local::Tab(None, shown) => {
        self.say(format!("usage: /{} feed, whispers or mentions", if shown { "show" } else { "hide" }))
      }
      Local::Zoom(Some(pct)) => self.zoom_to(pct),
      Local::Zoom(None) => self.say("usage: /zoom 1.5 (anything from 0.9 to 3)"),
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
      Local::Reset => self.ask_reset(),
      Local::Keys => self.open_screen(screen::Page::Keys),
      Local::Tour => self.tour_open(),
      Local::Login(arg) => self.login_cmd(&arg),
      Local::Logout => self.logout_cmd(),
      Local::Account => self.account_open(),
      Local::Following => self.toggle_following(),
      Local::Import => self.toggle_import(),
      Local::Emotes => self.inv_open(),
      Local::Blocked => self.blocked_here(),
      Local::Summary => self.summary_here(),
      Local::Status => self.status_here(),
      Local::Pred => self.pred_here(),
      Local::ModTools => self.tools_here(),
      Local::Lclear => self.lclear(),
      Local::ClearSent => self.sent_clear(),
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
    // after every built-in had its look (a built-in always wins a name); whisper's come later, so ask here
    if whisper::command(&self.line.text).is_none() && self.ucmd_send() {
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
    let legs: Vec<(Platform, Box<str>)> = self.legs().into_iter().map(|(p, k)| (p, k.into())).collect();
    if legs.is_empty() {
      return;
    }
    // a timeout has an end we know: say how long instead of sending a line that is sure to bounce. the feeds
    // that aren't timed out still get it, and the ones skipped are named (all of them: the words stay)
    let now = super::now_ms();
    let (open, shut): (Vec<_>, Vec<_>) =
      legs.into_iter().partition(|(p, k)| self.me.is_none() || !self.locks.get(*p, k, now).is_some_and(|l| l.blocks()));
    // a youtube feed that isn't live has no chat to take the line (the ext sends there only while live): in a
    // merged send it is left out quietly instead of a "not live" line under every message. sent to alone, it
    // still goes, so the reason shows
    let mut open = open;
    if open.len() > 1 && open.iter().any(|(p, k)| *p != Platform::Youtube || self.live.has(*p, k)) {
      open.retain(|(p, k)| *p != Platform::Youtube || self.live.has(*p, k));
    }
    if open.is_empty() {
      let (p, k) = &shut[0];
      if let Some(l) = self.locks.get(*p, k, now) {
        self.say(l.text(now));
      }
      return;
    }
    // the network part crashed: a line sent now goes nowhere, so it stays in the box
    if !self.net.alive() {
      return self.say("not sent: the network part stopped, restart heatsync-chat");
    }
    if let Some(text) = self.line.take() {
      for (p, k) in &shut {
        if let Some(l) = self.locks.get(*p, k, now) {
          let why = l.text(now);
          self.say(format!("skipped {}: {why}", p.name()));
        }
      }
      let reply = self.reply.take();
      if let Some(v) = self.view() {
        v.deselect();
      }
      let text = crate::model::emoji::convert(&text).into_owned();
      let picks = self.auto_add(&text);
      self.fan_out(&open, &text, reply);
      self.tell_origin(&open, &text, &picks);
      self.vouch(&open, picks);
      if let Some(v) = self.view() {
        v.scroll_to_bottom();
      }
      self.after_send();
    }
  }
}
