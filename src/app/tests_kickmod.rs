// kick moderation, headless: the app learns it moderates a kick room from the badge on our own
// line, then a kick line's delete / timeout / ban reach the net as kick mod requests

use super::*;
use crate::model::message::{Kind as MsgKind, Message};
use crate::model::modcmd::Verb;
use crate::net::Cmd;
use crate::net::auth::Me;

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

fn line(p: Platform, login: &str, id: &str, badges: &[&str]) -> Message {
  let mut m = Message::system(now_ms(), p, "hi");
  m.kind = MsgKind::Chat;
  m.login = login.into();
  m.nick = login.into();
  m.id = id.into();
  m.badges = badges.iter().map(|b| ((*b).into(), "1".into())).collect();
  m
}

fn relay(app: &mut App, m: Message) {
  let (platform, key) =
    if m.platform == Platform::Kick { (Platform::Kick, "xqc") } else { (Platform::Youtube, "@abc") };
  app.on_net(NetEvent::Relay { platform, key: key.into(), msgs: vec![m], history: false });
}

fn mods(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<crate::net::modact::Req> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Mod(r) => Some(r),
      _ => None,
    })
    .collect()
}

#[test]
fn our_own_badge_says_whether_we_moderate_a_kick_room() {
  let (mut app, _rx) = boot();
  assert!(!app.can_mod(Platform::Kick, "xqc"));
  // someone else's moderator badge is not ours
  relay(&mut app, line(Platform::Kick, "bob", "1", &["moderator"]));
  assert!(!app.can_mod(Platform::Kick, "xqc"));
  // our line (kick folds `_` into `-`) with the badge
  relay(&mut app, line(Platform::Kick, "mellen-k", "2", &["moderator"]));
  assert!(app.can_mod(Platform::Kick, "xqc"));
  // not youtube (it has its own room and role), and the twitch room of the same tab is its own question
  assert!(!app.can_mod(Platform::Youtube, "@abc"));
  assert!(!app.can_mod(Platform::Twitch, "forsen"));
  // a later line without the badge: we were unmodded
  relay(&mut app, line(Platform::Kick, "mellen-k", "3", &["subscriber"]));
  assert!(!app.can_mod(Platform::Kick, "xqc"));
  // our own channel is ours before any line
  assert!(app.can_mod(Platform::Kick, "mellen-k"));
  // logging out takes it all
  app.on_net(NetEvent::Me(None));
  assert!(!app.can_mod(Platform::Kick, "mellen-k"));
}

#[test]
fn a_kick_lines_actions_become_kick_requests() {
  let (mut app, mut rx) = boot();
  relay(&mut app, line(Platform::Kick, "mellen-k", "2", &["broadcaster"]));
  let bob = line(Platform::Kick, "bob", "m1", &[]);
  relay(&mut app, bob.clone());
  let who = app.subject_of(&bob).expect("a kick line has a person");
  assert_eq!((who.platform, &*who.chan), (Platform::Kick, "xqc"));
  // the menu offers delete + timeouts + ban + unban, no purge or blocked terms
  let rows: Vec<String> = app.person_rows(&who).into_iter().map(|r| r.1).collect();
  assert!(rows.iter().any(|r| r == "delete message") && rows.iter().any(|r| r == "timeout 10m"));
  assert!(rows.iter().any(|r| r == "ban") && rows.iter().any(|r| r == "unban"));
  assert!(!rows.iter().any(|r| r == "purge" || r == "blocked terms"));
  app.mod_run(Verb::Delete, vec![who.clone()], None);
  app.mod_run(Verb::Timeout(600), vec![who], None);
  let got = mods(&mut rx);
  assert_eq!(got.len(), 2);
  assert!(got.iter().all(|r| r.platform == Platform::Kick && &*r.chan == "xqc" && r.room.is_none()));
  assert_eq!((got[0].verb, got[0].msg_id.as_deref()), (Verb::Delete, Some("m1")));
  assert_eq!((got[1].verb, &*got[1].login), (Verb::Timeout(600), "bob"));
}

#[test]
fn youtube_lines_get_no_mod_rows_until_we_moderate_the_room() {
  let (mut app, _rx) = boot();
  let m = line(Platform::Youtube, "bob", "y1", &["moderator"]);
  relay(&mut app, m.clone());
  let who = app.subject_of(&m).expect("person");
  let rows: Vec<String> = app.person_rows(&who).into_iter().map(|r| r.1).collect();
  assert!(!rows.iter().any(|r| r == "ban" || r == "delete message"));
}

// youtube: our own line's moderator badge makes the room ours, the menu offers the youtube-capable rows (no
// purge), and a delete / timeout names the room, the chatter's UC id and (delete) the line's time and words
#[test]
fn a_youtube_lines_actions_become_youtube_requests() {
  let (mut app, mut rx) = boot();
  assert!(!app.can_mod(Platform::Youtube, "@abc"));
  // someone else's badge is not ours; our line with one is
  relay(&mut app, line(Platform::Youtube, "bob", "y0", &["moderator"]));
  assert!(!app.can_mod(Platform::Youtube, "@abc"));
  relay(&mut app, line(Platform::Youtube, "mellenyt", "y1", &["moderator"]));
  assert!(app.can_mod(Platform::Youtube, "@abc") && app.can_mod(Platform::Youtube, "@ABC"));
  let mut bob = line(Platform::Youtube, "bob", "y2", &[]);
  bob.user_id = "UCabcdefghijklmnopqrstuv".into();
  bob.body = "buy followers".into();
  let ts = bob.ts;
  // (youtube's lines drip out at their own pace: this one goes straight onto the tab)
  let chat = app.tabs.iter().position(|t| t.kind == crate::model::tabs::Kind::Chat).unwrap();
  app.tabs[chat].chat().push(bob.clone());
  let who = app.subject_of(&bob).expect("a person");
  let rows: Vec<String> = app.person_rows(&who).into_iter().map(|r| r.1).collect();
  assert!(rows.iter().any(|r| r == "delete message") && rows.iter().any(|r| r == "timeout 10m"));
  assert!(rows.iter().any(|r| r == "ban") && !rows.iter().any(|r| r == "purge" || r == "blocked terms"));
  app.mod_run(Verb::Delete, vec![who.clone()], None);
  app.mod_run(Verb::Timeout(600), vec![who], None);
  let got = mods(&mut rx);
  assert_eq!(got.len(), 2);
  assert!(got.iter().all(|r| r.platform == Platform::Youtube && &*r.chan == "@abc" && r.room.is_none()));
  assert!(got.iter().all(|r| r.user_id.as_deref() == Some("UCabcdefghijklmnopqrstuv")));
  assert_eq!((got[0].verb, got[0].msg_id.as_deref()), (Verb::Delete, Some("y2")));
  assert_eq!((got[0].msg_ts, got[0].msg_text.as_deref()), (Some(ts), Some("buy followers")));
  assert_eq!((got[1].verb, got[1].msg_ts), (Verb::Timeout(600), None));
  // our line without the badge: unmodded
  relay(&mut app, line(Platform::Youtube, "mellenyt", "y3", &[]));
  assert!(!app.can_mod(Platform::Youtube, "@abc"));
  // our own channel is ours before any line
  assert!(app.can_mod(Platform::Youtube, "@mellenyt"));
}
