// moderation (ext mod-toolbar.js): when our twitch badges say we moderate a room, a line's menu offers
// delete / timeout / purge / ban / unban, and /ban /timeout /unban /delete /purge work in the tab.
// heatsync's server does the twitch and kick calls (POST /api/mod/action)
use super::App;
use super::pop::{Pop, Subject, Todo};
use crate::model::message::{Message, Platform};
use crate::model::modcmd::{Cmd as Slash, Verb};
use crate::net::Cmd;
use crate::net::modact::{Done, Fail, Req};
use crate::util::time::now_ms;

impl App {
  pub(super) fn on_mod_state(&mut self, ch: &str, on: bool) {
    let had = self.mod_in.contains(ch);
    if on && !had {
      self.mod_in.insert(ch.into());
    } else if !on && had {
      self.mod_in.remove(ch);
    }
  }

  // do our badges let us moderate this twitch room? (kick has no way to ask, so its mod tools are
  // slash commands only and the server says no when we are not one)
  pub(super) fn can_mod(&self, p: Platform, chan: &str) -> bool {
    p == Platform::Twitch && self.me.is_some() && self.mod_in.contains(chan)
  }

  fn room_id(&self, chan: &str) -> Option<Box<str>> {
    self.rooms.iter().find(|r| &*r.0 == chan).map(|r| r.1.clone())
  }

  // the person behind a line, in the tab it was shown in
  pub(super) fn subject_of(&self, m: &Message) -> Option<Subject> {
    self.subject_for(m, &m.login, &m.nick, &m.user_id, m.color, &m.id)
  }

  // a person on a line: its sender, or (no line id, so no message actions) someone named in its words
  pub(super) fn subject_for(
    &self,
    m: &Message,
    login: &str,
    nick: &str,
    user_id: &str,
    color: u32,
    msg_id: &str,
  ) -> Option<Subject> {
    if m.kind == crate::model::message::Kind::System || login.is_empty() {
      return None;
    }
    let t = self.tabs.get(self.active)?;
    let chan: Box<str> = match m.x.as_deref().map(|x| &x.chan).filter(|c| !c.is_empty()) {
      Some(c) => c.clone(),
      None => t.sources.iter().find(|s| s.0 == m.platform).map(|s| s.1.clone())?,
    };
    let own = m.has(crate::model::message::flag::OWN)
      || self.me.as_ref().is_some_and(|me| me.names.iter().any(|n| n.eq_ignore_ascii_case(login)));
    Some(Subject {
      platform: m.platform,
      chan,
      login: login.into(),
      nick: nick.into(),
      user_id: user_id.into(),
      msg_id: msg_id.into(),
      color,
      own,
    })
  }

  fn req(&self, who: &Subject, verb: Verb, reason: Option<String>) -> Req {
    Req {
      platform: who.platform,
      chan: who.chan.clone(),
      room: (who.platform == Platform::Twitch).then(|| self.room_id(&who.chan)).flatten(),
      verb,
      login: who.login.clone(),
      user_id: (who.platform == Platform::Twitch && !who.user_id.is_empty()).then(|| who.user_id.clone()),
      msg_id: (!who.msg_id.is_empty()).then(|| who.msg_id.clone()),
      reason,
    }
  }

  // go: a ban asks first (setting), the rest just happen
  pub(super) fn mod_run(&mut self, verb: Verb, who: Vec<Subject>, reason: Option<String>) {
    if who.is_empty() {
      return;
    }
    if verb == Verb::Ban && self.set.confirm_ban {
      let head = format!("ban {} in #{}?", who[0].nick, who[0].chan);
      let at = (self.pointer.0.min(self.canvas.w - 160), self.pointer.1.min(self.canvas.h - 80));
      let todo = Todo { verb, who, reason };
      self.pop = Some(Pop::confirm(head, at, todo));
      return self.pop_changed();
    }
    self.mod_send(verb, who, reason);
  }

  pub(super) fn mod_send(&mut self, verb: Verb, who: Vec<Subject>, reason: Option<String>) {
    for w in &who {
      let req = self.req(w, verb, reason.clone());
      self.net.send(Cmd::Mod(req));
    }
    self.say(format!("{} {}…", verb.name(), who[0].nick));
  }

  // /ban and friends act on the platforms the active tab merges (twitch and kick), as the ext fans out
  pub(super) fn mod_slash(&mut self, cmd: Slash) {
    let (verb, name, reason) = match cmd {
      Slash::Usage(u) => return self.say(u),
      Slash::Do(v, n, r) => (v, n, r),
    };
    let Some(t) = self.tabs.get(self.active).filter(|t| t.kind == crate::model::tabs::Kind::Chat) else {
      return self.say(format!("/{} needs a channel tab", verb.name()));
    };
    if self.me.is_none() {
      return self.say("log in first (ctrl-l)");
    }
    let mut srcs: Vec<(Platform, Box<str>)> = t.sources.iter().filter(|s| s.0 != Platform::Youtube).cloned().collect();
    if srcs.is_empty() {
      return self.say(format!("/{} needs a twitch or kick channel", verb.name()));
    }
    // a message id lives on one platform: twitch first, as the ext does
    if verb == Verb::Delete {
      srcs.truncate(1);
    }
    let who: Vec<Subject> = srcs
      .into_iter()
      .map(|(platform, chan)| {
        let id = (platform == Platform::Twitch).then(|| t.view.user_id_of(&name)).flatten().unwrap_or_default();
        let (login, msg_id) =
          if verb == Verb::Delete { ("".into(), name.as_str().into()) } else { (name.as_str().into(), "".into()) };
        Subject { platform, chan, login, nick: name.as_str().into(), user_id: id, msg_id, color: 0, own: false }
      })
      .collect();
    if verb == Verb::Delete {
      // no name to show: the id stands in for it
      let mut who = who;
      who[0].nick = "message".into();
      return self.mod_run(verb, who, reason);
    }
    self.mod_run(verb, who, reason);
  }

  pub(super) fn on_mod_done(&mut self, d: Done) {
    let Done { platform, chan, verb, login, msg_id, res } = d;
    let who = if verb == Verb::Delete { "message".into() } else { login.to_string() };
    match res {
      Ok(()) => {
        self.say(format!("{} {who}", verb.done()));
        // twitch echoes bans, timeouts and deletes back through chat; unban never echoes, and kick echoes nothing
        let actor = self.me.as_ref().map_or_else(|| "you".to_owned(), |m| m.username.to_string());
        let note = match verb {
          Verb::Unban => Some(format!("{actor} unbanned {login}")),
          _ if platform != Platform::Kick => None,
          Verb::Ban => Some(format!("{actor} banned {login}")),
          Verb::Timeout(s) => Some(format!("{actor} timed out {login} for {s}s")),
          Verb::Purge => None,
          Verb::Delete => Some(format!("{actor} deleted a message")),
        };
        for i in self.tabs_with(platform, &chan) {
          let v = self.tabs[i].chat();
          if platform == Platform::Kick {
            match (verb, &msg_id) {
              (Verb::Delete, Some(id)) => v.clear_msg(id),
              (Verb::Unban, _) => {}
              _ => v.clear_user(&login),
            }
          }
          if let Some(n) = &note {
            v.push(Message::system(now_ms(), platform, n.as_str()));
          }
          self.touched(i);
        }
      }
      Err(Fail::Auth) => self.say("log in first (ctrl-l)"),
      Err(Fail::Relink) => {
        self.say("moderating needs a twitch permission. finish allowing it in your browser…");
        self.net.send(Cmd::Relink("mod"));
      }
      Err(Fail::Other(e)) => self.say(format!("{} failed: {e}", verb.name())),
    }
    self.bar_dirty = true;
    self.wake();
  }
}
