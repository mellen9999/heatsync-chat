// a run of timeouts folds into one line, headless: the count, what ends a run, the setting, enter and click
// opening it, the dimming of the lines it came from, bounded memory, and the picture of it
use super::*;
use crate::model::message::flag;
use crate::model::message::{Kind as MsgKind, Message};
use crate::net::irc::Chat;
use winit::keyboard::{Key, NamedKey};

fn app() -> App {
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen".into()], false);
  a.dpi = 1.0;
  a.res.set_scale(a.scale());
  a.resized(720, 400);
  let i = a.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  a.select(i);
  a
}

fn out(a: &mut App, who: &str) {
  a.on_chat("forsen", Chat::ClearUser { login: who.into(), secs: Some(60) });
}

fn ban(a: &mut App, who: &str) {
  a.on_chat("forsen", Chat::ClearUser { login: who.into(), secs: None });
}

fn say(a: &mut App, who: &str, body: &str) {
  let mut m = Message::system(now_ms(), Platform::Twitch, "x");
  (m.kind, m.login, m.nick, m.body, m.id) = (MsgKind::Chat, who.into(), who.into(), body.into(), body.into());
  a.on_chat("forsen", Chat::Msg(m));
}

fn bodies(a: &App) -> Vec<String> {
  a.tabs[a.active].view.bodies()
}

#[test]
fn three_timeouts_in_a_row_are_one_line() {
  let mut a = app();
  out(&mut a, "bob");
  assert_eq!(bodies(&a), ["bob timed out for 60s"], "one alone reads as itself");
  out(&mut a, "cat");
  out(&mut a, "dan");
  assert_eq!(bodies(&a), ["▸ 3 people timed out"]);
}

#[test]
fn a_chat_line_ends_the_run_and_is_never_hidden() {
  let mut a = app();
  out(&mut a, "bob");
  out(&mut a, "cat");
  say(&mut a, "ann", "still here");
  out(&mut a, "dan");
  out(&mut a, "eve");
  assert_eq!(bodies(&a), ["▸ 2 people timed out", "still here", "▸ 2 people timed out"]);
}

#[test]
fn bans_say_so() {
  let mut a = app();
  ban(&mut a, "bob");
  ban(&mut a, "cat");
  assert_eq!(bodies(&a), ["▸ 2 people banned"]);
  out(&mut a, "dan");
  assert_eq!(bodies(&a), ["▸ 3 people timed out or banned"]);
}

#[test]
fn the_setting_off_keeps_every_line() {
  let mut a = app();
  a.set.collapse_timeouts = false;
  out(&mut a, "bob");
  out(&mut a, "cat");
  assert_eq!(bodies(&a).len(), 2);
}

#[test]
fn timed_out_lines_still_dim_inside_a_run() {
  let mut a = app();
  say(&mut a, "bob", "hello");
  say(&mut a, "cat", "hi");
  out(&mut a, "bob");
  out(&mut a, "cat");
  let v = &a.tabs[a.active].view;
  assert!(v.msgs().filter(|m| m.kind.said()).all(|m| m.has(flag::CLEARED)), "both lines went dim");
  assert_eq!(v.bodies().last().unwrap(), "▸ 2 people timed out");
}

#[test]
fn enter_opens_the_picked_run_and_enter_closes_it() {
  let mut a = app();
  out(&mut a, "bob");
  out(&mut a, "cat");
  a.redraw();
  a.mode = Mode::Normal;
  a.key_normal(Key::Character("v"));
  a.key_normal(Key::Character("k"));
  assert!(a.selecting(), "the run can be picked");
  assert!(a.pick_key(Key::Named(NamedKey::Enter)));
  assert_eq!(bodies(&a), ["▾ 2 people timed out\n  bob timed out for 60s\n  cat timed out for 60s"]);
  assert!(a.pick_key(Key::Named(NamedKey::Enter)));
  assert_eq!(bodies(&a), ["▸ 2 people timed out"]);
}

#[test]
fn a_run_cannot_be_replied_to_or_copied_as_a_line() {
  let mut a = app();
  out(&mut a, "bob");
  out(&mut a, "cat");
  a.redraw();
  a.mode = Mode::Normal;
  a.key_normal(Key::Character("v"));
  a.key_normal(Key::Character("k"));
  assert!(a.tabs[a.active].view.selected().is_none(), "no line to act on");
}

#[test]
fn a_click_opens_it_and_the_picture_changes() {
  let mut a = app();
  out(&mut a, "bob");
  out(&mut a, "cat");
  a.redraw();
  let area = a.chat_area();
  let folded = a.canvas.px.clone();
  a.pointer = (area.x + 100, area.bottom() - 6);
  a.click();
  a.redraw();
  assert!(bodies(&a)[0].starts_with("▾ 2 people timed out"));
  assert_ne!(folded, a.canvas.px, "three lines are drawn where one was");
  a.click();
  a.redraw();
  assert_eq!(bodies(&a), ["▸ 2 people timed out"]);
}

#[test]
fn a_storm_stays_one_bounded_row() {
  let mut a = app();
  for i in 0..3000 {
    out(&mut a, &format!("user{i}"));
  }
  let v = &a.tabs[a.active].view;
  assert_eq!(v.len(), 1);
  assert_eq!(v.bodies(), ["▸ 3000 people timed out"]);
  let run = v.msgs().next().unwrap().x.as_ref().unwrap().run.as_ref().unwrap();
  assert!(run.text().lines().count() <= crate::model::runs::MAX_LINES + 3);
}

#[test]
fn a_kick_removal_folds_the_same_way() {
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["kick:xqc".into()], false);
  a.resized(720, 400);
  for who in ["a", "b"] {
    a.on_net(NetEvent::RelayClear { platform: Platform::Kick, key: "xqc".into(), login: who.into(), secs: Some(30) });
  }
  let i = a.tabs.iter().position(|t| &*t.name == "kick:xqc").unwrap();
  assert_eq!(a.tabs[i].view.bodies(), ["▸ 2 people timed out"]);
}
