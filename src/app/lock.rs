// the message box says why you can't chat here (banned, timed out with the time left, a room mode you don't
// meet), per room: a tab with twitch + kick locks only the leg that is locked, the one the box would send to.
// model::lock has the signals and words; this wires them to the box, the once-a-second countdown and the send
use super::{App, now_ms};
use crate::model::lock::{Cause, Lock};
use crate::model::message::Platform;
use crate::model::tabs::Kind;
use crate::ui::theme;

impl App {
  // the lock on the leg the box sends to (chat tabs only; a signed-out box has nothing to be locked from)
  pub(super) fn lock_here(&self) -> Option<Lock> {
    if self.me.is_none() || self.kind() != Kind::Chat || self.logs_shown().is_some() || self.feedish() {
      return None;
    }
    let (p, key, _) = self.target()?;
    self.locks.get(p, key, now_ms()).copied()
  }

  // red = out of the room, yellow = a rule of the room
  pub(super) fn lock_ink(l: Lock) -> u32 {
    if l.out() { theme::RED } else { theme::YELLOW }
  }

  fn lock_changed(&mut self) {
    self.input_dirty = true;
    self.wake();
  }

  // twitch or kick said a line of ours was refused for a lasting reason (Some), or got through (None)
  pub(super) fn lock_told(&mut self, p: Platform, chan: &str, cause: Option<Cause>) {
    let Some(c) = cause else {
      if self.locks.clear(p, chan) {
        self.lock_changed();
      }
      return;
    };
    let now = now_ms();
    let slow = (p == Platform::Twitch).then(|| self.modes.get(chan).and_then(|m| m.slow)).flatten();
    self.locks.offer(p, chan, Lock::new(c, now, slow), now);
    self.lock_changed();
  }

  // CLEARCHAT: when its target is us, the room itself tells us we are banned, or timed out for how long
  pub(super) fn lock_clearchat(&mut self, chan: &str, login: &str, secs: Option<u32>) {
    let mine =
      self.me.as_ref().and_then(|m| m.name_on(Platform::Twitch)).is_some_and(|n| n.eq_ignore_ascii_case(login));
    if !mine || secs == Some(0) {
      return;
    }
    let c = secs.map_or(Cause::Banned, Cause::Timeout);
    let now = now_ms();
    self.locks.put(Platform::Twitch, chan, Lock::new(c, now, None), now);
    self.lock_changed();
  }

  // ROOMSTATE: a mode turned off frees whoever it was keeping out
  pub(super) fn lock_modes(&mut self, chan: &str, sets: &[crate::model::modes::Set]) {
    if self.locks.modes(chan, sets) {
      self.lock_changed();
    }
  }

  // the countdown crossed a second (or ended): the box repaints, and re-arms itself if one is still running
  pub(super) fn lock_second(&mut self) {
    self.lock_sec = None;
    self.lock_changed();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::auth::Me;
  use crate::net::irc::Chat;
  use crate::net::{Cmd, Net, NetEvent};

  fn me(name: &str) -> Me {
    Me {
      username: name.into(),
      platform: "twitch".into(),
      twitch: Some(name.into()),
      twitch_id: None,
      names: vec![name.into()],
      kick: Some("mellenk".into()),
      youtube: None,
      packs: vec![],
    }
  }

  fn boot(chans: &[&str]) -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, chans.iter().map(|c| (*c).into()).collect(), false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(520, 300);
    app.me = Some(me("mellen"));
    app.redraw();
    while rx.try_recv().is_ok() {}
    (app, rx)
  }

  fn twitch(app: &mut App, ch: &str, c: Chat) {
    app.on_chat(ch, c);
    app.redraw();
  }

  #[test]
  fn a_timeout_aimed_at_you_fills_the_box_and_holds_the_send() {
    let (mut app, mut rx) = boot(&["forsen"]);
    assert_eq!(app.canvas.inked(theme::RED), 0);
    // someone else's timeout is not ours
    twitch(&mut app, "forsen", Chat::ClearUser { login: "other".into(), secs: Some(60) });
    assert!(app.lock_here().is_none());
    twitch(&mut app, "forsen", Chat::ClearUser { login: "mellen".into(), secs: Some(90) });
    assert!(app.hint().starts_with("you are timed out · "), "{}", app.hint());
    let red = app.canvas.inked(theme::RED);
    assert!(red > 0, "the lock is drawn red");
    app.line.set_text("hello".into());
    app.send();
    assert!(rx.try_recv().is_err(), "nothing went out");
    assert_eq!(app.line.text, "hello", "the words stay");
    // a once-a-second redraw is armed only while it counts
    app.line.set_text(String::new());
    app.redraw();
    assert!(app.lock_sec.is_some());
    app.lock_second();
    assert!(app.lock_sec.is_none());
    app.redraw();
    assert!(app.lock_sec.is_some());
    // it ends on its own
    let n = now_ms();
    app.locks.put(Platform::Twitch, "forsen", Lock { cause: Cause::Timeout(1), until: Some(n - 1) }, n - 5000);
    app.lock_second();
    app.redraw();
    assert!(app.lock_here().is_none() && app.lock_sec.is_none());
    assert!(app.hint().starts_with("type to chat"));
    app.line.set_text("hello".into());
    app.send();
    assert!(matches!(rx.try_recv(), Ok(Cmd::Send { .. })));
  }

  #[test]
  fn a_ban_tells_but_still_lets_twitch_judge_a_send() {
    let (mut app, mut rx) = boot(&["forsen"]);
    twitch(&mut app, "forsen", Chat::ClearUser { login: "mellen".into(), secs: None });
    assert_eq!(app.hint(), "you are banned from this channel");
    assert!(app.canvas.inked(theme::RED) > 0);
    app.line.set_text("hi".into());
    app.send();
    assert!(matches!(rx.try_recv(), Ok(Cmd::Send { .. })), "unbans never tell us, so twitch decides");
    // a line getting through proves we are in
    twitch(&mut app, "forsen", Chat::Sent);
    assert!(app.lock_here().is_none());
  }

  #[test]
  fn a_refused_line_names_the_room_rule_and_roomstate_frees_it() {
    use crate::model::modes::{Mode, Set};
    let (mut app, _rx) = boot(&["forsen"]);
    twitch(&mut app, "forsen", Chat::Lock(Cause::Subs));
    assert_eq!(app.hint(), "subscribers-only — sub to chat here");
    assert_eq!(app.canvas.inked(theme::RED), 0);
    assert!(app.canvas.inked(theme::YELLOW) > 0, "a room rule is yellow");
    twitch(&mut app, "forsen", Chat::Room { id: "1".into(), modes: vec![Set::new(Mode::Subs, None)] });
    assert!(app.lock_here().is_none());
    assert!(app.hint().starts_with("type to chat"));
  }

  #[test]
  fn each_leg_of_a_merged_tab_locks_alone() {
    let (mut app, _rx) = boot(&["forsen kick:xqc"]);
    app.on_net(NetEvent::Lock(Platform::Kick, "xqc".into(), Some(Cause::Banned)));
    app.redraw();
    // the box sends to the first leg (twitch): not locked
    assert_eq!(app.target().map(|t| t.0), Some(Platform::Twitch));
    assert!(app.lock_here().is_none());
    app.tabs[app.active].target = 1;
    app.input_dirty = true;
    assert_eq!(app.target().map(|t| t.0), Some(Platform::Kick));
    assert_eq!(app.lock_here().map(|l| l.cause), Some(Cause::Banned));
    // a kick send that went through frees it
    app.on_net(NetEvent::Lock(Platform::Kick, "xqc".into(), None));
    assert!(app.lock_here().is_none());
  }

  #[test]
  fn a_new_login_starts_unlocked() {
    let (mut app, _rx) = boot(&["forsen"]);
    twitch(&mut app, "forsen", Chat::Lock(Cause::Banned));
    app.on_net(NetEvent::Me(Some(me("other"))));
    assert!(app.lock_here().is_none());
  }
}
