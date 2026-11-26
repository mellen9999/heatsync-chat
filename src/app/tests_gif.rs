// the picture of a twitch gif-keyboard line (`chat:gif`): a line drawn from irc is its caption, and heatsync names
// the picture by message id. it reaches a line born after the event, and a row already on screen, never by words
use super::*;
use crate::model::chatgif::{self, Picture};
use crate::model::message::{Kind as MsgKind, Message};
use crate::net::irc::Chat;
use serde_json::json;

const CAPTION: &str = "[Mood Lol GIF by Giphy]";
const URL: &str = "https://media.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif";

fn app() -> App {
  let _ = std::fs::remove_dir_all(store::folder());
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen kick:xqc".into()], false);
  a.dpi = 1.0;
  a.res.set_scale(a.scale());
  a.resized(720, 400);
  a
}

fn said(p: Platform, id: &str, body: &str) -> Message {
  let mut m = Message::system(now_ms(), p, "x");
  (m.kind, m.id, m.login, m.nick, m.body) = (MsgKind::Chat, id.into(), "amy".into(), "amy".into(), body.into());
  m
}

fn tw(a: &mut App, id: &str, body: &str) {
  a.on_chat("forsen", Chat::Msg(said(Platform::Twitch, id, body)));
}

fn frame(id: &str, url: &str) -> NetEvent {
  let v = json!({"type": "chat:gif", "platform": "twitch", "channel": "forsen", "messageId": id,
    "gifs": [{"text": CAPTION, "url": url}]});
  NetEvent::Gif(Box::new(chatgif::frame(&v).expect("a good frame")))
}

// the picture each line in the forsen tab carries, in order
fn pics(a: &App) -> Vec<Option<String>> {
  let i = a.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  a.tabs[i].view.msgs().map(|m| m.x.as_ref().and_then(|x| x.gif.as_ref()).map(|g| g.url.to_string())).collect()
}

#[test]
fn a_line_born_after_its_event_carries_the_picture() {
  let mut a = app();
  a.on_net(frame("t1", URL));
  tw(&mut a, "t1", CAPTION);
  tw(&mut a, "t2", CAPTION);
  assert_eq!(pics(&a), [Some(URL.into()), None], "by id: the same caption on another line has none");
}

#[test]
fn a_row_already_on_screen_gains_it_by_id() {
  let mut a = app();
  tw(&mut a, "t1", CAPTION);
  tw(&mut a, "t2", CAPTION);
  assert_eq!(pics(&a), [None, None]);
  a.on_net(frame("t2", URL));
  assert_eq!(pics(&a), [None, Some(URL.into())], "the named line only; words never match");
}

#[test]
fn a_repeat_changes_nothing_and_a_new_url_replaces_the_picture() {
  let mut a = app();
  tw(&mut a, "t1", CAPTION);
  a.on_net(frame("t1", URL));
  let i = a.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  let pic = Picture { text: CAPTION.into(), url: URL.into() };
  assert!(!a.tabs[i].view.set_gif("t1", &pic), "what the row holds is not news");
  let other = "https://media0.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif";
  a.on_net(frame("t1", other));
  assert_eq!(pics(&a), [Some(other.into())]);
}

#[test]
fn another_platforms_line_with_the_same_id_is_left_alone() {
  let mut a = app();
  a.on_relay(Platform::Kick, "xqc", vec![said(Platform::Kick, "k1", CAPTION)], false);
  a.on_net(frame("k1", URL));
  let i = a.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  assert!(a.tabs[i].view.msgs().all(|m| m.x.is_none()), "a gif is a twitch line's");
  // and it is not held for a kick line that comes later
  a.on_relay(Platform::Kick, "xqc", vec![said(Platform::Kick, "k1", CAPTION)], false);
  assert!(a.tabs[i].view.msgs().all(|m| m.x.is_none()));
}

#[test]
fn a_history_line_the_event_named_is_born_with_it_too() {
  let mut a = app();
  a.on_net(frame("t1", URL));
  a.on_net(NetEvent::Backfill("forsen".into(), vec![Chat::Msg(said(Platform::Twitch, "t1", CAPTION))]));
  assert_eq!(pics(&a), [Some(URL.into())]);
}

#[test]
fn what_is_held_for_a_line_that_never_comes_is_bounded() {
  let mut a = app();
  for i in 0..400 {
    a.on_net(frame(&format!("t{i}"), URL));
  }
  tw(&mut a, "t0", CAPTION);
  tw(&mut a, "t399", CAPTION);
  assert_eq!(pics(&a), [None, Some(URL.into())], "the oldest were let go");
}
