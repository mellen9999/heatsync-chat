// when the network side comes back from trouble: lines a feed said while it was down are slotted in
// among the ones on screen, and esc gives up a sign-in that is still waiting on the browser
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::message::{Message, Platform, flag};
use crate::net::Cmd;

impl App {
  // what a feed said while it was down (twitch irc dropped, a youtube subscription resumed): slotted
  // into every tab carrying it by time, skipping the lines already there. tinted for a ping but, like
  // history, no sound or mentions entry for something that's already old
  pub(super) fn on_catch_up(&mut self, p: Platform, key: &str, mut msgs: Vec<Message>) {
    for m in &mut msgs {
      m.flags |= flag::HISTORY;
      self.res.chatters.heard(m);
      self.scan_mention(p, key, m);
      self.hs_origin(p, key, m);
    }
    for i in self.tabs_with(p, key) {
      if self.tabs[i].chat().fill_gap(msgs.clone()) > 0 && i == self.active {
        self.wake();
      }
    }
    self.trim_soon();
  }

  // esc while a sign-in waits on the browser gives it up. true = the key was that
  pub(super) fn login_esc(&mut self, k: &Key<&str>) -> bool {
    if !self.login_wait || *k != Key::Named(NamedKey::Escape) {
      return false;
    }
    self.login_wait = false;
    self.net.send(Cmd::CancelLogin);
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::{Net, NetEvent};

  fn line(id: &str, ts: i64, body: &str) -> Message {
    let mut m = Message::system(ts, Platform::Twitch, body);
    m.kind = crate::model::message::Kind::Chat;
    m.id = id.into();
    m.nick = "bob".into();
    m.login = "bob".into();
    m
  }

  fn app() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    let (net, asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    (app, asked)
  }

  fn shown(app: &mut App) -> Vec<String> {
    let i = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.tabs[i].chat().bodies()
  }

  #[test]
  fn what_was_said_while_twitch_was_down_lands_in_order_and_nothing_doubles() {
    let (mut app, _asked) = app();
    for (id, ts) in [("a", 10), ("b", 20), ("e", 50)] {
      app.on_net(NetEvent::Chat("forsen".into(), crate::net::irc::Chat::Msg(line(id, ts, id))));
    }
    // c and d were said while the socket was down; b and e are lines we saw
    let gap = vec![line("b", 20, "b"), line("c", 30, "c"), line("d", 40, "d"), line("e", 50, "e")];
    app.on_net(NetEvent::CatchUp { platform: Platform::Twitch, key: "forsen".into(), msgs: gap.clone() });
    assert_eq!(shown(&mut app), ["a", "b", "c", "d", "e"]);
    app.on_net(NetEvent::CatchUp { platform: Platform::Twitch, key: "forsen".into(), msgs: gap });
    assert_eq!(shown(&mut app), ["a", "b", "c", "d", "e"], "a second catch-up adds nothing");
    // a feed with no tab is nobody's business
    app.on_net(NetEvent::CatchUp { platform: Platform::Twitch, key: "nobody".into(), msgs: vec![line("z", 99, "z")] });
    assert_eq!(shown(&mut app).len(), 5);
  }

  #[test]
  fn esc_gives_up_a_sign_in_that_is_waiting_and_only_then() {
    let (mut app, mut asked) = app();
    // the joins the app sent on start
    while asked.try_recv().is_ok() {}
    let esc = Key::Named(NamedKey::Escape);
    assert!(!app.login_esc(&esc), "nothing waiting: esc is esc");
    app.on_net(NetEvent::LoginWait(true));
    assert!(!app.login_esc(&Key::Character("x")));
    assert!(app.login_esc(&esc));
    assert!(matches!(asked.try_recv(), Ok(Cmd::CancelLogin)));
    assert!(!app.login_esc(&esc), "one cancel per sign-in");
    assert!(asked.try_recv().is_err());
    app.on_net(NetEvent::LoginWait(true));
    app.on_net(NetEvent::LoginWait(false));
    assert!(!app.login_esc(&esc), "it finished by itself");
  }
}
