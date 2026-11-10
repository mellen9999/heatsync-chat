// a person's chat logs kept live, headless: their archive-line room is joined while the logs are open and let
// go on close, their new lines land at the bottom (once), and another person's or a hostile frame is nothing

use super::*;
use crate::model::message::{Kind as MsgKind, Message};
use crate::net::Cmd;
use crate::net::hs_ws::Cmd as HsCmd;
use serde_json::json;

fn rooms(asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(String, bool)> {
  std::iter::from_fn(|| asked.try_recv().ok())
    .filter_map(|c| if let Cmd::Hs(HsCmd::Feed(k, on)) = c { Some((k.to_string(), on)) } else { None })
    .collect()
}

fn frame(app: &mut App, v: serde_json::Value) {
  let (tx, rx) = std::sync::mpsc::channel();
  crate::net::hs_ws::feed_frame(&v, &crate::net::Out::Pipe(tx));
  for ev in rx.try_iter() {
    app.on_net(ev);
  }
}

fn row(user: &str, id: &str, text: &str) -> serde_json::Value {
  json!({"type": "user-log", "data": {"platform": "twitch", "channel": "forsen", "username": user,
    "display_name": user, "message": text, "message_id": id, "timestamp": "2026-11-10T12:00:00.000Z"}})
}

#[test]
fn a_persons_logs_take_their_new_lines_live() {
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let mut m = Message::system(crate::util::time::now_ms(), Platform::Twitch, "hello");
  m.kind = MsgKind::Chat;
  (m.id, m.login, m.nick) = ("1".into(), "Bob".into(), "Bob".into());
  app.tabs[chat].chat().push(m.clone());
  let who = app.subject_of(&m).unwrap();
  while asked.try_recv().is_ok() {}
  app.logs_open(&who);
  assert_eq!(rooms(&mut asked), [("ulog:twitch:bob".to_string(), true)]);
  let count = |app: &App| app.tabs[chat].view.msgs().filter(|m| &*m.body == "hi there").count();
  frame(&mut app, row("bob", "a1", "hi there"));
  assert_eq!(count(&app), 1, "at the bottom, any case of the name");
  frame(&mut app, row("bob", "a1", "hi there"));
  assert_eq!(count(&app), 1, "the same line twice is one");
  frame(&mut app, row("carol", "a2", "hi there"));
  let mut other = row("bob", "a3", "hi there");
  other["data"]["platform"] = json!("kick");
  frame(&mut app, other);
  frame(&mut app, json!({"type": "user-log", "data": {"platform": "twitch", "username": "bob"}}));
  frame(&mut app, json!({"type": "user-log", "data": "bob"}));
  assert_eq!(count(&app), 1, "someone else, another platform, and a line with no words are nothing");
  // the logs closed: the room is let go and a later line goes nowhere
  app.logs_close();
  assert_eq!(rooms(&mut asked), [("ulog:twitch:bob".to_string(), false)]);
  frame(&mut app, row("bob", "a4", "hi there"));
  assert_eq!(count(&app), 0);
}

// `user-log:removed` drops what it names from the open logs: ids, a channel, an exclusive window, or all of them
#[test]
fn removed_lines_leave_an_open_log() {
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let mut m = Message::system(crate::util::time::now_ms(), Platform::Twitch, "hello");
  m.kind = MsgKind::Chat;
  (m.id, m.login, m.nick) = ("1".into(), "bob".into(), "bob".into());
  let mut who = app.subject_of(&m).unwrap();
  // no home room: every channel shows
  who.chan = "".into();
  app.logs_open(&who);
  while asked.try_recv().is_ok() {}
  let at = |n: u32, ch: &str, id: &str| {
    let mut r = row("bob", id, &format!("line {id}"));
    r["data"]["timestamp"] = json!(format!("2026-11-10T12:00:0{n}.000Z"));
    r["data"]["channel"] = json!(ch);
    r
  };
  for (n, ch, id) in [(1, "forsen", "a"), (2, "forsen", "b"), (3, "xqc", "c"), (4, "forsen", "d")] {
    frame(&mut app, at(n, ch, id));
  }
  let shown = |app: &App| {
    let mut v: Vec<String> =
      app.tabs[chat].view.msgs().filter(|m| m.body.starts_with("line ")).map(|m| m.body.to_string()).collect();
    v.sort();
    v.join(",")
  };
  assert_eq!(shown(&app), "line a,line b,line c,line d");
  let gone = |d: serde_json::Value| json!({"type": "user-log:removed", "data": d});
  frame(&mut app, gone(json!({"platform": "twitch", "username": "bob", "ids": ["a"]})));
  assert_eq!(shown(&app), "line b,line c,line d", "by id");
  frame(&mut app, gone(json!({"platform": "twitch", "username": "carol", "ids": ["b"]})));
  frame(&mut app, gone(json!({"platform": "kick", "username": "bob", "ids": ["b"]})));
  frame(&mut app, gone(json!({"platform": "twitch", "username": "bob", "after": "garbage"})));
  assert_eq!(shown(&app), "line b,line c,line d", "another person, another platform, a bound that cannot be read");
  frame(&mut app, gone(json!({"platform": "twitch", "username": "bob", "channel": "xqc"})));
  assert_eq!(shown(&app), "line b,line d", "by channel");
  // a window, both bounds exclusive: 12:00:02 is the edge of it and stays
  frame(
    &mut app,
    gone(
      json!({"platform": "twitch", "username": "bob", "after": "2026-11-10T12:00:02.000Z", "before": "2026-11-10T12:00:09.000Z"}),
    ),
  );
  assert_eq!(shown(&app), "line b", "by window");
  frame(&mut app, gone(json!({"platform": "twitch", "username": "bob"})));
  assert_eq!(shown(&app), "", "nothing narrowing it: all of them");
}
