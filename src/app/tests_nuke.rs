// /nuke <term> [seconds], headless: which lines it picks, what it asks, what goes to the net, what it says after

use super::*;
use crate::model::message::{Kind as MsgKind, Message, flag};
use crate::net::Cmd;
use crate::net::auth::Me;
use crate::net::irc::Chat;
use crate::net::modact::{Fail, NukeDone};
use winit::keyboard::{Key, NamedKey};

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
    youtube: None,
    packs: vec![],
  })));
  app.on_chat("forsen", Chat::Mod(true));
  app.redraw();
  while rx.try_recv().is_ok() {}
  (app, rx)
}

fn line(p: Platform, id: &str, body: &str, age_s: i64) -> Message {
  let mut m = Message::system(now_ms() - age_s * 1000, p, body);
  m.kind = MsgKind::Chat;
  m.login = "bob".into();
  m.nick = "bob".into();
  m.id = id.into();
  m
}

fn twitch(app: &mut App, m: Message) {
  app.on_chat("forsen", Chat::Msg(m));
}

fn kick(app: &mut App, m: Message) {
  app.on_net(NetEvent::Relay { platform: Platform::Kick, key: "xqc".into(), msgs: vec![m], history: false });
}

fn said(app: &App) -> String {
  app.tabs[app.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
}

fn typed(app: &mut App, s: &str) {
  app.line.set_text(s.into());
  app.send();
}

fn head(app: &App) -> Option<String> {
  app.pop().and_then(|p| p.menu.head().map(str::to_owned))
}

fn nukes(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(Vec<crate::net::modact::Req>, String)> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Nuke(r, t) => Some((r, t)),
      _ => None,
    })
    .collect()
}

#[test]
fn it_picks_recent_standing_lines_with_the_term_from_both_platforms_once_each() {
  let (mut app, mut rx) = boot();
  twitch(&mut app, line(Platform::Twitch, "t1", "BUY followers now", 5));
  twitch(&mut app, line(Platform::Twitch, "t2", "hello there", 5));
  // too old for the default 30s, no id to delete by, and one already gone
  twitch(&mut app, line(Platform::Twitch, "t3", "buy this", 100));
  twitch(&mut app, line(Platform::Twitch, "", "buy buy", 1));
  kick(&mut app, line(Platform::Kick, "k1", "cheap buy", 3));
  app.redraw();
  twitch(&mut app, line(Platform::Twitch, "t4", "buy gone", 2));
  app.tabs[app.active].chat().clear_msg("t4");
  typed(&mut app, "/nuke buy");
  assert_eq!(head(&app).as_deref(), Some("delete 2 messages matching \"buy\" in #forsen?"));
  let rows: Vec<String> = app.pop().unwrap().menu.items.iter().map(|r| r.1.clone()).collect();
  assert_eq!(rows, ["delete", "cancel"]);
  // enter on "nuke": one paced job with a delete for each of the two lines, in the right rooms
  app.pop_key(Key::Named(NamedKey::Enter));
  let got = nukes(&mut rx);
  assert_eq!(got.len(), 1);
  let (reqs, term) = &got[0];
  assert_eq!(term, "buy");
  let rooms: Vec<(Platform, String, String)> =
    reqs.iter().map(|r| (r.platform, r.chan.to_string(), r.msg_id.as_deref().unwrap_or("").to_string())).collect();
  assert_eq!(rooms, [(Platform::Twitch, "forsen".into(), "t1".into()), (Platform::Kick, "xqc".into(), "k1".into())]);
  assert!(reqs.iter().all(|r| r.verb == crate::model::modcmd::Verb::Delete));
  assert_eq!(said(&app), "deleting 2 messages\u{2026}");
}

#[test]
fn a_longer_window_reaches_further_back_and_one_line_reads_singular() {
  let (mut app, mut rx) = boot();
  twitch(&mut app, line(Platform::Twitch, "t3", "buy this", 100));
  typed(&mut app, "/nuke buy");
  assert_eq!(said(&app), "/nuke: no messages matching \"buy\" in the last 30s");
  assert!(app.pop().is_none() && nukes(&mut rx).is_empty());
  typed(&mut app, "/nuke BUY 200");
  assert_eq!(head(&app).as_deref(), Some("delete 1 message matching \"BUY\" in #forsen?"));
  // esc: nothing goes
  app.pop_key(Key::Named(NamedKey::Escape));
  assert!(nukes(&mut rx).is_empty());
}

#[test]
fn more_than_a_hundred_keeps_the_oldest_hundred_and_says_so() {
  let (mut app, mut rx) = boot();
  for i in 0..130 {
    twitch(&mut app, line(Platform::Twitch, &format!("m{i}"), "spam spam", 10));
  }
  typed(&mut app, "/nuke spam");
  assert_eq!(head(&app).as_deref(), Some("delete 100+ (capped from 130) messages matching \"spam\" in #forsen?"));
  app.pop_key(Key::Named(NamedKey::Enter));
  let (reqs, _) = nukes(&mut rx).remove(0);
  assert_eq!(reqs.len(), 100);
  assert_eq!(reqs[0].msg_id.as_deref(), Some("m0"));
  assert_eq!(reqs[99].msg_id.as_deref(), Some("m99"));
}

#[test]
fn the_ext_errors_in_the_ext_order() {
  let (mut app, mut rx) = boot();
  // a term under 2 chars is the usage line
  typed(&mut app, "/nuke a");
  assert_eq!(said(&app), "usage: /nuke <term> [seconds] \u{2014} term must be 2+ chars");
  typed(&mut app, "/nuke");
  assert_eq!(said(&app), "usage: /nuke <term> [seconds] \u{2014} term must be 2+ chars");
  // logged out
  app.on_net(NetEvent::Me(None));
  typed(&mut app, "/nuke spam");
  assert_eq!(said(&app), "log in first (ctrl-l)");
  // a tab with only a youtube that is not live (or not ours): it says that, not "twitch or kick"
  let (net, _) = Net::tap();
  let mut yt = App::new(net, vec!["yt:@abc".into()], false);
  yt.dpi = 1.0;
  yt.resized(520, 300);
  yt.me = Some(Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: None,
    names: vec![],
    kick: None,
    youtube: None,
    packs: vec![],
  });
  yt.select(yt.tabs.iter().position(|t| t.sources.iter().any(|s| s.0 == Platform::Youtube)).unwrap());
  typed(&mut yt, "/nuke spam");
  assert_eq!(said(&yt), "youtube skipped: not live, or you haven't chatted there as a mod");
  assert!(nukes(&mut rx).is_empty());
}

#[test]
fn the_summary_counts_what_landed_and_kick_lines_go_grey() {
  let (mut app, mut rx) = boot();
  kick(&mut app, line(Platform::Kick, "k1", "buy", 3));
  app.redraw();
  let done = |ok: Vec<(Platform, Box<str>, Box<str>)>, fail| NukeDone { term: "buy".into(), total: 3, ok, fail };
  app.on_nuke_done(done(vec![(Platform::Kick, "xqc".into(), "k1".into())], Some(Fail::Other("rate".into()))));
  assert_eq!(said(&app), "delete failed: rate");
  let all: Vec<String> = app.tabs[app.active].view.msgs().map(|m| m.body.to_string()).collect();
  assert!(all.iter().any(|b| b == "deleted 1/3 matching \"buy\""), "{all:?}");
  let k = app.tabs[app.active].view.msgs().find(|m| &*m.id == "k1").unwrap();
  assert!(k.flags & flag::CLEARED != 0, "kick echoes nothing: cleared here");
  // all landed: just the count
  app.on_nuke_done(done(vec![], None));
  assert_eq!(said(&app), "deleted 0/3 matching \"buy\"");
  // a login that is out says so, a missing permission starts the relink
  app.on_nuke_done(done(vec![], Some(Fail::Auth)));
  assert_eq!(said(&app), "log in first (ctrl-l)");
  app.on_nuke_done(done(vec![], Some(Fail::Relink)));
  assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, Cmd::Relink("mod"))));
}
