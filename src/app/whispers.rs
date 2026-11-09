// the whispers tab (ext whispers.js): twitch whispers and heatsync dms to and from you in one timeline.
// reading: heatsync's server holds the twitch subscription and pushes `whisper:new` (and `dm:new`) to the
// signed-in socket; history comes from /api/whispers and /api/dm when the tab first opens. sending:
// /w user msg, /dm user msg, /r msg, or just type in the tab to answer the last person. a failed send stays in the list: Enter retries
// it (or re-links twitch with the whispers permission when that is what is missing)
use super::App;
use super::card::Load;
use crate::i18n::{f, t};
use crate::model::message::{Message, Platform};
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
  // the dm (by its row's key) waiting on "yes, delete for both"
  pub(super) pending_delete: Option<u64>,
  // the toasts still showing, for the per-sender count
  toasts: Vec<Toast>,
  toast_seq: u64,
  // every toast line still in some tab's scrollback, so streamer mode can redo it in all of them
  lines: Vec<ToastLine>,
}

// what a toast line says, kept apart from the line so a flip of streamer mode can rewrite it anywhere
struct ToastLine {
  id: Box<str>,
  ts: i64,
  name: Box<str>,
  snippet: Box<str>,
  n: u32,
}

impl ToastLine {
  fn message(&self, hide: bool) -> Message {
    let more = if self.n > 1 { format!(" ×{}", self.n) } else { String::new() };
    // streamer mode: who, never what
    let line = if hide {
      f("app.whispers.toast_from", &[("name", &self.name), ("more", &more)])
    } else {
      f("app.whispers.toast", &[("name", &self.name), ("text", &self.snippet), ("more", &more)])
    };
    let mut m = Message::system(self.ts, Platform::Twitch, line);
    m.color = theme::CYAN;
    m.id = self.id.clone();
    m
  }
}

// a toast line is never remembered past this many
const LINES_MAX: usize = 64;

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

fn need_login() -> &'static str {
  t("app.whispers.need_login")
}
// heatsync caps a dm at 500 characters
const DM_MAX: usize = 500;
fn unknown() -> &'static str {
  t("app.whispers.unknown")
}

impl App {
  // what the empty tab says: it depends on who you are
  pub(super) fn whisper_empty(&mut self) {
    let text = match &self.me {
      None => t("app.builtin.empty_whispers"),
      Some(m) if m.twitch_id.is_none() => t("app.whispers.empty_no_twitch"),
      Some(_) => t("app.whispers.empty"),
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
    self.whisper.pending_delete = None;
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
    let m = whisper::message(w, self.hides_whispers());
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
      let hide = self.hides_whispers();
      let rows: Vec<_> = self.whisper.timeline.iter().rev().take(1).map(|w| whisper::message(w, hide)).collect();
      for m in rows {
        self.tabs[i].chat().push(m);
      }
    } else {
      // history lands in time order among whatever is already here
      let hide = self.hides_whispers();
      let rows: Vec<_> = self.whisper.timeline.iter().map(|w| whisper::message(w, hide)).collect();
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

  // the toast for a whisper or dm that landed off the whispers tab. a blocked person gets none
  fn whisper_heard(&mut self, w: &Whisper) {
    if crate::model::filter::with(|f| f.blocked(&crate::model::filter::key(&w.who.login))) {
      return;
    }
    if self.set.whisper_toast {
      self.whisper_toast(w);
    }
  }

  // `{name} whispered: {first 80 chars}`, with ` ×N` for a burst from one sender. the line has no click
  // (a status line here is a plain line in the chat): the whispers tab is a click away on the tab bar
  fn whisper_toast(&mut self, w: &Whisper) {
    let now = now_ms();
    let hot = self.streamer_hot();
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
    let n = t.n;
    let old = t.id.clone();
    let Some(tab) = self.tabs.get_mut(self.active) else { return };
    let v = tab.chat();
    let snippet: Box<str> = w.text.chars().take(TOAST_CHARS).collect::<String>().into();
    let mut line = ToastLine { id: fresh.clone(), ts: now, name: w.who.name.clone(), snippet, n };
    // the toast still last in the chat counts up in place; one with lines after it starts a new line
    if *old != *fresh && v.msgs().next_back().is_some_and(|l| *l.id == *old) {
      line.id = old.clone();
      v.replace(&old, line.message(hot));
      if let Some(l) = ws.lines.iter_mut().find(|l| *l.id == *old) {
        *l = line;
      }
    } else {
      t.id = fresh;
      v.push(line.message(hot));
      if ws.lines.len() >= LINES_MAX {
        ws.lines.remove(0);
      }
      ws.lines.push(line);
    }
  }

  // streamer mode flipped: every toast line in every tab says (or stops hiding) its words
  pub(super) fn toasts_rebuild(&mut self) {
    let hide = self.streamer_hot();
    let lines = std::mem::take(&mut self.whisper.lines);
    for tab in &mut self.tabs {
      for v in tab.views() {
        for l in &lines {
          v.replace(&l.id, l.message(hide));
        }
      }
    }
    self.whisper.lines = lines;
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
    let Some(w) = self.whisper.timeline.settle(key, status, who).cloned() else { return };
    self.put_row(&w);
    match failed {
      // the ext surfaces the tab on a failure, where the row carries the retry
      Some(why) => {
        if let Some(i) = self.tab_of(Kind::Whispers) {
          self.select(i);
        }
        let key = if w.who.via == Via::Heatsync { "app.whispers.dm_failed" } else { "app.whispers.whisper_failed" };
        self.say(f(key, &[("why", &why.to_string())]));
      }
      None if self.kind() != Kind::Whispers => self.say(f("app.whispers.sent_to", &[("name", &w.who.name)])),
      None => self.wake(),
    }
    self.input_dirty = true;
  }

  // /w and /r from any tab
  pub(super) fn whisper_slash(&mut self, c: Slash) {
    match c {
      Slash::Usage => self.say(t("app.whispers.usage")),
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
            self.say(f(
              if via == Via::Heatsync { "app.whispers.type_dm" } else { "app.whispers.type_whisper" },
              &[("name", &name)],
            ));
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
      None => self.say(unknown()),
    }
  }

  fn start_whisper(&mut self, who: Who, text: String) {
    if who.via == Via::Heatsync {
      if self.me.is_none() {
        return self.say(need_login());
      }
      if text.chars().count() > DM_MAX {
        return self.say(f("app.whispers.dm_too_long", &[("max", &DM_MAX.to_string())]));
      }
    } else {
      match &self.me {
        None => return self.say(need_login()),
        // a kick-only or youtube-only login: the twitch sign-in (with the whispers permission) is one step away
        Some(m) if m.twitch_id.is_none() => {
          self.net.send(Cmd::Relink("whispers"));
          return self.say(t("app.whispers.need_twitch"));
        }
        // twitch told us which permission packs this login holds, and whispers is not one: ask now
        // instead of failing the send first
        Some(m) if !m.packs.is_empty() && !m.has_pack("whispers") => {
          self.net.send(Cmd::Relink("whispers"));
          return self.say(super::feed::needs_twitch("whispers"));
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

  pub(super) fn selected_whisper(&self) -> Option<Whisper> {
    let t = self.tabs.get(self.active).filter(|t| t.kind == Kind::Whispers)?;
    self.whisper.timeline.get(whisper::key_of(&t.view.selected()?.id)?).cloned()
  }

  // Enter / click on a row: a failed send is retried (or re-linked); anything else answers that person
  pub(super) fn whisper_activate(&mut self) {
    let Some(w) = self.selected_whisper() else { return };
    match &w.status {
      Status::Failed(whisper::Fail::Relink) => {
        self.say(t("app.whispers.relink"));
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
    self.whisper.timeline.last.as_ref().map(|w| {
      f(
        if w.via == Via::Heatsync { "app.whispers.label_dm" } else { "app.whispers.label_whisper" },
        &[("name", &w.name)],
      )
    })
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
    assert!(whisper::message(&w, false).has(flag::ALERT));
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
