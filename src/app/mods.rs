// moderation (ext mod-toolbar.js): when our twitch badges say we moderate a room, a line's menu offers
// delete / timeout / purge / ban / unban, and /ban /timeout /unban /delete /purge work in the tab.
// heatsync's server does the twitch and kick calls (POST /api/mod/action)
use super::App;
use super::overlay::Overlay;
use super::pop::{Pop, Subject, Todo};
use crate::cosmetics::id::PaintId;
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
      self.held_sweep();
    } else if !on && had {
      self.mod_in.remove(ch);
    }
  }

  // do we moderate this room? twitch: our badges say so. kick: our own lines there wear a moderator
  // or broadcaster badge, or it is our own channel. youtube: never (heatsync has no youtube moderation)
  pub(super) fn can_mod(&self, p: Platform, chan: &str) -> bool {
    let Some(me) = &self.me else { return false };
    match p {
      Platform::Twitch => self.mod_in.contains(chan),
      Platform::Kick => {
        self.kick_mod.contains(chan) || me.kick.as_deref().is_some_and(|k| crate::model::modrole::kick_slug(k) == chan)
      }
      Platform::Youtube => false,
    }
  }

  // a live kick line of ours tells whether we moderate that room; a line without the badge takes it
  // back (we were unmodded), like twitch's USERSTATE does
  pub(super) fn kick_role_seen(&mut self, slug: &str, msgs: &[Message]) {
    let mine = self.me.as_ref().and_then(|m| m.kick.as_deref());
    let Some(own) = msgs.iter().rev().find(|m| m.kind.said() && crate::model::modrole::kick_is_me(&m.login, mine))
    else {
      return;
    };
    if crate::model::modrole::kick_badges_mod(&own.badges) {
      self.kick_mod.insert(slug.into());
    } else {
      self.kick_mod.remove(slug);
    }
  }

  // one line in the mod log tab: the room, then what happened
  pub(super) fn mod_log(&mut self, p: Platform, chan: &str, what: String) {
    let Some(i) = self.tab_of(crate::model::tabs::Kind::ModLog) else { return };
    self.tabs[i].chat().push(Message::system(now_ms(), p, format!("{chan} · {what}")));
    if i != self.active {
      self.tabs[i].unread = true;
      self.bar_dirty = true;
    }
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
    if !m.kind.said() || login.is_empty() {
      return None;
    }
    let chan = self.room_of(m)?;
    let own = m.has(crate::model::message::flag::OWN)
      || self.me.as_ref().is_some_and(|me| me.names.iter().any(|n| n.eq_ignore_ascii_case(login)));
    Some(Subject {
      platform: m.platform,
      chan,
      login: login.into(),
      nick: nick.into(),
      user_id: user_id.into(),
      msg_id: msg_id.into(),
      color: crate::model::color::readable(color),
      own,
      paint: (login == &*m.login).then(|| PaintId::of_message(m)).flatten(),
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
    // the ext asks before a ban when confirming is on or it has reasons to offer
    let reasons: Vec<String> = crate::model::filter::with(|f| f.reasons().iter().map(|r| r.to_string()).collect());
    if verb == Verb::Ban && (self.set.confirm_ban || !reasons.is_empty()) {
      let head = format!("ban {} in #{}?", who[0].nick, who[0].chan);
      let at = (self.pointer.0.min(self.canvas.w - 160), self.pointer.1.min(self.canvas.h - 80));
      let todo = Todo { verb, who, reason, reasons };
      self.overlay = Some(Overlay::Pop(Pop::confirm(head, at, todo)));
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
        Subject {
          platform,
          chan,
          login,
          nick: name.as_str().into(),
          user_id: id,
          msg_id,
          color: 0,
          own: false,
          paint: None,
        }
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
        // twitch's own echo logs a ban, timeout or delete; an unban and anything on kick only show here
        if let Some(n) = note {
          self.mod_log(platform, &chan, n);
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::filter;
  use crate::net::Net;
  use crate::net::irc::Chat;
  use winit::keyboard::Key;

  fn bans(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(String, Option<String>)> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| match c {
        Cmd::Mod(r) if r.verb == Verb::Ban => Some((r.login.to_string(), r.reason)),
        _ => None,
      })
      .collect()
  }

  fn room() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    let (net, rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
    app.me = Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(1),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
    app.on_chat("forsen", Chat::Mod(true));
    (app, rx)
  }

  fn typed(app: &mut App, s: &str) {
    app.line.set_text(s.into());
    app.send();
  }

  #[test]
  fn ban_reasons_make_a_ban_ask_and_a_pick_sends_it_with_the_reason() {
    filter::edit(|f| f.set_reasons_text(""));
    let (mut app, mut rx) = room();
    // no reasons and no confirm: straight through, as before
    typed(&mut app, "/ban bob");
    assert_eq!(bans(&mut rx), [("bob".to_owned(), None)]);
    typed(&mut app, "/banreason spam");
    typed(&mut app, "/banreason hate speech");
    assert_eq!(filter::with(|f| f.reasons_text()), "spam\nhate speech");
    // now it asks first; nothing goes until a pick
    typed(&mut app, "/ban bob");
    assert!(bans(&mut rx).is_empty());
    let rows: Vec<String> = app.pop().unwrap().menu.items.iter().map(|i| i.1.clone()).collect();
    assert_eq!(rows, ["yes, ban", "1 spam", "2 hate speech", "cancel"]);
    // j j enter: the second reason
    for k in ["j", "j"] {
      app.pop_key(Key::Character(k));
    }
    app.pop_key(Key::Named(winit::keyboard::NamedKey::Enter));
    assert_eq!(bans(&mut rx), [("bob".to_owned(), Some("hate speech".to_owned()))]);
    // a digit picks at once; one past the list does nothing
    typed(&mut app, "/ban amy");
    app.pop_key(Key::Character("5"));
    assert!(app.pop().is_some() && bans(&mut rx).is_empty());
    app.pop_key(Key::Character("1"));
    assert_eq!(bans(&mut rx), [("amy".to_owned(), Some("spam".to_owned()))]);
    // enter on the first row bans with the reason typed, if any
    typed(&mut app, "/ban cy rule 3");
    app.pop_key(Key::Named(winit::keyboard::NamedKey::Enter));
    assert_eq!(bans(&mut rx), [("cy".to_owned(), Some("rule 3".to_owned()))]);
    // timeouts never ask
    typed(&mut app, "/timeout bob 10m");
    assert!(app.pop().is_none());
    // emptied (say from the account), it goes straight through again
    filter::edit(|f| f.set_reasons_text(""));
    typed(&mut app, "/ban dee");
    assert_eq!(bans(&mut rx), [("dee".to_owned(), None)]);
  }
}
