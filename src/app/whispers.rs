// the whispers tab (ext whispers.js): twitch whispers to and from you in one timeline.
// reading: heatsync's server holds the twitch subscription and pushes `whisper:new` to the signed-in
// socket; history comes from /api/whispers when the tab first opens. sending: /w user msg, /r msg,
// or just type in the tab to answer the last person. a failed send stays in the list: Enter retries
// it (or re-links twitch with the whispers permission when that is what is missing)
use super::App;
use crate::model::tabs::Kind;
use crate::model::whisper::{self, Cmd as Slash, Status, Whisper, Who};
use crate::net::Cmd;
use crate::net::social;

const NEED_LOGIN: &str = "log in to heatsync.org to send whispers (ctrl-l)";
const UNKNOWN: &str = "unknown user. whisper someone first";

impl App {
  // what the empty tab says: it depends on who you are
  pub(super) fn whisper_empty(&mut self) {
    let text = match &self.me {
      None => "log in to read and send twitch whispers (ctrl-l)",
      Some(m) if m.twitch_id.is_none() => "whispers are twitch-only. send one (/w user msg) to log in with twitch",
      Some(_) => "no whispers yet. /w user msg to start one, /r msg to answer the last",
    };
    if let Some(i) = self.tab_of(Kind::Whispers) {
      self.tabs[i].chat().set_empty(text);
    }
  }

  // the history, once per login, the first time the tab is looked at
  pub(super) fn whispers_load(&mut self) {
    let Some(id) = self.me.as_ref().and_then(|m| m.twitch_id) else { return };
    if !self.whispers_loaded {
      self.whispers_loaded = true;
      self.net.send(Cmd::Social(social::Cmd::WhisperHistory { me: id, epoch: self.whisper_epoch }));
    }
  }

  // a different login (or none): the old person's whispers must not stay on screen
  pub(super) fn whispers_reset(&mut self) {
    self.whispers = Default::default();
    self.whispers_loaded = false;
    self.whisper_epoch = self.whisper_epoch.wrapping_add(1);
    if let Some(i) = self.tab_of(Kind::Whispers) {
      self.tabs[i].chat().clear();
      self.tabs[i].unread = false;
    }
    self.whisper_empty();
    self.bar_dirty = true;
    self.wake();
  }

  fn put_row(&mut self, w: &Whisper) {
    let Some(i) = self.tab_of(Kind::Whispers) else { return };
    let m = whisper::message(w);
    let v = self.tabs[i].chat();
    if !v.replace(&m.id.clone(), m.clone()) {
      v.push(m);
    }
  }

  pub(super) fn on_whispers(&mut self, items: Vec<Whisper>, live: bool) {
    let Some(i) = self.tab_of(Kind::Whispers) else { return };
    let mut fresh = false;
    for w in items {
      fresh |= self.whispers.add(w).is_some();
    }
    if !fresh {
      return;
    }
    if live {
      let rows: Vec<_> = self.whispers.iter().rev().take(1).map(whisper::message).collect();
      for m in rows {
        self.tabs[i].chat().push(m);
      }
    } else {
      // history lands in time order among whatever is already here
      let rows: Vec<_> = self.whispers.iter().map(whisper::message).collect();
      let v = self.tabs[i].chat();
      v.clear();
      for m in rows {
        v.push(m);
      }
    }
    if i == self.active {
      self.input_dirty = true;
    }
    if live {
      self.unseen_note(crate::model::unseen::Surface::Whispers);
    }
    self.wake();
  }

  pub(super) fn on_whisper_sent(&mut self, key: u64, res: Result<Who, whisper::Fail>) {
    let (status, who) = match res {
      Ok(w) => (Status::Sent, Some(w)),
      Err(f) => (Status::Failed(f), None),
    };
    let failed = match &status {
      Status::Failed(f) => Some(whisper::fail_text(f)),
      _ => None,
    };
    let Some(w) = self.whispers.settle(key, status, who).cloned() else { return };
    self.put_row(&w);
    match failed {
      // the ext surfaces the tab on a failure, where the row carries the retry
      Some(why) => {
        if let Some(i) = self.tab_of(Kind::Whispers) {
          self.select(i);
        }
        self.say(format!("whisper failed: {why}"));
      }
      None if self.kind() != Kind::Whispers => self.say(format!("sent to {}", w.who.name)),
      None => self.wake(),
    }
    self.input_dirty = true;
  }

  // /w and /r from any tab
  pub(super) fn whisper_slash(&mut self, c: Slash) {
    match c {
      Slash::Usage => self.say("usage: /w user msg · /r msg"),
      Slash::To(login, None) => {
        let who = self.whispers.known(&login).unwrap_or(Who {
          id: "".into(),
          name: login.clone(),
          login,
          color: crate::ui::theme::WHITE,
        });
        // ext: opens the conversation and points /r at it
        let name = who.name.clone();
        self.whispers.last = Some(who);
        if let Some(i) = self.tab_of(Kind::Whispers) {
          self.select(i);
        }
        self.say(format!("type to whisper {name}"));
      }
      Slash::To(login, Some(text)) => {
        let who = self.whispers.known(&login).unwrap_or(Who {
          id: "".into(),
          name: login.clone(),
          login,
          color: crate::ui::theme::WHITE,
        });
        self.start_whisper(who, text);
      }
      Slash::Reply(text) => self.reply_whisper(text),
    }
  }

  // answer the last person (typed in the tab, or /r)
  pub(super) fn reply_whisper(&mut self, text: String) {
    match self.whispers.last.clone() {
      Some(who) => self.start_whisper(who, text),
      None => self.say(UNKNOWN),
    }
  }

  fn start_whisper(&mut self, who: Who, text: String) {
    match &self.me {
      None => return self.say(NEED_LOGIN),
      // a kick-only or youtube-only login: the twitch sign-in (with the whispers permission) is one step away
      Some(m) if m.twitch_id.is_none() => {
        self.net.send(Cmd::Relink("whispers"));
        return self.say("whispers need twitch. finish logging in with twitch in your browser…");
      }
      // twitch told us which permission packs this login holds, and whispers is not one: ask now
      // instead of failing the send first
      Some(m) if !m.packs.is_empty() && !m.has_pack("whispers") => {
        self.net.send(Cmd::Relink("whispers"));
        return self.say("whispers need a twitch permission. finish allowing it in your browser…");
      }
      Some(_) => {}
    }
    let w = self.whispers.push_out(who, &text).clone();
    self.put_row(&w);
    self.send_whisper(&w);
    if let Some(i) = self.tab_of(Kind::Whispers)
      && i == self.active
    {
      self.tabs[i].chat().scroll_to_bottom();
    }
    self.input_dirty = true;
    self.wake();
  }

  fn send_whisper(&mut self, w: &Whisper) {
    let id = (!w.who.id.is_empty()).then(|| w.who.id.clone());
    self.net.send(Cmd::Social(social::Cmd::WhisperSend {
      key: w.key,
      login: w.who.login.clone(),
      id,
      text: w.text.to_string(),
    }));
  }

  fn selected_whisper(&self) -> Option<Whisper> {
    let t = self.tabs.get(self.active).filter(|t| t.kind == Kind::Whispers)?;
    self.whispers.get(whisper::key_of(&t.view.selected()?.id)?).cloned()
  }

  // Enter / click on a row: a failed send is retried (or re-linked); anything else answers that person
  pub(super) fn whisper_activate(&mut self) {
    let Some(w) = self.selected_whisper() else { return };
    match &w.status {
      Status::Failed(whisper::Fail::Relink) => {
        self.say("finish re-linking in your browser…");
        self.net.send(Cmd::Relink("whispers"));
      }
      Status::Failed(_) => {
        if let Some(w) = self.whispers.retry(w.key).cloned() {
          self.put_row(&w);
          self.send_whisper(&w);
        }
      }
      _ => self.whisper_reply_selected(),
    }
    self.wake();
  }

  // `r`: point replies at the person on the selected row and start typing
  pub(super) fn whisper_reply_selected(&mut self) {
    let Some(w) = self.selected_whisper() else { return };
    self.whispers.last = Some(w.who);
    self.enter_insert();
    self.input_dirty = true;
    self.wake();
  }

  // the input bar's label while the whispers tab is up: who a plain line goes to
  pub(super) fn whisper_label(&self) -> Option<String> {
    self.whispers.last.as_ref().map(|w| format!("{} ", w.name))
  }
}

// failed rows carry the red edge; kept here so the test sees the same thing the tab draws
#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::flag;
  use crate::model::whisper::Fail;

  #[test]
  fn a_failed_row_is_red() {
    let who = Who { id: "1".into(), login: "bob".into(), name: "Bob".into(), color: 0 };
    let mut t = crate::model::whisper::Timeline::default();
    let k = t.push_out(who, "hi").key;
    let w = t.settle(k, Status::Failed(Fail::Server), None).unwrap().clone();
    assert!(whisper::message(&w).has(flag::ALERT));
  }
}
