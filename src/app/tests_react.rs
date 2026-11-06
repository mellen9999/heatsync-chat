// reaction chips under feed posts, headless: drawn only where a post has them, a click flips your reaction at
// once and a refusal puts it back, live frames from other people land (yours do not), the thread's room is
// joined while it is open. HSC_DUMP=$d/react.ppm cargo test --release headless_react_chips

use super::*;
use crate::images::decode::{Decoded, Frame};
use crate::model::post::parse;
use crate::model::tabs::Kind;
use crate::net::{Cmd, social};
use serde_json::json;

const KEK: &str = "https://cdn.7tv.app/emote/kek/1x.webp";

type Asked = tokio::sync::mpsc::UnboundedReceiver<Cmd>;

fn post(id: &str, reactions: serde_json::Value) -> crate::model::post::Post {
  parse(&json!({"base36_id": id, "username": "bob", "user_id": "9", "content": "hello feed", "reactions": reactions}))
    .unwrap()
}

fn boot(posts: Vec<crate::model::post::Post>) -> (App, Asked, usize) {
  let (net, asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), false);
  app.set = Settings::default();
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  app.feed.posts = posts;
  app.feed.loaded = true;
  let feed = app.tabs.iter().position(|t| t.kind == Kind::Feed).expect("a feed tab");
  app.select(feed);
  app.feed_show();
  app.redraw();
  (app, asked, feed)
}

fn log_in(app: &mut App) {
  app.me = Some(crate::net::auth::Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: None,
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  });
}

fn sent(asked: &mut Asked) -> Vec<Cmd> {
  std::iter::from_fn(|| asked.try_recv().ok()).collect()
}

fn reacts(cmds: &[Cmd]) -> Vec<(String, u64, bool)> {
  cmds
    .iter()
    .filter_map(|c| match c {
      Cmd::Social(social::Cmd::React { id, emote, on }) => Some((id.to_string(), *emote, *on)),
      _ => None,
    })
    .collect()
}

// every picture asked for lands as a flat 16x16 block
fn land(app: &mut App, asked: &mut Asked) -> Vec<Cmd> {
  let mut other = Vec::new();
  for _ in 0..3 {
    app.redraw();
    let mut wanted = Vec::new();
    for c in sent(asked) {
      match c {
        Cmd::Image(id, _, target) => wanted.push((id, target)),
        c => other.push(c),
      }
    }
    if wanted.is_empty() {
      break;
    }
    for (id, target) in wanted {
      let d = Decoded { w: 16, h: 16, frames: vec![Frame { rgba: [0, 0, 255, 255].repeat(256).into(), delay_ms: 0 }] };
      app.res.images.loaded(id, target, Ok(d));
    }
    for t in &mut app.tabs {
      t.view.relayout();
    }
  }
  other
}

// where a chip of this emote can be clicked, found the way the pointer finds it
fn chip(app: &mut App, tab: usize, emote: u64) -> Option<(i32, i32)> {
  let area = app.chat_area();
  (area.y..area.bottom()).step_by(3).find_map(|y| {
    (area.x..area.right()).step_by(5).find_map(|x| {
      let res = &mut app.res;
      let hit = app.tabs[tab].view.chip_at(area, x, y, res);
      hit.filter(|(_, e)| *e == emote).map(|_| (x, y))
    })
  })
}

fn chips_of(app: &App, id: &str) -> Vec<(u64, u32, bool, bool)> {
  app.feed.find(id).unwrap().reactions.iter().map(|r| (r.id, r.count, r.mine, r.url.is_some())).collect()
}

fn dump(app: &App) {
  if let Some(path) = std::env::var_os("HSC_DUMP") {
    let c = &app.canvas;
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(path, out);
  }
}

fn last_say(app: &App, tab: usize) -> String {
  app.tabs[tab].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
}

#[test]
fn headless_react_chips() {
  let reactions = json!([
    {"emote_id": 1, "emote_name": "kek", "emote_url": KEK, "count": 2, "reacted": false},
    {"emote_id": 2, "emote_name": "gated", "emote_url": KEK, "count": 1, "reacted": true, "nsfw": true},
    {"emote_id": 3, "emote_name": "offsite", "emote_url": "https://evil.example/x.png", "count": 4, "reacted": false},
  ]);
  let (mut app, mut asked, feed) = boot(vec![post("00004a", reactions), post("00004b", json!([]))]);
  assert_eq!(chips_of(&app, "00004a"), [(1, 2, false, true), (2, 1, true, false), (3, 4, false, false)]);
  assert!(chips_of(&app, "00004b").is_empty());
  let rows = app.feed.rows(None, false);
  assert_eq!(
    rows.iter().filter(|m| m.x.as_ref().is_some_and(|x| !x.react.is_empty())).count(),
    1,
    "only the post with chips"
  );
  land(&mut app, &mut asked);
  dump(&app);
  // the picture is the chip's own, loaded through the image loader: a chip that is a name has none
  let area = app.chat_area();
  let hits: Vec<_> = (area.y..area.bottom())
    .step_by(3)
    .flat_map(|y| (area.x..area.right()).step_by(5).map(move |x| (x, y)))
    .filter_map(|(x, y)| {
      let res = &mut app.res;
      app.tabs[feed].view.chip_at(area, x, y, res)
    })
    .collect();
  assert!(!hits.is_empty() && hits.iter().all(|(id, _)| &**id == "00004a"), "a post without chips has none to hit");

  // logged out: the click says so and asks nothing
  let at = chip(&mut app, feed, 1).expect("the kek chip is on screen");
  app.pointer = at;
  app.click();
  assert_eq!(last_say(&app, feed), "log in at heatsync.org first");
  assert!(reacts(&sent(&mut asked)).is_empty());
  assert!(app.tabs[feed].view.selected().is_none(), "the chip took the click: no row picked");

  // logged in: shows at once, one request at a time, a refusal puts it back
  log_in(&mut app);
  app.feed_show();
  app.redraw();
  let at = chip(&mut app, feed, 1).expect("still there");
  app.pointer = at;
  app.click();
  let cmds = sent(&mut asked);
  assert_eq!(reacts(&cmds), [("00004a".to_owned(), 1, true)]);
  assert!(
    !cmds.iter().any(|c| matches!(c, Cmd::Social(social::Cmd::Vote { .. } | social::Cmd::Thread { .. }))),
    "no other row click fired"
  );
  assert!(app.feed.thread.is_none() && app.tabs[feed].view.selected().is_none());
  assert_eq!(chips_of(&app, "00004a")[0], (1, 3, true, true));
  app.redraw();
  let at = chip(&mut app, feed, 1).unwrap();
  app.pointer = at;
  app.click();
  assert!(reacts(&sent(&mut asked)).is_empty(), "one request per chip at a time");
  app.on_net(NetEvent::Reacted { id: "00004a".into(), emote: 1, on: true, res: Err(500) });
  assert_eq!(chips_of(&app, "00004a")[0], (1, 2, false, true), "rolled back");
  assert_eq!(last_say(&app, feed), "that didn’t work — try again");
  app.on_net(NetEvent::Reacted { id: "00004a".into(), emote: 1, on: true, res: Err(401) });
  assert_eq!(chips_of(&app, "00004a")[0], (1, 2, false, true), "an answer nobody waits for changes nothing");
  // press, accepted: it stays; press again: it comes off, a refusal keeps it
  app.react_toggle("00004a".into(), 1);
  app.on_net(NetEvent::Reacted { id: "00004a".into(), emote: 1, on: true, res: Ok(true) });
  assert_eq!(chips_of(&app, "00004a")[0], (1, 3, true, true));
  assert_eq!(reacts(&sent(&mut asked)), [("00004a".to_owned(), 1, true)]);
  app.react_toggle("00004a".into(), 1);
  assert_eq!(reacts(&sent(&mut asked)), [("00004a".to_owned(), 1, false)], "DELETE this time");
  assert_eq!(chips_of(&app, "00004a")[0], (1, 2, false, true));
  app.on_net(NetEvent::Reacted { id: "00004a".into(), emote: 1, on: false, res: Err(500) });
  assert_eq!(chips_of(&app, "00004a")[0], (1, 3, true, true), "a refused removal keeps it");
  // the only reactor leaving drops the chip; a refusal brings it back
  app.react_toggle("00004a".into(), 2);
  assert_eq!(chips_of(&app, "00004a").iter().map(|c| c.0).collect::<Vec<_>>(), [1, 3], "dropped at 0");
  app.on_net(NetEvent::Reacted { id: "00004a".into(), emote: 2, on: false, res: Err(500) });
  assert!(chips_of(&app, "00004a").contains(&(2, 1, true, false)), "back, still a name");
  // taking back what the server never had is as good as done (the send side maps 404 to ok)
  // the server says you had it already (a thread's list does not know): the count held you
  app.react_toggle("00004a".into(), 3);
  assert!(chips_of(&app, "00004a").contains(&(3, 5, true, false)));
  app.on_net(NetEvent::Reacted { id: "00004a".into(), emote: 3, on: true, res: Ok(false) });
  assert!(chips_of(&app, "00004a").contains(&(3, 4, true, false)));
  // a chip that is not there, or a post that is gone: nothing
  while asked.try_recv().is_ok() {}
  app.react_toggle("00004a".into(), 99);
  app.react_toggle("zzzz".into(), 1);
  assert!(reacts(&sent(&mut asked)).is_empty());
}

#[test]
fn headless_react_frames() {
  let (mut app, mut asked, _) = boot(vec![post(
    "00004a",
    json!([{"emote_id": 1, "emote_name": "kek", "emote_url": KEK, "count": 2, "reacted": true}]),
  )]);
  log_in(&mut app);
  app.on_net(NetEvent::HsUser("9".into()));
  let frame = |ty: &str, user: serde_json::Value, extra: serde_json::Value| {
    let mut v = json!({"type": ty, "message_id": "00004a", "user_id": user, "emote_id": 1});
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
      o.extend(e.clone());
    }
    NetEvent::Reaction(crate::model::reaction::parse_frame(&v, ty == "reaction:added").unwrap())
  };
  // somebody else reacts: counted, on the post wherever it shows
  app.on_net(frame("reaction:added", json!("7"), json!({"emote_name": "kek"})));
  assert_eq!(chips_of(&app, "00004a")[0], (1, 3, true, true));
  // yours already showed from the press: not counted twice
  app.on_net(frame("reaction:added", json!("9"), json!({"emote_name": "kek"})));
  app.on_net(frame("reaction:removed", json!(9), json!({})));
  assert_eq!(chips_of(&app, "00004a")[0], (1, 3, true, true));
  app.on_net(frame("reaction:removed", json!("7"), json!({})));
  assert_eq!(chips_of(&app, "00004a")[0], (1, 2, true, true));
  // a new emote starts a chip; a flagged one is its name only; the server may send no picture at all
  app.on_net(frame("reaction:added", json!("7"), json!({"emote_id": 5, "emote_name": "fresh", "emote_url": KEK})));
  app.on_net(frame(
    "reaction:added",
    json!("7"),
    json!({"emote_id": 6, "emote_name": "nsfw", "emote_url": KEK, "nsfw": true}),
  ));
  app.on_net(frame(
    "reaction:added",
    json!("7"),
    json!({"emote_id": 7, "emote_name": "cw", "emote_url": KEK, "cw_cats": ["gore"]}),
  ));
  app.on_net(frame("reaction:added", json!("7"), json!({"emote_id": 8, "emote_name": "bare"})));
  app.on_net(frame(
    "reaction:added",
    json!("7"),
    json!({"emote_id": 9, "emote_name": "off", "emote_url": "http://evil.example/x"}),
  ));
  assert_eq!(
    chips_of(&app, "00004a")[1..],
    [(5, 1, false, true), (6, 1, false, false), (7, 1, false, false), (8, 1, false, false), (9, 1, false, false)]
  );
  // the last one leaves: the chip goes
  app.on_net(frame("reaction:removed", json!("7"), json!({"emote_id": 5})));
  assert!(!chips_of(&app, "00004a").iter().any(|c| c.0 == 5));
  // a post that is not here: nothing happens
  let v = json!({"message_id": "00ffff", "user_id": "7", "emote_id": 1, "emote_name": "kek"});
  app.on_net(NetEvent::Reaction(crate::model::reaction::parse_frame(&v, true).unwrap()));
  // the heat a vote moved
  app.on_net(NetEvent::PostHeat { id: "00004a".into(), heat: 42 });
  assert_eq!(app.feed.find("00004a").unwrap().heat, 42);
  app.on_net(NetEvent::PostHeat { id: "00ffff".into(), heat: 1 });
  // signing out forgets who we were
  app.on_net(NetEvent::HsAuth(false));
  assert!(app.hs_uid.is_none());
  while asked.try_recv().is_ok() {}
}

#[test]
fn headless_react_thread_room() {
  use crate::net::hs_ws::Cmd as Hs;
  let op = post("00004a", json!([{"emote_id": 1, "emote_name": "kek", "emote_url": KEK, "count": 1}]));
  let (mut app, mut asked, _) = boot(vec![op.clone()]);
  app.feed_open("00004a");
  let rooms = |asked: &mut Asked| {
    sent(asked)
      .into_iter()
      .filter_map(|c| if let Cmd::Hs(Hs::Thread(r)) = c { Some(r.map(|r| r.to_string())) } else { None })
      .collect::<Vec<_>>()
  };
  assert_eq!(rooms(&mut asked), [Some("00004a".to_owned())], "the open thread's room is joined");
  let re = post("00004b", json!([{"emote_id": 1, "emote_name": "kek", "emote_url": KEK, "count": 3}]));
  app.on_net(NetEvent::Thread { id: "00004a".into(), res: Ok((op, vec![re])) });
  // the reaction lands on the thread's own copy and the feed's
  let v = json!({"message_id": "00004a", "user_id": "7", "emote_id": 1, "emote_name": "kek"});
  app.on_net(NetEvent::Reaction(crate::model::reaction::parse_frame(&v, true).unwrap()));
  let t = app.feed.thread.as_ref().unwrap();
  assert_eq!(t.op.as_ref().unwrap().reactions[0].count, 2);
  assert_eq!(app.feed.posts[0].reactions[0].count, 2);
  assert_eq!(t.replies[0].reactions[0].count, 3, "another post is left alone");
  assert!(app.feed_back());
  assert_eq!(rooms(&mut asked), [None], "closing leaves it");
  assert!(!app.feed_back());
  assert!(rooms(&mut asked).is_empty());
}
