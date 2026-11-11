// the whole app headless, send to several feeds: which feeds a line reaches, the tightest limit, a timed-out
// feed skipped by name, and one send's echoes folding into one row

use super::*;
use crate::model::lock::{Cause, Lock};
use crate::model::message::{Kind as MsgKind, Message};
use crate::net::Cmd;
use crate::net::auth::Me;
use crate::net::irc::Chat;

fn me() -> Me {
  Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: Some("mellenk".into()),
    youtube: Some("mellenyt".into()),
    packs: vec![],
  }
}

// a tab on all three platforms, signed in
fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen kick:xqc yt:@abc".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(520, 300);
  app.on_net(NetEvent::Me(Some(me())));
  // the youtube relay is connected: that feed is live and takes lines
  app.on_net(NetEvent::YtStatus { key: "@abc".into(), status: "connected".into() });
  app.redraw();
  while rx.try_recv().is_ok() {}
  (app, rx)
}

// what a send put on the wire: (platform, channel, text, has a reply)
fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(Platform, String, String, bool)> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Send { platform, chan, text, reply, .. } => Some((platform, chan.to_string(), text, reply.is_some())),
      _ => None,
    })
    .collect()
}

fn type_and_send(app: &mut App, text: &str) {
  app.line.set_text(text.into());
  app.send();
}

fn own(p: Platform, body: &str) -> Message {
  let mut m = Message::system(now_ms(), p, body);
  m.kind = MsgKind::Chat;
  m.login = "mellen".into();
  m.nick = "mellen".into();
  m
}

fn rows(app: &App, body: &str) -> usize {
  app.tabs[app.active].view.bodies().iter().filter(|b| *b == body).count()
}

#[test]
fn a_line_goes_to_every_feed_of_the_tab() {
  let (mut app, mut rx) = boot();
  type_and_send(&mut app, "hello");
  let got = sent(&mut rx);
  let want = [(Platform::Twitch, "forsen"), (Platform::Kick, "xqc"), (Platform::Youtube, "@abc")];
  assert_eq!(got.len(), 3, "{got:?}");
  for ((p, c, t, _), (wp, wc)) in got.iter().zip(want) {
    assert_eq!((*p, c.as_str(), t.as_str()), (wp, wc, "hello"));
  }
  assert_eq!(app.line.text, "", "the box empties once");
}

// the network part crashed: the line stays in the box and the user is told, nothing is "sent" into the void
#[test]
fn a_line_stays_in_the_box_when_the_network_is_gone() {
  let (mut app, rx) = boot();
  drop(rx);
  type_and_send(&mut app, "hello");
  assert_eq!(app.line.text, "hello", "not lost");
  let said = app.tabs[app.active].view.bodies();
  assert!(said.iter().any(|b| b.contains("not sent") && b.contains("restart heatsync-chat")), "{said:?}");
}

#[test]
fn ctrl_n_steps_all_then_each_and_alt_keys_pick_a_pair() {
  let (mut app, mut rx) = boot();
  let who = |rx: &mut _| sent(rx).into_iter().map(|s| s.0).collect::<Vec<_>>();
  app.cycle_send();
  type_and_send(&mut app, "a");
  assert_eq!(who(&mut rx), [Platform::Twitch]);
  app.cycle_send();
  type_and_send(&mut app, "b");
  assert_eq!(who(&mut rx), [Platform::Kick]);
  app.cycle_send();
  app.cycle_send();
  type_and_send(&mut app, "c");
  assert_eq!(who(&mut rx).len(), 3, "back to all");
  // kick off: twitch + youtube; the box speaks as twitch
  assert!(app.toggle_send(Platform::Kick));
  type_and_send(&mut app, "d");
  assert_eq!(who(&mut rx), [Platform::Twitch, Platform::Youtube]);
  assert_eq!(app.target().map(|t| t.0), Some(Platform::Twitch));
  // the last two can't both go
  assert!(app.toggle_send(Platform::Twitch));
  assert!(app.toggle_send(Platform::Youtube));
  type_and_send(&mut app, "e");
  assert_eq!(who(&mut rx), [Platform::Youtube], "the last feed stays on");
  // a platform the tab doesn't have is not ours to handle
  let (net, _rx) = Net::tap();
  let mut solo = App::new(net, vec!["forsen".into()], false);
  assert!(!solo.toggle_send(Platform::Kick));
}

#[test]
fn the_box_tag_and_limit_follow_the_feeds() {
  let (mut app, _rx) = boot();
  let tag = |app: &App| app.send_tag().iter().map(|p| p.0).collect::<String>();
  assert_eq!(tag(&app), "[TKY] ");
  let legs = |app: &App| app.legs().into_iter().map(|l| l.0).collect::<Vec<_>>();
  assert_eq!(crate::model::sendto::limit(&legs(&app)), 200, "youtube is the tightest");
  app.toggle_send(Platform::Youtube);
  assert_eq!(tag(&app), "[TK] ");
  assert_eq!(crate::model::sendto::limit(&legs(&app)), 500);
  // a pair steps back to all, then each alone
  app.cycle_send();
  assert_eq!(tag(&app), "[TKY] ");
  app.cycle_send();
  assert_eq!(tag(&app), "[T] ");
  // the box goes red past the tightest limit among the feeds
  app.cycle_send();
  app.cycle_send();
  app.cycle_send();
  assert_eq!(tag(&app), "[TKY] ");
  app.line.set_text("x".repeat(201));
  app.redraw();
  assert!(app.canvas.inked(crate::ui::theme::RED) > 0, "201 characters is over for youtube");
}

#[test]
fn a_timed_out_feed_is_skipped_by_name_and_the_others_still_get_it() {
  let (mut app, mut rx) = boot();
  let n = now_ms();
  app.locks.put(Platform::Kick, "xqc", Lock { cause: Cause::Timeout(60), until: Some(n + 60_000) }, n);
  type_and_send(&mut app, "hi");
  let got = sent(&mut rx);
  assert_eq!(got.iter().map(|s| s.0).collect::<Vec<_>>(), [Platform::Twitch, Platform::Youtube]);
  let said = app.tabs[app.active].view.bodies();
  assert!(said.iter().any(|b| b.starts_with("skipped kick: you are timed out")), "{said:?}");
  // every feed out: nothing goes and the words stay
  app.locks.put(Platform::Twitch, "forsen", Lock { cause: Cause::Timeout(60), until: Some(n + 60_000) }, n);
  app.locks.put(Platform::Youtube, "@abc", Lock { cause: Cause::Timeout(60), until: Some(n + 60_000) }, n);
  type_and_send(&mut app, "again");
  assert!(sent(&mut rx).is_empty());
  assert_eq!(app.line.text, "again");
}

#[test]
fn the_echoes_of_one_send_show_once() {
  let (mut app, mut rx) = boot();
  type_and_send(&mut app, "gg wp");
  let _ = sent(&mut rx);
  // twitch comes back first, then kick and youtube through the relay
  app.on_chat("forsen", Chat::Msg(own(Platform::Twitch, "gg wp")));
  app.on_net(NetEvent::Relay {
    platform: Platform::Kick,
    key: "xqc".into(),
    msgs: vec![own(Platform::Kick, "gg wp")],
    history: false,
  });
  app.on_net(NetEvent::Relay {
    platform: Platform::Youtube,
    key: "@abc".into(),
    msgs: vec![own(Platform::Youtube, "gg wp")],
    history: false,
  });
  assert_eq!(rows(&app, "gg wp"), 1);
  // someone else saying the same words is not your echo
  let mut other = own(Platform::Kick, "gg wp");
  other.login = "bob".into();
  other.nick = "bob".into();
  app.on_net(NetEvent::Relay { platform: Platform::Kick, key: "xqc".into(), msgs: vec![other], history: false });
  assert_eq!(rows(&app, "gg wp"), 2);
  // a partial return (youtube never answers) is still one row, whichever feed is first
  type_and_send(&mut app, "second");
  app.on_net(NetEvent::Relay {
    platform: Platform::Kick,
    key: "xqc".into(),
    msgs: vec![own(Platform::Kick, "second")],
    history: false,
  });
  app.on_chat("forsen", Chat::Msg(own(Platform::Twitch, "second")));
  assert_eq!(rows(&app, "second"), 1);
  // a send to one feed is never folded: its echo shows
  app.cycle_send();
  type_and_send(&mut app, "solo");
  app.on_chat("forsen", Chat::Msg(own(Platform::Twitch, "solo")));
  assert_eq!(rows(&app, "solo"), 1);
}

// a reply threads on its own platform; youtube gets the @name; the rest send plain
#[test]
fn a_reply_reaches_each_feed_in_its_own_way() {
  let (mut app, mut rx) = boot();
  let mut m = own(Platform::Kick, "hey");
  m.login = "bob".into();
  m.nick = "bob".into();
  m.id = "kid".into();
  app.on_net(NetEvent::Relay { platform: Platform::Kick, key: "xqc".into(), msgs: vec![m], history: false });
  app.tabs[app.active].view.select_start(&mut app.res, 300);
  app.reply_selected();
  type_and_send(&mut app, "yes");
  let got = sent(&mut rx);
  assert_eq!(got.len(), 3);
  assert_eq!((got[0].2.as_str(), got[0].3), ("yes", false), "twitch can't thread a kick line");
  assert_eq!((got[1].2.as_str(), got[1].3), ("yes", true), "kick threads it");
  assert_eq!((got[2].2.as_str(), got[2].3), ("@bob yes", false), "youtube opens with the @name");
}

// the choice rides in the tab's entry in the ext's shape: ctrl-n goes up to the account, and a choice made on
// another client lays onto the tab even when the list of tabs is the same:
//   cargo test -j2 --release headless_send_targets_sync
#[test]
fn headless_send_targets_sync() {
  use crate::model::tabsync::frame;
  use crate::net::hs_ws::Cmd as HsCmd;
  use serde_json::json;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen kick:xqc".into()], false);
  app.me = Some(me());
  app.hs_authed = true;
  let push = |app: &mut App, v: serde_json::Value| app.on_net(NetEvent::TabSync(frame(&v).unwrap()));
  push(
    &mut app,
    json!({"type": "multichat:config", "rev": 1, "channels": [
      {"id": "forsen", "twitch": "forsen", "kick": "xqc", "sendTargets": {"kick": false}, "preview": true},
    ]}),
  );
  assert_eq!(app.legs().into_iter().map(|l| l.0).collect::<Vec<_>>(), [Platform::Twitch], "the account's choice");
  while asked.try_recv().is_ok() {}
  // ctrl-n: twitch alone -> kick alone, written back with the other fields kept
  app.cycle_send();
  let up: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Hs(HsCmd::TabSync(list)) => Some(list),
      _ => None,
    })
    .collect();
  assert_eq!(up.len(), 1, "one list goes up");
  assert_eq!(up[0][0]["sendTargets"], json!({"twitch": false, "kick": true}));
  assert_eq!(up[0][0]["preview"], json!(true));
  // another client turns twitch off: same tabs, new choice
  push(
    &mut app,
    json!({"type": "multichat:config", "rev": 2, "channels": [
      {"id": "forsen", "twitch": "forsen", "kick": "xqc", "sendTargets": {"twitch": false, "kick": true}},
    ]}),
  );
  assert_eq!(app.legs().into_iter().map(|l| l.0).collect::<Vec<_>>(), [Platform::Kick]);
  assert_eq!(app.target().map(|t| t.0), Some(Platform::Kick));
}

#[test]
fn a_youtube_feed_that_isnt_live_is_left_out_of_a_merged_send_quietly() {
  let (mut app, mut rx) = boot();
  app.on_net(NetEvent::YtStatus { key: "@abc".into(), status: "not-live".into() });
  let before = app.tabs[app.active].view.bodies().len();
  type_and_send(&mut app, "hi");
  let got = sent(&mut rx);
  assert_eq!(got.iter().map(|s| s.0).collect::<Vec<_>>(), [Platform::Twitch, Platform::Kick]);
  let after = app.tabs[app.active].view.bodies();
  assert!(!after[before..].iter().any(|b| b.contains("youtube")), "no notice: {after:?}");
  // sent to youtube alone it still goes, so its refusal can say why
  app.cycle_send();
  app.cycle_send();
  app.cycle_send();
  type_and_send(&mut app, "yo");
  assert_eq!(sent(&mut rx).iter().map(|s| s.0).collect::<Vec<_>>(), [Platform::Youtube]);
}

// what was told to heatsync as `chat:origin_tag`: (room, text, send id)
fn origins(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(String, String, String)> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Hs(crate::net::hs_ws::Cmd::Origin(c, t, id, _)) => Some((c.to_string(), t.to_string(), id.to_string())),
      _ => None,
    })
    .collect()
}

// the site tells heatsync once per send, not once per feed; the twitch room is the one named, else kick, else youtube
#[test]
fn a_send_is_told_to_heatsync_once_whatever_the_feeds() {
  let (mut app, mut rx) = boot();
  type_and_send(&mut app, "hello");
  let got = origins(&mut rx);
  assert_eq!(got.len(), 1, "{got:?}");
  assert_eq!((got[0].0.as_str(), got[0].1.as_str()), ("forsen", "hello"));
  assert!(got[0].2.starts_with("hsc-"));
  type_and_send(&mut app, "/me waves");
  assert_eq!(origins(&mut rx)[0].1, "waves", "the platforms echo a /me line bare");
  // one feed at a time: twitch, then kick, then youtube
  for room in ["forsen", "xqc", "@abc"] {
    app.cycle_send();
    type_and_send(&mut app, "solo");
    let got = origins(&mut rx);
    assert_eq!(got.iter().map(|o| o.0.as_str()).collect::<Vec<_>>(), [room]);
  }
}

// a kick emote named in a merged tab: kick gets its token (or kick shows everyone the word), twitch and
// youtube the word; any line in the tab draws it, and it goes with the tab's kick leg
#[test]
fn a_kick_emote_is_a_token_on_kick_a_word_elsewhere_and_drawn_on_every_line() {
  let (mut app, mut rx) = boot();
  let set = crate::emotes::providers::kick(&serde_json::json!([
    {"name": "Global", "emotes": [{"id": 37245, "name": "peepoDJ", "subscribers_only": false}]}
  ]));
  app.on_net(NetEvent::KickEmotes("xqc".into(), set));
  type_and_send(&mut app, "peepoDJ turn it up");
  let got = sent(&mut rx);
  let text = |p| got.iter().find(|g| g.0 == p).map(|g| g.2.as_str());
  assert_eq!(text(Platform::Kick), Some("[emote:37245:peepoDJ] turn it up"));
  assert_eq!(text(Platform::Twitch), Some("peepoDJ turn it up"));
  assert_eq!(text(Platform::Youtube), Some("peepoDJ turn it up"));
  let line = own(Platform::Twitch, "peepoDJ");
  let drawn = |app: &mut App| {
    crate::emotes::resolve::find_word(&mut app.res.sources(), "forsen", &line, 0, 7, "peepoDJ").map(|f| f.0.to_string())
  };
  assert_eq!(drawn(&mut app).as_deref(), Some("https://files.kick.com/emotes/37245/fullsize"));
  let spec = crate::model::chan::Spec::parse("forsen", "", "@abc").unwrap();
  app.edit_tab(app.active, &spec);
  assert_eq!(drawn(&mut app), None, "the kick leg is gone, and its emotes with it");
}

// one line is one send: every leg and the origin tag carry the same id, so heatsync ties the kick leg (which
// carries it to the server) to the twitch one, and the site folds them into one row
#[test]
fn every_leg_of_a_line_carries_its_one_send_id() {
  let (mut app, mut rx) = boot();
  type_and_send(&mut app, "hello");
  let cmds: Vec<Cmd> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
  let legs: Vec<Option<Box<str>>> = cmds
    .iter()
    .filter_map(|c| match c {
      Cmd::Send { send, .. } => Some(send.clone()),
      _ => None,
    })
    .collect();
  let tag = cmds.iter().find_map(|c| match c {
    Cmd::Hs(crate::net::hs_ws::Cmd::Origin(_, _, id, _)) => Some(id.clone()),
    _ => None,
  });
  assert_eq!(legs.len(), 3, "{legs:?}");
  let id = tag.expect("the origin tag went out");
  assert!(id.starts_with("hsc-"), "{id}");
  assert!(legs.iter().all(|l| l.as_deref() == Some(&*id)), "{legs:?} vs {id}");
  // the next line is another send
  type_and_send(&mut app, "again");
  let next = std::iter::from_fn(|| rx.try_recv().ok()).find_map(|c| match c {
    Cmd::Send { send, .. } => send,
    _ => None,
  });
  assert!(next.is_some());
}
