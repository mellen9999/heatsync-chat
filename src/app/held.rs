// the automod hold queue (ext automod-queue.js): in each open twitch room you moderate, heatsync's
// server is asked to watch automod for you. a held message shows as a yellow-edged row of its room with
// allow / deny (click, or the line menu); it settles when you or another mod act, or lapses after
// twitch's hour. the watch is renewed every half hour and again after the socket reconnects (holds
// pushed while it was down come back with the answer)
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::App;
use crate::model::held::{self, End, Hold, Seen, State};
use crate::model::message::Platform;
use crate::net::Cmd;
use crate::net::held::{Cmd as HeldCmd, Fail};
use crate::util::time::now_ms;

// the server keeps a watch alive while it hears from us; the ext renews every 25-30 min
const RENEW: Duration = Duration::from_secs(30 * 60);
// how often lapsed holds and due renewals are looked for
const TICK: Duration = Duration::from_secs(60);
const MAX_OPEN: usize = 500;
pub(super) const NEED_PACK: &str = "automod holds need a twitch permission: ctrl-l → allow automod holds";

#[derive(Default)]
pub(super) struct Queue {
  seen: Seen,
  // room → when its watch was last asked for
  watched: HashMap<Box<str>, Instant>,
  // unsettled holds with a row: id → (room, held at)
  open: HashMap<Box<str>, (Box<str>, i64)>,
  // the twitch login lacks the permission: no watching until the login changes; told once
  blocked: bool,
  told: bool,
  due: Option<Instant>,
}

impl Queue {
  pub(super) fn due(&self) -> Option<Instant> {
    self.due
  }

  // the login was checked again (a relink may have brought the permission): ask afresh
  pub(super) fn reset(&mut self) {
    self.watched.clear();
    self.blocked = false;
  }
}

impl App {
  // ask for the watch of every open twitch room we moderate that hasn't had one lately
  pub(super) fn held_sweep(&mut self) {
    let Some(me) = &self.me else { return };
    if me.twitch_id.is_none() || self.held.blocked || !self.rooms.iter().any(|r| self.can_mod(Platform::Twitch, &r.0)) {
      return;
    }
    // twitch said which permission packs this login holds, and automod is not one
    if !me.packs.is_empty() && !me.has_pack("automod") {
      return self.held_tell();
    }
    let now = Instant::now();
    let rooms: Vec<(Box<str>, Box<str>)> = self
      .rooms
      .iter()
      .filter(|(ch, _)| self.can_mod(Platform::Twitch, ch))
      .filter(|(ch, _)| self.held.watched.get(ch).is_none_or(|t| now.duration_since(*t) >= RENEW))
      .cloned()
      .collect();
    for (ch, room) in rooms {
      self.held.watched.insert(ch.clone(), now);
      self.net.send(Cmd::Held(HeldCmd::Watch(ch, room)));
    }
    self.held.due.get_or_insert(now + TICK);
  }

  // the socket is back: what was pushed while it was down is only in the watch's answer
  pub(super) fn held_reconnected(&mut self) {
    self.held.watched.clear();
    self.held_sweep();
  }

  pub(super) fn held_tick(&mut self) {
    self.held.due = None;
    let now = now_ms();
    let lapsed: Vec<Box<str>> =
      self.held.open.iter().filter(|(_, (_, at))| now - at >= held::EXPIRE_MS).map(|(id, _)| id.clone()).collect();
    for id in lapsed {
      self.held_end(&id, End::Expired);
    }
    self.held_sweep();
    if !self.held.open.is_empty() {
      self.held.due.get_or_insert(Instant::now() + TICK);
    }
  }

  // once a session, in the rooms we moderate: how to turn the queue on
  fn held_tell(&mut self) {
    if std::mem::replace(&mut self.held.told, true) {
      return;
    }
    let rooms: Vec<Box<str>> =
      self.rooms.iter().filter(|(ch, _)| self.can_mod(Platform::Twitch, ch)).map(|r| r.0.clone()).collect();
    for ch in rooms {
      for i in self.tabs_with(Platform::Twitch, &ch) {
        self.tabs[i].chat().push(crate::model::message::Message::system(now_ms(), Platform::Twitch, NEED_PACK));
        self.touched(i);
      }
    }
  }

  pub(super) fn on_held(&mut self, h: Hold) {
    let tabs = self.tabs_with(Platform::Twitch, &h.chan);
    // a room we left, or one we don't moderate (a stale push)
    if tabs.is_empty() || !self.can_mod(Platform::Twitch, &h.chan) || !self.held.seen.first(&h, now_ms()) {
      return;
    }
    if self.held.open.len() >= MAX_OPEN
      && let Some(k) = self.held.open.iter().min_by_key(|e| e.1.1).map(|e| e.0.clone())
    {
      self.held.open.remove(&k);
    }
    self.held.open.insert(h.id.clone(), (h.chan.clone(), h.held_at));
    self.held.due.get_or_insert(Instant::now() + TICK);
    let line = held::line(&h);
    for i in tabs {
      self.tabs[i].chat().push(line.clone());
      self.touched(i);
      // something waits on you: the tab turns yellow like the channel's own news
      if i != self.active && !self.tabs[i].event {
        self.tabs[i].event = true;
        self.bar_dirty = true;
      }
    }
  }

  pub(super) fn on_held_watched(&mut self, chan: Box<str>, res: Result<Vec<Hold>, Fail>) {
    match res {
      Ok(holds) => {
        for h in holds.into_iter().filter(|h| h.chan == chan) {
          self.on_held(h);
        }
      }
      Err(Fail::Relink) => {
        self.held.blocked = true;
        self.held_tell();
      }
      // asked again on the next tick
      Err(Fail::Other(_)) => {
        self.held.watched.remove(&chan);
      }
      // logged out, or not a mod after all: the next login / renewal asks again
      Err(_) => {}
    }
  }

  // a hold settled (or lapsed): its rows say how, and its buttons go
  pub(super) fn held_end(&mut self, id: &str, end: End) {
    self.held.seen.settled(id, now_ms());
    let chan = self.held.open.remove(id).map(|o| o.0);
    self.held_set(id, chan.as_deref(), &State::Ended(end));
  }

  pub(super) fn on_held_end(&mut self, chan: Box<str>, id: Box<str>, end: End) {
    self.held.open.insert(id.clone(), (chan, 0));
    self.held_end(&id, end);
  }

  fn held_set(&mut self, id: &str, chan: Option<&str>, st: &State) {
    let Some(chan) = chan else { return };
    for i in self.tabs_with(Platform::Twitch, chan) {
      if self.tabs[i].chat().set_held(id, st) && i == self.active {
        self.wake();
      }
    }
  }

  // allow (true) or deny the hold `id`
  pub(super) fn held_act(&mut self, id: &str, allow: bool) {
    let Some(chan) = self.held.open.get(id).map(|o| o.0.clone()) else { return };
    self.held_set(id, Some(&chan), &State::Busy);
    self.net.send(Cmd::Held(HeldCmd::Act(chan, id.into(), allow)));
  }

  pub(super) fn on_held_acted(&mut self, chan: Box<str>, id: Box<str>, allow: bool, res: Result<(), Fail>) {
    // the confirming push names who did it; until then (or if it never comes) the answer settles it
    let why = match res {
      Ok(()) => return self.held_end(&id, if allow { End::Allowed(None) } else { End::Denied(None) }),
      Err(Fail::Gone) => return self.held_end(&id, End::Expired),
      Err(Fail::Relink) => {
        self.say(super::feed::needs_twitch("automod holds"));
        self.net.send(Cmd::Relink("automod"));
        "needs a twitch permission"
      }
      Err(Fail::Auth) => "log in again (ctrl-l)",
      Err(Fail::NotMod) => "you're not a mod here",
      Err(Fail::Other(_)) => "action failed",
    };
    if self.held.open.contains_key(&id) {
      self.held_set(&id, Some(&chan), &State::Failed(why.into()));
    }
  }

  // a click on a hold's allow / deny. true = the click was spent
  pub(super) fn held_press(&mut self) -> bool {
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    let res = &mut self.res;
    let Some((id, allow)) = self.tabs.get_mut(self.active).and_then(|t| t.view.held_at(chat, x, y, res)) else {
      return false;
    };
    self.held_act(&id, allow);
    true
  }

  // the selected line is an open hold: its id
  pub(super) fn held_selected(&self) -> Option<Box<str>> {
    let m = self.tabs.get(self.active)?.view.selected()?;
    let h = m.x.as_ref()?.held.as_ref().filter(|h| h.open())?;
    self.held.open.contains_key(&h.id).then(|| h.id.clone())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::irc::Chat;
  use crate::net::{Net, NetEvent};
  use serde_json::json;

  fn me(packs: &[&str]) -> crate::net::auth::Me {
    crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(1),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: packs.iter().map(|&p| p.into()).collect(),
    }
  }

  fn hold(id: &str, chan: &str, ago_ms: i64) -> Hold {
    let v = json!({"msgId": id, "broadcasterLogin": chan, "senderId": "77", "senderLogin": "bob",
      "senderName": "Bob", "text": "you are a dummy", "heldAt": now_ms() - ago_ms,
      "reason": "blocked_term", "terms": ["dummy"]});
    held::hold(&v, now_ms()).unwrap()
  }

  fn app(packs: &[&str]) -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>, usize) {
    let (net, asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    app.me = Some(me(packs));
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
    (app, asked, tab)
  }

  fn watches(asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(Box<str>, Box<str>)> {
    std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| if let Cmd::Held(HeldCmd::Watch(ch, room)) = c { Some((ch, room)) } else { None })
      .collect()
  }

  fn state(app: &mut App, tab: usize) -> Option<State> {
    Some(app.tabs[tab].chat().last_msg()?.x.as_ref()?.held.as_ref()?.state.clone())
  }

  // where the open hold's allow / deny sits on screen
  fn button(app: &mut App, tab: usize, allow: bool) -> Option<(i32, i32)> {
    app.redraw();
    let area = app.chat_area();
    (area.y..area.bottom()).step_by(2).find_map(|y| {
      (area.x..area.right()).step_by(2).find_map(|x| {
        let res = &mut app.res;
        app.tabs[tab].view.held_at(area, x, y, res).filter(|h| h.1 == allow).map(|_| (x, y))
      })
    })
  }

  #[test]
  fn a_room_you_moderate_is_watched_and_its_holds_are_allowed_from_the_row() {
    let (mut app, mut asked, tab) = app(&["base", "automod"]);
    app.on_chat("forsen", Chat::Room { id: "22484632".into(), modes: Vec::new() });
    assert!(watches(&mut asked).is_empty(), "not a mod there (yet)");
    app.on_chat("forsen", Chat::Mod(true));
    assert_eq!(watches(&mut asked), vec![("forsen".into(), "22484632".into())]);
    app.held_sweep();
    assert!(watches(&mut asked).is_empty(), "renewed every half hour, not on every look");
    // the watch hands back what is still held; a replay of it over the socket is the same row
    let h = hold("aaa-1", "forsen", 60_000);
    app.on_net(NetEvent::HeldWatched { chan: "forsen".into(), res: Ok(vec![h.clone()]) });
    app.on_net(NetEvent::Held(Box::new(h)));
    let rows = |app: &mut App| app.tabs[tab].chat().len();
    assert_eq!(rows(&mut app), 1);
    assert_eq!(state(&mut app, tab), Some(State::Open));
    let yellow = app.canvas.px.iter().filter(|&&p| p == crate::ui::theme::YELLOW).count();
    let at = button(&mut app, tab, true).expect("an allow button on the row");
    assert!(app.canvas.px.iter().filter(|&&p| p == crate::ui::theme::YELLOW).count() > yellow, "drawn in yellow");
    assert!(button(&mut app, tab, false).is_some_and(|d| d.0 > at.0), "deny right of allow");
    app.pointer = at;
    app.click();
    let acts: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| if let Cmd::Held(HeldCmd::Act(ch, id, a)) = c { Some((ch, id, a)) } else { None })
      .collect();
    assert_eq!(acts, vec![("forsen".into(), "aaa-1".into(), true)]);
    assert_eq!(state(&mut app, tab), Some(State::Busy));
    assert!(button(&mut app, tab, true).is_none(), "no second press while it is asked");
    app.on_net(NetEvent::HeldActed { chan: "forsen".into(), id: "aaa-1".into(), allow: true, res: Ok(()) });
    assert_eq!(state(&mut app, tab), Some(State::Ended(End::Allowed(None))));
    // the server's push names who did it
    let end = End::Allowed(Some("mellen".into()));
    app.on_net(NetEvent::HeldEnd { chan: "forsen".into(), id: "aaa-1".into(), end: end.clone() });
    assert_eq!(state(&mut app, tab), Some(State::Ended(end)));
    // settled: the next backlog doesn't bring it back
    app.on_net(NetEvent::HeldWatched { chan: "forsen".into(), res: Ok(vec![hold("aaa-1", "forsen", 60_000)]) });
    assert_eq!(rows(&mut app), 1);
  }

  #[test]
  fn a_failed_press_says_why_and_offers_the_buttons_again_and_an_hour_lapses_it() {
    let (mut app, _asked, tab) = app(&["base", "automod"]);
    app.on_chat("forsen", Chat::Room { id: "22484632".into(), modes: Vec::new() });
    app.on_chat("forsen", Chat::Mod(true));
    app.on_net(NetEvent::Held(Box::new(hold("bbb-2", "forsen", held::EXPIRE_MS - 1000))));
    app.held_act("bbb-2", false);
    let fail = Err(Fail::Other("http 502".into()));
    app.on_net(NetEvent::HeldActed { chan: "forsen".into(), id: "bbb-2".into(), allow: false, res: fail });
    assert_eq!(state(&mut app, tab), Some(State::Failed("action failed".into())));
    assert!(button(&mut app, tab, false).is_some(), "deny again");
    // twitch's hour runs out
    app.held.open.get_mut("bbb-2").unwrap().1 -= 1000;
    app.held_tick();
    assert_eq!(state(&mut app, tab), Some(State::Ended(End::Expired)));
    assert!(button(&mut app, tab, true).is_none());
    // a hold already past its hour, or in a room we don't moderate, gets no row
    app.on_net(NetEvent::Held(Box::new(hold("ccc-3", "forsen", held::EXPIRE_MS))));
    app.on_net(NetEvent::Held(Box::new(hold("ddd-4", "xqc", 0))));
    let xqc = app.tabs.iter().position(|t| &*t.name == "xqc").unwrap();
    assert_eq!(state(&mut app, tab), Some(State::Ended(End::Expired)));
    assert!(state(&mut app, xqc).is_none());
  }

  #[test]
  fn without_the_permission_nothing_is_asked_and_you_are_told_how_once() {
    let (mut app, mut asked, tab) = app(&["base"]);
    app.on_chat("forsen", Chat::Room { id: "22484632".into(), modes: Vec::new() });
    app.on_chat("forsen", Chat::Mod(true));
    app.held_sweep();
    assert!(watches(&mut asked).is_empty());
    let said = |app: &mut App| app.tabs[tab].chat().len();
    assert_eq!(said(&mut app), 1);
    assert_eq!(&*app.tabs[tab].chat().last_msg().unwrap().body, NEED_PACK);
    // the server can say so too (permissions unknown here): told once, then quiet until a new login
    app.me = Some(me(&[]));
    app.held.reset();
    app.held.told = false;
    app.held_sweep();
    assert_eq!(watches(&mut asked).len(), 1);
    app.on_net(NetEvent::HeldWatched { chan: "forsen".into(), res: Err(Fail::Relink) });
    app.held.watched.clear();
    app.held_sweep();
    assert!(watches(&mut asked).is_empty(), "blocked until the login is checked again");
    assert_eq!(said(&mut app), 2);
  }
}
