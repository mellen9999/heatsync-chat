// pin / unpin for everyone, headless: who may, what a line's menu and the bar's × do, and the words
// each answer says. the ws `pin:set|clear` echo (the read side) is covered in model/pin.rs and net/pin.rs

use super::*;
use crate::model::message::{Kind as MsgKind, Message};
use crate::model::pin::Pin;
use crate::net::Cmd;
use crate::net::auth::Me;
use crate::net::irc::Chat;
use crate::net::pinact::{Done, Fail, Op};
use serde_json::json;
use winit::keyboard::Key;

fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen kick:xqc yt:@abc".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(520, 300);
  app.on_net(NetEvent::Me(Some(Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: Some("mellen_k".into()),
    youtube: Some("mellenyt".into()),
    packs: vec![],
  })));
  app.redraw();
  while rx.try_recv().is_ok() {}
  (app, rx)
}

fn line(p: Platform, login: &str, id: &str) -> Message {
  let mut m = Message::system(now_ms(), p, "hi");
  m.kind = MsgKind::Chat;
  m.login = login.into();
  m.nick = login.into();
  m.id = id.into();
  m
}

fn pin_of(msg_id: &str) -> Pin {
  crate::model::pin::parse(
    &json!({"id": format!("p-{msg_id}"), "message_id": msg_id, "content": "hi", "username": "bob"}),
  )
  .unwrap()
}

fn said(app: &App) -> String {
  app.tabs[app.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
}

fn pins(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Op> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Pin(o) => Some(o),
      _ => None,
    })
    .collect()
}

fn pick(app: &mut App, id: &str) {
  let h = app.chat_area().h;
  let res = &mut app.res;
  assert!(app.tabs[app.active].view.select_id(id, res, h), "{id} is on screen");
}

#[test]
fn only_the_owner_or_a_twitch_mod_may_pin_and_kick_mods_may_not() {
  let (mut app, _rx) = boot();
  assert!(!app.can_pin(Platform::Twitch, "forsen"));
  assert!(!app.can_pin(Platform::Kick, "xqc"));
  // a twitch mod of the room
  app.on_chat("forsen", Chat::Mod(true));
  assert!(app.can_pin(Platform::Twitch, "forsen"));
  // our own channels: kick folds `_` into `-`
  assert!(app.can_pin(Platform::Twitch, "mellen"));
  assert!(app.can_pin(Platform::Kick, "mellen-k"));
  // a kick moderator badge on our line is not enough: the server refuses kick mods
  let mut mine = line(Platform::Kick, "mellen-k", "k1");
  mine.badges = vec![("moderator".into(), "1".into())].into();
  app.on_net(NetEvent::Relay { platform: Platform::Kick, key: "xqc".into(), msgs: vec![mine], history: false });
  assert!(app.can_mod(Platform::Kick, "xqc"));
  assert!(!app.can_pin(Platform::Kick, "xqc"));
  assert!(!app.can_pin(Platform::Youtube, "@abc"));
  app.on_net(NetEvent::Me(None));
  assert!(!app.can_pin(Platform::Twitch, "forsen"));
}

#[test]
fn a_mods_line_menu_pins_and_a_pinned_line_offers_unpin() {
  let (mut app, mut rx) = boot();
  app.on_chat("forsen", Chat::Mod(true));
  app.on_chat("forsen", Chat::Msg(line(Platform::Twitch, "bob", "t1")));
  app.on_chat("forsen", Chat::Msg(line(Platform::Twitch, "amy", "t2")));
  app.redraw();
  pick(&mut app, "t1");
  app.line_menu_key();
  let rows: Vec<String> = app.pop().unwrap().menu.items.iter().map(|r| r.1.clone()).collect();
  assert!(rows.iter().any(|r| r == "pin message") && !rows.iter().any(|r| r == "unpin"), "{rows:?}");
  app.pin_selected(true);
  match pins(&mut rx).as_slice() {
    [Op::Set { platform: Platform::Twitch, chan, msg_id }] => assert_eq!((&**chan, &**msg_id), ("forsen", "t1")),
    _ => panic!("one pin request"),
  }
  // the room now holds t1's pin: that line offers unpin, the other still offers pin
  app.on_net(NetEvent::Pin { platform: Platform::Twitch, key: "forsen".into(), pin: Some(pin_of("t1")) });
  app.close_pop();
  app.line_menu_key();
  let rows: Vec<String> = app.pop().unwrap().menu.items.iter().map(|r| r.1.clone()).collect();
  assert!(rows.iter().any(|r| r == "unpin") && !rows.iter().any(|r| r == "pin message"), "{rows:?}");
  app.pin_selected(false);
  assert!(matches!(pins(&mut rx).as_slice(), [Op::Clear { platform: Platform::Twitch, chan }] if &**chan == "forsen"));
  app.close_pop();
  pick(&mut app, "t2");
  app.line_menu_key();
  let rows: Vec<String> = app.pop().unwrap().menu.items.iter().map(|r| r.1.clone()).collect();
  assert!(rows.iter().any(|r| r == "pin message"), "{rows:?}");
}

#[test]
fn no_pin_row_for_a_non_mod_a_kick_mod_a_youtube_line_or_a_line_without_an_id() {
  let (mut app, _rx) = boot();
  let t = line(Platform::Twitch, "bob", "t1");
  assert_eq!(app.pin_menu(&t), None, "not a mod");
  app.on_chat("forsen", Chat::Mod(true));
  assert_eq!(app.pin_menu(&t), Some(false));
  assert_eq!(app.pin_menu(&line(Platform::Twitch, "bob", "")), None, "no id to pin");
  assert_eq!(app.pin_menu(&line(Platform::Youtube, "bob", "y1")), None);
  // kick: moderator is not enough
  let mut mine = line(Platform::Kick, "mellen-k", "k1");
  mine.badges = vec![("broadcaster".into(), "1".into())].into();
  app.on_net(NetEvent::Relay { platform: Platform::Kick, key: "xqc".into(), msgs: vec![mine], history: false });
  assert_eq!(app.pin_menu(&line(Platform::Kick, "bob", "k2")), None);
}

#[test]
fn the_bar_x_asks_a_pinner_and_just_hides_for_everyone_else() {
  let (mut app, mut rx) = boot();
  app.on_net(NetEvent::Pin { platform: Platform::Twitch, key: "forsen".into(), pin: Some(pin_of("t1")) });
  // not a mod: a local hide, nothing asked, nothing sent
  app.pin_x();
  assert!(app.pop().is_none() && pins(&mut rx).is_empty());
  assert!(app.pin_bar.pins.current(&app.tabs[app.active].sources).is_none(), "hidden here");
  // a mod: asked first, esc cancels without a request
  app.on_chat("forsen", Chat::Mod(true));
  app.on_net(NetEvent::Pin { platform: Platform::Twitch, key: "forsen".into(), pin: Some(pin_of("t2")) });
  app.pin_x();
  let rows: Vec<String> = app.pop().unwrap().menu.items.iter().map(|r| r.1.clone()).collect();
  assert_eq!(rows, ["yes, unpin", "cancel"]);
  app.pop_key(Key::Named(winit::keyboard::NamedKey::Escape));
  assert!(app.pop().is_none() && pins(&mut rx).is_empty());
  assert!(app.pin_bar.pins.current(&app.tabs[app.active].sources).is_some(), "still pinned");
  // y says yes
  app.pin_x();
  app.pop_key(Key::Character("y"));
  assert!(matches!(pins(&mut rx).as_slice(), [Op::Clear { platform: Platform::Twitch, chan }] if &**chan == "forsen"));
  // enter on the first row says yes too
  app.pin_x();
  app.pop_key(Key::Named(winit::keyboard::NamedKey::Enter));
  assert_eq!(pins(&mut rx).len(), 1);
}

#[test]
fn every_answer_says_the_exts_words() {
  let (mut app, mut rx) = boot();
  let done = |set, res| Done { platform: Platform::Twitch, chan: "forsen".into(), set, res };
  app.on_pin_done(done(true, Ok(Some(pin_of("t1")))));
  assert_eq!(said(&app), "pinned");
  assert!(app.pin_bar.pins.current(&app.tabs[app.active].sources).is_some(), "the answer repaints the bar");
  app.on_pin_done(done(false, Ok(None)));
  assert_eq!(said(&app), "unpinned");
  assert!(app.pin_bar.pins.current(&app.tabs[app.active].sources).is_none());
  for (f, want) in [
    (Fail::Auth, "sign in to heatsync again"),
    (Fail::Denied, "only mods can pin here"),
    (Fail::NotArchived, "not in the archive yet \u{2014} try again in a moment"),
    (Fail::Other, "couldn't pin"),
  ] {
    app.on_pin_done(done(true, Err(f)));
    assert_eq!(said(&app), want);
  }
  // a missing permission says so and starts the relink
  app.on_pin_done(done(true, Err(Fail::Relink)));
  assert_eq!(said(&app), "link heatsync to set this");
  assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, Cmd::Relink("mod"))));
}
