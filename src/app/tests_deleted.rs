// a deleted line names its sender when the line is still in the tab; a line the tab never held says nothing
// (a notice about a line you never saw is noise). twitch and kick both
use super::*;
use crate::model::message::{Kind as MsgKind, Message};
use crate::net::irc::Chat;

fn app() -> App {
  let _ = std::fs::remove_dir_all(store::folder());
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen kick:xqc".into()], false);
  a.dpi = 1.0;
  a.res.set_scale(a.scale());
  a.resized(720, 400);
  a
}

fn said(p: Platform, id: &str, who: &str) -> Message {
  let mut m = Message::system(now_ms(), p, "x");
  (m.kind, m.id, m.login, m.nick, m.body) = (MsgKind::Chat, id.into(), who.into(), who.into(), "gg".into());
  m
}

fn notes(a: &App) -> Vec<String> {
  let i = a.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  a.tabs[i].view.msgs().filter(|m| m.kind == MsgKind::System).map(|m| m.body.to_string()).collect()
}

fn deleted(a: &App) -> Vec<String> {
  notes(a).into_iter().filter(|b| b.contains("deleted")).collect()
}

#[test]
fn a_twitch_delete_of_a_held_line_names_the_sender() {
  let mut a = app();
  a.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "t1", "amy")));
  a.on_chat("forsen", Chat::ClearMsg("t1".into(), "amy".into()));
  assert_eq!(deleted(&a), ["message from amy deleted"]);
}

#[test]
fn a_twitch_delete_of_a_line_never_held_says_nothing() {
  let mut a = app();
  a.on_chat("forsen", Chat::ClearMsg("gone".into(), "amy".into()));
  assert!(deleted(&a).is_empty(), "{:?}", notes(&a));
}

#[test]
fn a_kick_delete_names_the_sender_from_the_held_line() {
  let mut a = app();
  a.on_relay(Platform::Kick, "xqc", vec![said(Platform::Kick, "k1", "bob")], false);
  a.on_net(NetEvent::RelayDelete { platform: Platform::Kick, key: "xqc".into(), id: "k1".into() });
  assert_eq!(deleted(&a), ["message from bob deleted"], "the event carries only the id");
}

#[test]
fn a_kick_delete_of_a_line_never_held_says_nothing() {
  let mut a = app();
  a.on_net(NetEvent::RelayDelete { platform: Platform::Kick, key: "xqc".into(), id: "nope".into() });
  assert!(deleted(&a).is_empty(), "{:?}", notes(&a));
}
