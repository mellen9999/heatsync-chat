// the user card's clip row, headless: when it shows, what a press asks for, what each answer does

use super::*;
use crate::model::card::Info;
use crate::model::message::{Kind as MsgKind, Message};
use crate::net::Cmd;
use crate::net::auth::Me;
use crate::net::irc::Chat;
use crate::net::twclip::{Fail, Made};
use winit::keyboard::Key;

fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen kick:xqc".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(520, 300);
  app.on_net(NetEvent::Me(Some(Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  })));
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

// a card for `login`'s line, its facts delivered with `live` as given
fn card_of(app: &mut App, p: Platform, login: &str, live: bool) {
  match p {
    Platform::Twitch => app.on_chat("forsen", Chat::Msg(line(p, login, "m1"))),
    _ => {
      app.on_net(NetEvent::Relay { platform: p, key: "xqc".into(), msgs: vec![line(p, login, "m1")], history: false })
    }
  }
  app.redraw();
  let h = app.chat_area().h;
  let res = &mut app.res;
  assert!(app.tabs[app.active].view.select_id("m1", res, h));
  app.card_selected();
  let info = Info { registered: true, live, ..Info::default() };
  app.on_net(NetEvent::Card { platform: p, login: login.into(), res: Ok(info) });
}

fn rows(app: &App) -> Vec<String> {
  app.card().map(|c| c.menu.items.iter().map(|r| r.1.clone()).collect()).unwrap_or_default()
}

fn said(app: &App) -> String {
  app.tabs[app.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
}

fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Cmd> {
  std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

fn made(clip: Option<&str>) -> Made {
  Made { clip_url: clip.map(Into::into), edit_url: None }
}

#[test]
fn the_row_shows_only_for_a_live_twitch_person() {
  let (mut app, _rx) = boot();
  card_of(&mut app, Platform::Twitch, "bob", true);
  assert_eq!(rows(&app).last().map(String::as_str), Some("clip"));
  app.close_card();
  let (mut app, _rx) = boot();
  card_of(&mut app, Platform::Twitch, "bob", false);
  assert!(!rows(&app).iter().any(|r| r.starts_with("clip")), "{:?}", rows(&app));
  // kick has no twitch clip, live or not
  app.close_card();
  let (mut app, _rx) = boot();
  card_of(&mut app, Platform::Kick, "bob", true);
  assert!(!rows(&app).iter().any(|r| r.starts_with("clip")), "{:?}", rows(&app));
}

#[test]
fn c_clips_the_room_once_and_the_answer_copies_the_link() {
  let (mut app, mut rx) = boot();
  card_of(&mut app, Platform::Twitch, "bob", true);
  sent(&mut rx);
  app.card_key(Key::Character("c"));
  assert!(matches!(sent(&mut rx).as_slice(), [Cmd::Clip(l, c)] if &**l == "bob" && &**c == "forsen"));
  assert_eq!(rows(&app).last().map(String::as_str), Some("clipping\u{2026}"));
  assert!(app.card().is_some(), "the card stays up");
  // pressed again while it works: nothing more is asked
  app.card_key(Key::Character("c"));
  assert!(sent(&mut rx).is_empty());
  app.on_net(NetEvent::Clip { login: "bob".into(), res: Ok(made(Some("https://clips.twitch.tv/x"))) });
  assert!(matches!(sent(&mut rx).as_slice(), [Cmd::Copy { text, done }]
    if text == "https://clips.twitch.tv/x" && done == "url copied"));
  assert_eq!(rows(&app).last().map(String::as_str), Some("url copied"));
  // a press now (no edit page known) copies the link again, it does not clip twice
  app.card_key(Key::Character("c"));
  assert!(matches!(sent(&mut rx).as_slice(), [Cmd::Copy { .. }]));
}

#[test]
fn enter_on_the_row_clips_too() {
  let (mut app, mut rx) = boot();
  card_of(&mut app, Platform::Twitch, "bob", true);
  sent(&mut rx);
  let last = rows(&app).len() - 1;
  app.card_mut().unwrap().menu.at = last;
  app.card_key(Key::Named(winit::keyboard::NamedKey::Enter));
  assert!(matches!(sent(&mut rx).as_slice(), [Cmd::Clip(..)]));
  assert!(app.card().is_some());
}

#[test]
fn no_link_says_created_a_failure_says_why_and_the_row_resets() {
  let (mut app, mut rx) = boot();
  card_of(&mut app, Platform::Twitch, "bob", true);
  app.card_key(Key::Character("c"));
  sent(&mut rx);
  app.on_net(NetEvent::Clip { login: "bob".into(), res: Ok(made(None)) });
  assert_eq!(said(&app), "clip created");
  assert_eq!(rows(&app).last().map(String::as_str), Some("clip created"));
  assert!(sent(&mut rx).is_empty(), "nothing to copy");
  // a fresh card, a failed clip: heatsync's own words, and the row is a clip button again
  app.close_card();
  app.card_selected();
  app.card_key(Key::Character("c"));
  sent(&mut rx);
  app.on_net(NetEvent::Clip { login: "bob".into(), res: Err(Fail::Other("Channel not found on Twitch".into())) });
  assert_eq!(said(&app), "clip failed: Channel not found on Twitch");
  assert_eq!(rows(&app).last().map(String::as_str), Some("clip"));
}

#[test]
fn a_missing_permission_starts_the_clips_relink() {
  let (mut app, mut rx) = boot();
  card_of(&mut app, Platform::Twitch, "bob", true);
  app.card_key(Key::Character("c"));
  sent(&mut rx);
  app.on_net(NetEvent::Clip { login: "bob".into(), res: Err(Fail::Relink) });
  assert!(sent(&mut rx).iter().any(|c| matches!(c, Cmd::Relink("clips"))));
  assert_eq!(rows(&app).last().map(String::as_str), Some("clip"));
}

#[test]
fn logged_out_it_says_so_and_a_closed_card_still_gets_its_link_copied() {
  let (mut app, mut rx) = boot();
  card_of(&mut app, Platform::Twitch, "bob", true);
  sent(&mut rx);
  app.on_net(NetEvent::Me(None));
  app.card_key(Key::Character("c"));
  assert_eq!(said(&app), "log in first (ctrl-l)");
  assert!(!sent(&mut rx).iter().any(|c| matches!(c, Cmd::Clip(..))));
  // signed in again, clip, close the card, then the answer lands
  let (mut app, mut rx) = boot();
  card_of(&mut app, Platform::Twitch, "bob", true);
  app.card_key(Key::Character("c"));
  sent(&mut rx);
  app.close_card();
  app.on_net(NetEvent::Clip { login: "bob".into(), res: Ok(made(Some("https://clips.twitch.tv/y"))) });
  assert!(matches!(sent(&mut rx).as_slice(), [Cmd::Copy { text, .. }] if text == "https://clips.twitch.tv/y"));
}
