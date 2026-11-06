// the whispers tab (ext whispers.js): twitch whispers and heatsync dms to and from you in one timeline.
// reading: heatsync's server holds the twitch subscription and pushes `whisper:new` (and `dm:new`) to the
// signed-in socket; history comes from /api/whispers and /api/dm when the tab first opens. sending:
// /w user msg, /dm user msg, /r msg, or just type in the tab to answer the last person. a failed send stays in the list: Enter retries
// it (or re-links twitch with the whispers permission when that is what is missing)
use super::App;
use super::card::Load;
use crate::model::message::{Extra, Kind as MsgKind, Message, Platform, Seg};
use crate::model::tabs::Kind;
use crate::model::whisper::{self, Cmd as Slash, Status, Timeline, Via, Whisper, Who};
use crate::net::{Cmd, social};
use crate::ui::theme;
use crate::util::time::now_ms;

// the whispers timeline, whether this login's history was asked for, and a counter bumped on every login
// change so a history read from an earlier one is dropped
#[derive(Default)]
pub(super) struct Whispers {
  pub(super) timeline: Timeline,
  pub(super) loaded: bool,
  pub(super) epoch: u32,
  // the toasts still showing, for the per-sender count
  toasts: Vec<Toast>,
  toast_seq: u64,
}

// one sender's toast: how many whispers it stands for, when the last landed, and the row that says it
struct Toast {
  via: Via,
  login: Box<str>,
  n: u32,
  at: i64,
  id: Box<str>,
}

// a sender's whispers inside this long fold into one toast (the ext's toast lives 6s)
const TOAST_MS: i64 = 6000;
// how much of the whisper the toast shows
const TOAST_CHARS: usize = 80;
const TOASTS_MAX: usize = 32;

const NEED_LOGIN: &str = "log in to heatsync.org to send whispers and dms (ctrl-l)";
// heatsync caps a dm at 500 characters
const DM_MAX: usize = 500;
const UNKNOWN: &str = "unknown user. whisper someone first";

impl App {
  // what the empty tab says: it depends on who you are
  pub(super) fn whisper_empty(&mut self) {
    let text = match &self.me {
      None => "log in to read and send whispers and dms (ctrl-l)",
      Some(m) if m.twitch_id.is_none() => "no dms yet. /dm user msg to start one. whispers need twitch (/w user msg)",
      Some(_) => "nothing yet. /w user msg whispers, /dm user msg dms, /r msg answers the last",
    };
    if let Some(i) = self.tab_of(Kind::Whispers) {
      self.tabs[i].chat().set_empty(text);
    }
  }

  // the history, once per login, the first time the tab is looked at
  pub(super) fn whispers_load(&mut self) {
    let Some(me) = &self.me else { return };
    let id = me.twitch_id;
    if !self.whisper.loaded {
      self.whisper.loaded = true;
      self.net.send(Cmd::Social(social::Cmd::WhisperHistory { me: id, epoch: self.whisper.epoch }));
    }
  }

  // a different login (or none): the old person's whispers must not stay on screen
  pub(super) fn whispers_reset(&mut self) {
    self.whisper.timeline = Default::default();
    self.whisper.loaded = false;
    self.whisper.epoch = self.whisper.epoch.wrapping_add(1);
    if let Some(i) = self.tab_of(Kind::Whispers) {
      self.tabs[i].chat().clear();
      self.tabs[i].unread = false;
    }
    self.whisper_empty();
    self.bar_dirty = true;
    self.wake();
  }

  pub(super) fn put_row(&mut self, w: &Whisper) {
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
    let mut heard = Vec::new();
    for w in items {
      if let Some(added) = self.whisper.timeline.add(w) {
        fresh = true;
        if live && !added.out {
          heard.push(added.clone());
        }
      }
    }
    if !fresh {
      return;
    }
    if live {
      let rows: Vec<_> = self.whisper.timeline.iter().rev().take(1).map(whisper::message).collect();
      for m in rows {
        self.tabs[i].chat().push(m);
      }
    } else {
      // history lands in time order among whatever is already here
      let rows: Vec<_> = self.whisper.timeline.iter().map(whisper::message).collect();
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
    // someone whispered while you are looking at something else (ext handleIncomingWhisper)
    if self.kind() != Kind::Whispers {
      for w in &heard {
        self.whisper_heard(w);
      }
    }
    self.wake();
  }

  // the toast and the inline row for a whisper or dm that landed off the whispers tab. a blocked person
  // gets neither
  fn whisper_heard(&mut self, w: &Whisper) {
    if crate::model::filter::with(|f| f.blocked(&crate::model::filter::key(&w.who.login))) {
      return;
    }
    if self.set.whisper_toast {
      self.whisper_toast(w);
    }
    if self.set.inline_dm {
      self.whisper_inline(w);
    }
  }

  // `{name} whispered: {first 80 chars}`, with ` ×N` for a burst from one sender. the line has no click
  // (a status line here is a plain line in the chat): the whispers tab is a click away on the tab bar
  fn whisper_toast(&mut self, w: &Whisper) {
    let now = now_ms();
    let ws = &mut self.whisper;
    ws.toasts.retain(|t| now - t.at < TOAST_MS);
    ws.toast_seq += 1;
    let fresh: Box<str> = format!("whisper-toast-{}", ws.toast_seq).into();
    let login = w.who.login.to_lowercase();
    let at = match ws.toasts.iter().position(|t| t.via == w.who.via && *t.login == *login) {
      Some(i) => i,
      None => {
        if ws.toasts.len() >= TOASTS_MAX {
          ws.toasts.remove(0);
        }
        ws.toasts.push(Toast { via: w.who.via, login: login.into(), n: 0, at: now, id: fresh.clone() });
        ws.toasts.len() - 1
      }
    };
    let t = &mut ws.toasts[at];
    (t.n, t.at) = (t.n + 1, now);
    let more = if t.n > 1 { format!(" ×{}", t.n) } else { String::new() };
    let snippet: String = w.text.chars().take(TOAST_CHARS).collect();
    let mut m = Message::system(now, Platform::Twitch, format!("{} whispered: {snippet}{more}", w.who.name));
    m.color = theme::CYAN;
    let old = t.id.clone();
    let Some(tab) = self.tabs.get_mut(self.active) else { return };
    let v = tab.chat();
    // the toast still last in the chat counts up in place; one with lines after it starts a new line
    if *old != *fresh && v.msgs().next_back().is_some_and(|l| *l.id == *old) {
      m.id = old.clone();
      v.replace(&old, m);
    } else {
      m.id = fresh.clone();
      t.id = fresh;
      v.push(m);
    }
  }

  // `[dm]` row in the chat you are on (ext injectInlineNotif 'dm'): yellow edge, the sender's name in
  // their color. a heatsync dm's sender is a heatsync name, not a twitch login, so it carries none
  fn whisper_inline(&mut self, w: &Whisper) {
    if self.kind() != Kind::Chat {
      return;
    }
    let twitch = w.who.via == Via::Twitch;
    let mut m = Message::system(w.ts, Platform::Twitch, "");
    m.kind = MsgKind::Chat;
    m.login = if twitch { w.who.login.clone() } else { "".into() };
    m.user_id = if twitch { w.who.id.clone() } else { "".into() };
    m.nick = w.who.name.clone();
    m.color = w.who.color;
    m.body = w.text.clone();
    m.x = Some(Box::new(Extra {
      head: Box::new([Seg { text: "[dm] ".into(), ink: theme::YELLOW, bold: true }]),
      edge: theme::YELLOW,
      ..Extra::default()
    }));
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.chat().push(m);
    }
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
    let Some(w) = self.whisper.timeline.settle(key, status, who).cloned() else { return };
    self.put_row(&w);
    match failed {
      // the ext surfaces the tab on a failure, where the row carries the retry
      Some(why) => {
        if let Some(i) = self.tab_of(Kind::Whispers) {
          self.select(i);
        }
        let what = if w.who.via == Via::Heatsync { "dm" } else { "whisper" };
        self.say(format!("{what} failed: {why}"));
      }
      None if self.kind() != Kind::Whispers => self.say(format!("sent to {}", w.who.name)),
      None => self.wake(),
    }
    self.input_dirty = true;
  }

  // /w and /r from any tab
  pub(super) fn whisper_slash(&mut self, c: Slash) {
    match c {
      Slash::Usage => self.say("usage: /w user msg · /dm user msg · /r msg"),
      Slash::To(via, login, text) => {
        let who = self.whisper.timeline.known(via, &login).unwrap_or(Who {
          via,
          id: "".into(),
          name: login.clone(),
          login,
          color: crate::ui::theme::WHITE,
        });
        match text {
          // ext: opens the conversation and points /r at it
          None => {
            let name = who.name.clone();
            self.whisper.timeline.last = Some(who);
            if let Some(i) = self.tab_of(Kind::Whispers) {
              self.select(i);
            }
            self.say(format!("type to {} {name}", if via == Via::Heatsync { "dm" } else { "whisper" }));
          }
          Some(text) => self.start_whisper(who, text),
        }
      }
      Slash::Reply(text) => self.reply_whisper(text),
    }
  }

  // answer the last person (typed in the tab, or /r)
  pub(super) fn reply_whisper(&mut self, text: String) {
    match self.whisper.timeline.last.clone() {
      Some(who) => self.start_whisper(who, text),
      None => self.say(UNKNOWN),
    }
  }

  fn start_whisper(&mut self, who: Who, text: String) {
    if who.via == Via::Heatsync {
      if self.me.is_none() {
        return self.say(NEED_LOGIN);
      }
      if text.chars().count() > DM_MAX {
        return self.say(format!("dm too long: {DM_MAX} characters at most"));
      }
    } else {
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
    }
    let w = self.whisper.timeline.push_out(who, &text).clone();
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

  // the platform a dm's name is looked up on: this tab's, twitch when it has none (as /follow does)
  fn dm_platform(&self) -> Platform {
    self.tabs.get(self.active).and_then(|t| t.sources.first()).map_or(Platform::Twitch, |s| s.0)
  }

  fn send_whisper(&mut self, w: &Whisper) {
    let mut id = (!w.who.id.is_empty()).then(|| w.who.id.clone());
    if w.who.via == Via::Heatsync {
      let platform = self.dm_platform();
      if id.is_none()
        && let Some(Load::Done(i)) = self.cards.get(platform, &w.who.login)
      {
        id = i.profile_id.clone();
      }
      return self.net.send(Cmd::Social(social::Cmd::DmSend {
        key: w.key,
        platform,
        login: w.who.login.clone(),
        id,
        text: w.text.to_string(),
      }));
    }
    self.net.send(Cmd::Social(social::Cmd::WhisperSend {
      key: w.key,
      login: w.who.login.clone(),
      id,
      text: w.text.to_string(),
    }));
  }

  fn selected_whisper(&self) -> Option<Whisper> {
    let t = self.tabs.get(self.active).filter(|t| t.kind == Kind::Whispers)?;
    self.whisper.timeline.get(whisper::key_of(&t.view.selected()?.id)?).cloned()
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
        if let Some(w) = self.whisper.timeline.retry(w.key).cloned() {
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
    self.whisper.timeline.last = Some(w.who);
    self.enter_insert();
    self.input_dirty = true;
    self.wake();
  }

  // the input bar's label while the whispers tab is up: who a plain line goes to
  pub(super) fn whisper_label(&self) -> Option<String> {
    self
      .whisper
      .timeline
      .last
      .as_ref()
      .map(|w| format!("{} {} ", if w.via == Via::Heatsync { "dm" } else { "whisper" }, w.name))
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
    let who = Who { via: Via::Twitch, id: "1".into(), login: "bob".into(), name: "Bob".into(), color: 0 };
    let mut t = crate::model::whisper::Timeline::default();
    let k = t.push_out(who, "hi").key;
    let w = t.settle(k, Status::Failed(Fail::Server), None).unwrap().clone();
    assert!(whisper::message(&w).has(flag::ALERT));
  }

  fn signed_in(app: &mut App, twitch: bool) {
    app.me = Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: twitch.then_some(1),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
  }

  fn dms(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<String> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| match c {
        Cmd::Social(social::Cmd::DmSend { login, id, text, .. }) => Some(format!("dm {login} {id:?} {text}")),
        Cmd::Social(social::Cmd::WhisperSend { login, text, .. }) => Some(format!("whisper {login} {text}")),
        Cmd::Social(social::Cmd::WhisperHistory { me, .. }) => Some(format!("history {me:?}")),
        _ => None,
      })
      .collect()
  }

  fn said(app: &App) -> String {
    app.tabs[app.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
  }

  #[test]
  fn dm_sends_open_reply_and_say_why_they_fail() {
    use crate::net::NetEvent;
    let (net, mut rx) = crate::net::Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let typed = |app: &mut App, s: &str| {
      app.line.set_text(s.into());
      app.send();
    };
    typed(&mut app, "/dm amy hello");
    assert!(dms(&mut rx).is_empty(), "logged out");
    assert!(said(&app).contains("log in"));
    // a heatsync login without twitch can still dm, and loads dm-only history
    signed_in(&mut app, false);
    app.whispers_load();
    assert_eq!(dms(&mut rx), ["history None"]);
    typed(&mut app, "/dm @Amy hello there");
    assert_eq!(dms(&mut rx), ["dm amy None hello there"]);
    typed(&mut app, "/w amy nope");
    assert!(dms(&mut rx).is_empty(), "whispers still need twitch");
    typed(&mut app, &format!("/dm amy {}", "x".repeat(501)));
    assert!(dms(&mut rx).is_empty());
    assert!(said(&app).contains("too long"));
    // the net says it went through: the person is now known by id
    let key = app.whisper.timeline.iter().next().unwrap().key;
    let amy = Who { via: Via::Heatsync, id: "77".into(), login: "amy".into(), name: "amy".into(), color: 0xffffff };
    app.on_net(NetEvent::WhisperSent { key, res: Ok(amy) });
    assert_eq!(app.whisper.timeline.iter().next().unwrap().status, Status::Sent);
    // bare /dm opens the tab and points /r at that person; /r then goes by id
    typed(&mut app, "/dm bob");
    assert_eq!(app.kind(), Kind::Whispers);
    assert!(said(&app).contains("type to dm bob"));
    typed(&mut app, "/r hi bob");
    assert_eq!(dms(&mut rx), ["dm bob None hi bob"]);
    typed(&mut app, "/dm amy");
    typed(&mut app, "/r again");
    assert_eq!(dms(&mut rx), ["dm amy Some(\"77\") again"], "a known person is not looked up twice");
    // mutual follow: said plainly, the row keeps the retry
    let key = app.whisper.timeline.iter().last().unwrap().key;
    app.on_net(NetEvent::WhisperSent { key, res: Err(whisper::Fail::Follow("amy".into())) });
    assert!(said(&app).contains("dm failed: dms need a follow both ways. /follow amy first"));
    app.whispers_reset();
    // a name with no heatsync account
    typed(&mut app, "/dm ghost hi");
    let key = app.whisper.timeline.iter().last().unwrap().key;
    app.on_net(NetEvent::WhisperSent { key, res: Err(whisper::Fail::Other("ghost isn't on heatsync".into())) });
    assert!(said(&app).contains("ghost isn't on heatsync"));
    // a twitch whisper still goes to twitch, and its reply stays a whisper
    signed_in(&mut app, true);
    dms(&mut rx);
    typed(&mut app, "/w bob yo");
    typed(&mut app, "/r again");
    assert_eq!(dms(&mut rx), ["whisper bob yo", "whisper bob again"]);
  }
}
