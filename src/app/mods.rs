// moderation (ext mod-toolbar.js): when our twitch badges say we moderate a room, a line's menu offers
// delete / timeout / purge / ban / unban, and /ban /timeout /unban /delete /purge work in the tab.
// heatsync's server does the twitch and kick calls (POST /api/mod/action)
use super::App;
use super::overlay::Overlay;
use super::pop::Act;
use super::pop::{Pop, Subject, Todo};
use crate::cosmetics::id::PaintId;
use crate::i18n::{f, t, tn};
use crate::model::message::{Message, Platform};
use crate::model::modcmd::{Cmd as Slash, NUKE_MAX, Verb};
use crate::model::modlog_file;
use crate::net::Cmd;
use crate::net::modact::{Done, Fail, NukeDone, Req};
use crate::store;
use crate::util::time::now_ms;

// how long an action of ours waits for twitch's echo of it to be named
const MOD_ECHO_MS: i64 = 30_000;
// how much of a long term the question shows
const NUKE_TERM_SHOWN: usize = 40;

impl App {
  pub(super) fn on_mod_state(&mut self, ch: &str, on: bool) {
    let had = self.mod_in.contains(ch);
    if on && !had {
      self.mod_in.insert(ch.into());
      self.mod_buttons_seed();
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
      if self.kick_mod.insert(slug.into()) {
        self.mod_buttons_seed();
      }
    } else {
      self.kick_mod.remove(slug);
    }
  }

  // one line in the mod log tab: the room, then what happened. an action you took through heatsync names you
  // (twitch's echo of it does not say who did it, so a recent one of ours that matches is the one). kept in the
  // `modlog` file, so a restart brings it back
  pub(super) fn mod_log(&mut self, p: Platform, chan: &str, what: String) {
    let Some(i) = self.tab_of(crate::model::tabs::Kind::ModLog) else { return };
    let what = match self.mod_actor(chan, &what) {
      Some(by) => f("mod.log.by", &[("what", &what), ("by", &by)]),
      None => what,
    };
    self.tabs[i].chat().push(Message::system(now_ms(), p, format!("{chan} \u{b7} {what}")));
    self.modlog_save(i);
    if i != self.active {
      self.tabs[i].unread = true;
      self.bar_dirty = true;
    }
  }

  // an action of ours is waiting for its echo in `chan`: the person it was for
  pub(super) fn mod_expect(&mut self, chan: &str, login: &str) {
    let now = now_ms();
    self.mod_by.retain(|e| now - e.2 < MOD_ECHO_MS);
    if self.mod_by.len() >= 16 {
      self.mod_by.remove(0);
    }
    self.mod_by.push((chan.into(), login.to_lowercase().into(), now));
  }

  // the echo `what` in `chan` is one of ours: its actor (our username), the wait ended
  fn mod_actor(&mut self, chan: &str, what: &str) -> Option<String> {
    let now = now_ms();
    self.mod_by.retain(|e| now - e.2 < MOD_ECHO_MS);
    let low = what.to_lowercase();
    let at = self.mod_by.iter().position(|e| &*e.0 == chan && !e.1.is_empty() && low.contains(&*e.1))?;
    self.mod_by.remove(at);
    self.me.as_ref().map(|m| m.username.to_string()).filter(|u| !u.is_empty())
  }

  // the tab's rows, written soon (the store batches a burst into one write)
  fn modlog_save(&self, i: usize) {
    let v = &self.tabs[i].view;
    let text = modlog_file::encode(v.msgs().filter(|m| !m.body.is_empty()).map(|m| (m.ts, m.platform, &*m.body)));
    if text.is_empty() {
      return;
    }
    store::write_later(modlog_file::FILE, &text);
  }

  // at boot: the last run's mod log comes back as history. a file with unusable lines is written again without
  // them; one that is not ours is set aside, not read
  pub(super) fn modlog_restore(&mut self) {
    let Some(i) = self.tab_of(crate::model::tabs::Kind::ModLog) else { return };
    let Some(text) = store::read_capped(modlog_file::FILE, modlog_file::MAX_BYTES) else { return };
    let (rows, junk) = modlog_file::decode(&text);
    if rows.is_empty() && junk > 0 {
      return store::set_aside(modlog_file::FILE, None);
    }
    if junk > 0 {
      store::set_aside(modlog_file::FILE, Some(junk));
    }
    self.tabs[i].chat().prepend(rows.into_iter().map(|(ts, p, x)| Message::system(ts, p, x)).collect());
    if junk > 0 {
      self.modlog_save(i);
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
      let head = f("mod.ban.ask", &[("nick", &who[0].nick), ("chan", &who[0].chan)]);
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
    self.say(f("mod.send", &[("verb", verb.word()), ("who", &who[0].nick)]));
  }

  // the twitch and kick feeds of the channel tab on screen that a mod command acts on; says why when there are none
  fn mod_srcs(&mut self, what: &str) -> Option<Vec<(Platform, Box<str>)>> {
    let Some(tab) = self.tabs.get(self.active).filter(|t| t.kind == crate::model::tabs::Kind::Chat) else {
      self.say(f("mod.needs_tab", &[("cmd", what)]));
      return None;
    };
    if self.me.is_none() {
      self.say(t("mod.common.login_first"));
      return None;
    }
    let srcs: Vec<(Platform, Box<str>)> = tab.sources.iter().filter(|s| s.0 != Platform::Youtube).cloned().collect();
    if srcs.is_empty() {
      self.say(f("mod.needs_channel", &[("cmd", what)]));
      return None;
    }
    Some(srcs)
  }

  // /ban and friends act on the platforms the active tab merges (twitch and kick), as the ext fans out
  pub(super) fn mod_slash(&mut self, cmd: Slash) {
    let (verb, name, reason) = match cmd {
      Slash::Usage(u) => return self.say(u),
      Slash::Nuke(n) => return self.nuke_slash(n),
      Slash::Do(v, n, r) => (v, n, r),
    };
    let Some(mut srcs) = self.mod_srcs(verb.name()) else { return };
    let Some(tab) = self.tabs.get(self.active) else { return };
    // a message id lives on one platform: twitch first, as the ext does
    if verb == Verb::Delete {
      srcs.truncate(1);
    }
    let who: Vec<Subject> = srcs
      .into_iter()
      .map(|(platform, chan)| {
        let id = (platform == Platform::Twitch).then(|| tab.view.user_id_of(&name)).flatten().unwrap_or_default();
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
      who[0].nick = t("mod.word.message").into();
      return self.mod_run(verb, who, reason);
    }
    self.mod_run(verb, who, reason);
  }

  // /nuke <term> [seconds]: the lines of this tab that say the term lately, asked about before they go
  fn nuke_slash(&mut self, parsed: Option<(String, u32)>) {
    let Some(srcs) = self.mod_srcs("nuke") else { return };
    let Some((term, secs)) = parsed else { return self.say(t("mod.cmd.nuke_usage")) };
    let (needle, since) = (term.to_lowercase(), now_ms() - i64::from(secs) * 1000);
    let Some(tab) = self.tabs.get(self.active) else { return };
    let mut seen = std::collections::HashSet::new();
    let mut who: Vec<Subject> = Vec::new();
    for m in tab.view.msgs() {
      // a line of someone's, still standing, in the window, that says the term (a substring, not a pattern)
      if !m.kind.said()
        || m.platform == Platform::Youtube
        || m.id.is_empty()
        || m.flags & crate::model::message::flag::CLEARED != 0
        || m.ts < since
        || !m.body.to_lowercase().contains(&needle)
        || !seen.insert(m.id.clone())
      {
        continue;
      }
      let Some(chan) = self.room_of(m) else { continue };
      who.push(Subject {
        platform: m.platform,
        chan,
        login: m.login.clone(),
        nick: t("mod.word.message").into(),
        user_id: "".into(),
        msg_id: m.id.clone(),
        color: 0,
        own: false,
        paint: None,
      });
    }
    let total = who.len();
    if total == 0 {
      return self.say(f("mod.nuke.none", &[("term", &term), ("secs", &secs.to_string())]));
    }
    // the oldest are kept if there are more than the cap: a raid's first lines go first
    who.truncate(NUKE_MAX);
    let n = who.len();
    let capped = if total > n { f("mod.nuke.capped", &[("total", &total.to_string())]) } else { String::new() };
    let shown: String = term.chars().take(NUKE_TERM_SHOWN).collect();
    let shown = if shown.len() < term.len() { format!("{shown}\u{2026}") } else { shown };
    let head = tn("mod.nuke.ask", n as u64, &[("capped", &capped), ("shown", &shown), ("chan", &srcs[0].1)]);
    let at = (self.pointer.0.min(self.canvas.w - 160), self.pointer.1.min(self.canvas.h - 80));
    let todo = Todo { verb: Verb::Delete, who, reason: Some(term), reasons: Vec::new() };
    self.overlay = Some(Overlay::Pop(Pop::confirm_as(head, at, (Act::NukeYes, t("mod.verb.delete").to_owned()), todo)));
    self.pop_changed();
  }

  // "nuke": every delete goes to the net, which paces them; the summary comes back as `NukeDone`
  pub(super) fn nuke_go(&mut self, who: Vec<Subject>, term: String) {
    if who.is_empty() {
      return;
    }
    let reqs: Vec<Req> = who.iter().map(|w| self.req(w, Verb::Delete, None)).collect();
    self.say(tn("mod.nuke.deleting", reqs.len() as u64, &[]));
    self.net.send(Cmd::Nuke(reqs, term));
  }

  pub(super) fn on_nuke_done(&mut self, d: NukeDone) {
    let NukeDone { term, total, ok, fail } = d;
    // twitch echoes each delete back through chat; kick echoes nothing, so its lines go grey here
    for (platform, chan, id) in ok.iter().filter(|o| o.0 == Platform::Kick) {
      for i in self.tabs_with(*platform, chan) {
        self.tabs[i].chat().clear_msg(id);
        self.touched(i);
      }
    }
    match fail {
      Some(Fail::Auth) => self.say(t("mod.common.login_first")),
      Some(Fail::Relink) => {
        self.say(super::feed::needs_twitch(t("mod.noun")));
        self.net.send(Cmd::Relink("mod"));
      }
      _ => {
        let n = ok.len();
        self.say(f("mod.nuke.done", &[("n", &n.to_string()), ("total", &total.to_string()), ("term", &term)]));
        if let Some(Fail::Other(e)) = fail
          && n < total
        {
          self.say(f("mod.nuke.failed", &[("err", &e)]));
        }
      }
    }
    self.bar_dirty = true;
    self.wake();
  }

  pub(super) fn on_mod_done(&mut self, d: Done) {
    let Done { platform, chan, verb, login, msg_id, res } = d;
    let who = if verb == Verb::Delete { t("mod.word.message").into() } else { login.to_string() };
    match res {
      Ok(()) => {
        self.say(f("mod.done", &[("what", &verb.done()), ("who", &who)]));
        // twitch echoes bans, timeouts and deletes back through chat; unban never echoes, and kick echoes nothing
        let actor = self.me.as_ref().map_or_else(|| t("mod.word.you").to_owned(), |m| m.username.to_string());
        let note = match verb {
          Verb::Unban => Some(f("mod.note.unbanned", &[("actor", &actor), ("login", &login)])),
          _ if platform != Platform::Kick => None,
          Verb::Ban => Some(f("mod.note.banned", &[("actor", &actor), ("login", &login)])),
          Verb::Timeout(s) => {
            Some(f("mod.note.timed_out", &[("actor", &actor), ("login", &login), ("n", &s.to_string())]))
          }
          Verb::Purge => None,
          Verb::Delete => Some(f("mod.note.deleted", &[("actor", &actor)])),
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
        match note {
          Some(n) => self.mod_log(platform, &chan, n),
          None => self.mod_expect(&chan, &login),
        }
      }
      Err(Fail::Auth) => self.say(t("mod.common.login_first")),
      Err(Fail::Relink) => {
        self.say(super::feed::needs_twitch(t("mod.noun")));
        self.net.send(Cmd::Relink("mod"));
      }
      Err(Fail::Other(e)) => self.say(f("mod.failed", &[("verb", verb.word()), ("err", &e)])),
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
