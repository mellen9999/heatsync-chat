// the mentions tab between runs, headless: written soon after a mention, restored as history for the same
// account only, cleared on logout (the scratch store keeps every test off the real config)

use super::*;
use crate::model::mention_file as file;
use crate::model::message::{Extra, Kind as MsgKind, Message, flag};
use crate::net::auth::Me;
use crate::net::{Cmd, NetEvent};
use serde_json::json;

fn me(name: &str) -> Me {
  Me {
    username: name.into(),
    platform: "twitch".into(),
    twitch: Some(name.into()),
    twitch_id: Some(1),
    names: vec![name.to_lowercase().into()],
    kick: None,
    youtube: None,
    packs: vec![],
  }
}

fn app() -> App {
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen".into()], false);
  // a test never pings or notifies
  a.focused = true;
  a
}

fn rows(a: &App) -> Vec<(String, u8)> {
  let t = a.tabs.iter().find(|t| t.kind == Kind::Mentions).unwrap();
  t.view.msgs().map(|m| (m.body.to_string(), m.flags)).collect()
}

fn mention(a: &mut App, id: &str) {
  let mut m = Message::system(now_ms(), Platform::Twitch, "");
  (m.kind, m.id, m.login, m.nick) = (MsgKind::Chat, id.into(), "bob".into(), "bob".into());
  m.body = format!("hey @mellen {id}").into();
  a.scan_mention(Platform::Twitch, "forsen", &mut m);
}

fn fresh() {
  let _ = std::fs::remove_dir_all(store::folder());
}

fn login(a: &mut App, name: &str) {
  a.on_net(NetEvent::Me(Some(me(name))));
}

fn saved(body: &str, at: i64) -> String {
  let mut m = Message::system(at, Platform::Twitch, "");
  (m.kind, m.id, m.login, m.nick) = (MsgKind::Chat, "g".into(), "bob".into(), "bob".into());
  m.body = body.into();
  m.x = Some(Box::new(Extra { chan: "forsen".into(), ..Extra::default() }));
  file::encode("mellen", at, [m].iter())
}

fn exists(name: &str) -> bool {
  store::path_of(name).unwrap().exists()
}

#[test]
fn a_mention_is_written_once_soon_and_comes_back_for_the_same_account_as_history() {
  fresh();
  let mut a = app();
  login(&mut a, "Mellen");
  mention(&mut a, "1");
  mention(&mut a, "2");
  assert!(a.mentions_save.is_some(), "a write is due");
  assert!(!exists(file::FILE), "not yet: the burst is one write");
  a.mentions_flush();
  assert!(a.mentions_save.is_none());
  let mode = std::fs::metadata(store::path_of(file::FILE).unwrap()).unwrap().permissions();
  assert_eq!(std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777, 0o600);
  // the next run
  let mut b = app();
  assert!(rows(&b).is_empty());
  login(&mut b, "mellen");
  let got = rows(&b);
  assert_eq!(got.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["hey @mellen 1", "hey @mellen 2"]);
  assert!(got.iter().all(|r| r.1 & flag::HISTORY != 0 && r.1 & flag::ALERT != 0), "history, still red-edged");
  assert!(b.flash.is_none() && b.mentions_save.is_none(), "history never flashes and needs no write");
  // the same account checked again (a renewal) does not double them
  login(&mut b, "mellen");
  assert_eq!(rows(&b).len(), 2);
}

#[test]
fn another_account_never_gets_them_and_logout_clears_the_file() {
  fresh();
  let mut a = app();
  login(&mut a, "mellen");
  mention(&mut a, "1");
  a.mentions_flush();
  let mut other = app();
  login(&mut other, "someone");
  assert!(rows(&other).is_empty());
  assert!(exists(file::FILE), "left for its owner");
  // logging out wipes the file and the tab
  a.on_net(NetEvent::Me(None));
  assert!(!exists(file::FILE));
  assert!(rows(&a).is_empty());
  let mut again = app();
  login(&mut again, "mellen");
  assert!(rows(&again).is_empty());
}

#[test]
fn switching_accounts_in_one_run_drops_the_last_ones_mentions() {
  fresh();
  let mut a = app();
  login(&mut a, "mellen");
  mention(&mut a, "1");
  a.mentions_flush();
  login(&mut a, "someone");
  assert!(rows(&a).is_empty());
  assert!(!exists(file::FILE));
}

#[test]
fn a_day_old_file_is_dropped_and_a_corrupt_or_huge_one_is_set_aside() {
  fresh();
  store::write(file::FILE, &saved("old", now_ms() - file::MAX_AGE_MS - 5000)).unwrap();
  let mut a = app();
  login(&mut a, "mellen");
  assert!(rows(&a).is_empty());
  assert!(!exists(file::FILE), "a stale file is removed");
  store::write(file::FILE, "this is not a mentions file\n").unwrap();
  let mut b = app();
  login(&mut b, "mellen");
  assert!(rows(&b).is_empty());
  assert!(exists("mentions.bak"), "kept as mentions.bak, like the other files");
  // too big to be ours: set aside unread
  store::write(file::FILE, &"x".repeat(file::MAX_BYTES as usize + 1)).unwrap();
  let mut c = app();
  login(&mut c, "mellen");
  assert!(rows(&c).is_empty() && !exists(file::FILE));
}

#[test]
fn a_file_with_some_bad_lines_restores_the_good_ones_and_is_written_again() {
  fresh();
  store::write(file::FILE, &format!("{}{{broken\n", saved("good", now_ms()))).unwrap();
  let mut a = app();
  login(&mut a, "mellen");
  assert_eq!(rows(&a).len(), 1);
  assert!(exists("mentions.bak"));
  assert!(a.mentions_save.is_some(), "the good line is written back");
  a.mentions_flush();
  let mut b = app();
  login(&mut b, "mellen");
  assert_eq!(rows(&b).len(), 1);
}

// heatsync notifications are rows of this tab, like the ext's hsNotifIn
fn notif(a: &mut App, v: serde_json::Value) {
  let it = crate::model::inbox::parse(&v, now_ms());
  if let Some(it) = it {
    a.on_net(NetEvent::Notif(it));
  }
}

fn reads(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> usize {
  let mut n = 0;
  while let Ok(c) = rx.try_recv() {
    n += matches!(c, Cmd::Social(crate::net::social::Cmd::NotifsRead)) as usize;
  }
  n
}

#[test]
fn each_kind_is_a_row_in_the_exts_words_and_an_unknown_one_is_dropped() {
  fresh();
  let mut a = app();
  login(&mut a, "mellen");
  let all = [
    json!({"kind": "reply", "message_id": "a1", "content": "lol", "parent_is_op": true}),
    json!({"kind": "reply", "message_id": "a2", "parent_is_op": false}),
    json!({"kind": "reply", "message_id": "a3", "from_username": "Anonymous"}),
    json!({"kind": "mention", "message_id": "a4"}),
    json!({"kind": "feedback_fixed"}),
    json!({"kind": "system", "content": "a terminal logged in", "url": "/cli"}),
    json!({"kind": "follow", "content": "bob followed you"}),
  ];
  for v in all {
    notif(&mut a, v);
  }
  let got: Vec<String> = rows(&a).into_iter().map(|r| r.0).collect();
  assert_eq!(
    got,
    [
      "replied to your post: lol",
      "replied to your reply",
      "anonymous: replied to you",
      "you were mentioned in a post",
      "your report was fixed"
    ]
  );
  let t = a.tabs.iter().find(|t| t.kind == Kind::Mentions).unwrap();
  let m = t.view.last_msg().unwrap();
  assert_eq!((&*m.nick, m.platform), ("heatsync", Platform::Twitch));
}

#[test]
fn nobody_signed_in_keeps_no_notification() {
  fresh();
  let mut a = app();
  notif(&mut a, json!({"kind": "mention", "message_id": "ab12"}));
  assert!(rows(&a).is_empty() && !a.notif_unread);
}

#[test]
fn a_notification_lights_the_mentions_tab_and_looking_reads_it_once() {
  fresh();
  let (net, mut rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen".into()], false);
  a.focused = true;
  login(&mut a, "mellen");
  let tab = a.tab_of(Kind::Mentions).unwrap();
  assert!(!a.tabs[tab].unread);
  notif(&mut a, json!({"kind": "reply", "message_id": "ab12", "content": "hi"}));
  assert!(a.tabs[tab].unread, "the tab is lit like for any mention");
  assert!(a.mentions_save.is_some(), "and the row is kept across restarts");
  reads(&mut rx);
  a.select(tab);
  assert!(!a.tabs[tab].unread);
  assert_eq!(reads(&mut rx), 1, "the account is told they are read");
  // looking again with nothing new asks for nothing
  a.unseen_clear(crate::model::unseen::Surface::Mentions);
  assert_eq!(reads(&mut rx), 0);
  // one arriving while you look is read at once
  notif(&mut a, json!({"kind": "mention", "message_id": "cd34"}));
  assert_eq!(reads(&mut rx), 1);
  assert!(!a.tabs[tab].unread);
}

#[test]
fn enter_on_a_notification_opens_its_post_in_the_feed() {
  fresh();
  let mut a = app();
  a.canvas.resize(720, 360);
  login(&mut a, "mellen");
  notif(&mut a, json!({"kind": "mention", "message_id": "ab12", "content": "look"}));
  let tab = a.tab_of(Kind::Mentions).unwrap();
  a.select(tab);
  let id = a.tabs[tab].view.last_msg().unwrap().id.clone();
  let h = a.chat_area().h;
  let res = &mut a.res;
  assert!(a.tabs[tab].view.select_id(&id, res, h));
  a.jump_to_mention();
  let feed = a.tab_of(Kind::Feed).unwrap();
  assert_eq!(a.active, feed);
  assert_eq!(a.feed.thread.as_ref().map(|t| &*t.id), Some("ab12"));
}

#[test]
fn notification_rows_survive_a_restart_as_history_and_the_next_look_reads_them() {
  fresh();
  let mut a = app();
  login(&mut a, "mellen");
  notif(&mut a, json!({"kind": "reply", "message_id": "ab12", "content": "yo", "parent_is_op": true}));
  mention(&mut a, "1");
  a.mentions_flush();
  let (net, mut rx) = Net::tap();
  let mut b = App::new(net, vec!["forsen".into()], false);
  b.focused = true;
  login(&mut b, "mellen");
  let got = rows(&b);
  assert_eq!(got.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["replied to your post: yo", "hey @mellen 1"]);
  assert!(got.iter().all(|r| r.1 & flag::HISTORY != 0));
  let tab = b.tab_of(Kind::Mentions).unwrap();
  let m = b.tabs[tab].view.msgs().next().unwrap();
  assert_eq!(crate::model::inbox::post_of(m), Some("ab12"));
  reads(&mut rx);
  b.select(tab);
  assert_eq!(reads(&mut rx), 1);
}

#[test]
fn logout_or_another_login_clears_them() {
  fresh();
  let mut a = app();
  login(&mut a, "mellen");
  notif(&mut a, json!({"kind": "mention", "message_id": "ab12"}));
  a.mentions_flush();
  login(&mut a, "someone");
  assert!(rows(&a).is_empty() && !a.notif_unread && !exists(file::FILE));
  notif(&mut a, json!({"kind": "mention", "message_id": "ab12"}));
  a.on_net(NetEvent::Me(None));
  assert!(rows(&a).is_empty() && !a.notif_unread);
}

// a notification row in the file with a hostile id or body is junk, never a crash
#[test]
fn a_hostile_notification_line_in_the_file_is_dropped() {
  use crate::model::mention_file::{Read, decode};
  let head = json!({"v": 1, "owner": "mellen", "ts": now_ms()}).to_string();
  let line = |id: &str| json!({"id": id, "body": "x\u{7}y", "ts": 1}).to_string();
  let file = format!("{head}\n{}\n{}\n{}\n", line("hsn-m-1-ab"), line("hsn-q-1-ab"), line("hsn-m-1-../etc"));
  let Read::Rows { rows, junk } = decode(&file, "mellen", now_ms()) else { panic!() };
  assert_eq!((rows.len(), junk), (1, 2));
  assert_eq!(&*rows[0].body, "xy");
}

// the way out and the update restart both go through flush_to_disk: nothing waiting is lost
#[test]
fn leaving_or_restarting_writes_the_mentions_and_the_settings_that_were_waiting() {
  fresh();
  let mut a = app();
  login(&mut a, "mellen");
  mention(&mut a, "1");
  a.zoom_by(1);
  a.zoom_by(1);
  assert!(a.mentions_save.is_some() && !exists(file::FILE), "the mention waits");
  assert!(!exists("settings"), "a zoom step doesn't touch the disk on the ui thread");
  a.flush_to_disk();
  assert!(a.mentions_save.is_none() && exists(file::FILE));
  assert!(exists("settings"));
  assert!(store::read("settings").is_some_and(|s| s.contains("zoom=")));
}

// a highlight rule's sound: the rule's own preset, for a live line from someone else in its channel
#[test]
fn a_highlight_rules_sound_plays_for_a_live_line_in_its_channel() {
  use crate::util::sound::Preset;
  let rules = json!([
    { "id": "a", "enabled": true, "scope": "forsen", "sound": "knock", "color": "#ff0000", "action": "highlight",
      "match": { "type": "keyword", "value": "gg" } },
    { "id": "b", "enabled": true, "scope": "all", "sound": "blip", "action": "highlight",
      "match": { "type": "keyword", "value": "hype" } },
    { "id": "c", "enabled": true, "scope": "all", "sound": "chime", "action": "hide",
      "match": { "type": "keyword", "value": "spam" } },
  ]);
  let a = app();
  assert!(crate::model::filter::edit(|f| f.set_rules_text(&rules.to_string())));
  let say = |login: &str, body: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, "");
    (m.kind, m.login, m.nick, m.body) = (MsgKind::Chat, login.into(), login.into(), body.into());
    m
  };
  assert_eq!(a.line_sound("forsen", &say("bob", "gg wp")), Some(Preset::Knock));
  assert_eq!(a.line_sound("xqc", &say("bob", "gg wp")), None, "the rule is for forsen's tab");
  assert_eq!(a.line_sound("xqc", &say("bob", "so hype")), Some(Preset::Blip));
  assert_eq!(a.line_sound("forsen", &say("bob", "spam gg")), None, "a hidden line is silent");
  assert_eq!(a.line_sound("forsen", &say("bob", "nothing")), None);
  let mut old = say("bob", "gg");
  old.flags |= flag::HISTORY;
  assert_eq!(a.line_sound("forsen", &old), None, "backlog is silent");
  let mut mine = say("bob", "gg");
  mine.flags |= flag::OWN;
  assert_eq!(a.line_sound("forsen", &mine), None, "your own line is silent");
  crate::model::filter::edit(|f| f.block("bob"));
  assert_eq!(a.line_sound("forsen", &say("bob", "gg")), None, "a blocked person never sounds");
}

// twitch shared chat sends one line to every room of the session. you hear it from the first, and the
// mentions tab holds it once; each room's own tab still shows its copy (it is that room's chat)
#[test]
fn a_shared_chat_line_lands_in_mentions_once_whichever_room_it_comes_through() {
  fresh();
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen".into(), "xqc".into()], false);
  a.focused = true;
  login(&mut a, "mellen");
  let line = |id: &str, source: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, "");
    (m.kind, m.id, m.login, m.nick) = (MsgKind::Chat, id.into(), "bob".into(), "bob".into());
    m.body = format!("hey @mellen {source}").into();
    if source != "-" {
      m.extra().source = source.into();
    }
    m
  };
  let in_room = |a: &App, name: &str| {
    let t = a.tabs.iter().find(|t| &*t.name == name).unwrap();
    t.view.msgs().filter(|m| m.kind == MsgKind::Chat).map(|m| m.flags & flag::ALERT != 0).collect::<Vec<_>>()
  };
  // the same line in both rooms: one mention row, but both rooms show it, red-edged
  a.on_chat("forsen", Chat::Msg(line("a1", "src-1")));
  a.on_chat("xqc", Chat::Msg(line("b1", "src-1")));
  assert_eq!(rows(&a).len(), 1, "the mentions tab holds it once");
  assert_eq!(in_room(&a, "forsen"), [true]);
  assert_eq!(in_room(&a, "xqc"), [true], "the other room's copy is still that room's line");
  // another line is another source id: its own row
  a.on_chat("xqc", Chat::Msg(line("b2", "src-2")));
  a.on_chat("forsen", Chat::Msg(line("a2", "src-2")));
  assert_eq!(rows(&a).len(), 2);
  // lines with no source id are never merged: two people can say the same thing
  a.on_chat("forsen", Chat::Msg(line("a3", "-")));
  a.on_chat("xqc", Chat::Msg(line("b3", "-")));
  assert_eq!(rows(&a).len(), 4);
  // memory is bounded: a long session forgets the oldest ids instead of growing
  for i in 0..2000 {
    let mut m = Message::system(now_ms(), Platform::Twitch, "");
    m.extra().source = format!("flood-{i}").into();
    a.mentions.again(&m);
  }
  assert!(!a.mentions.again(&line("x", "src-1")), "the oldest id was forgotten, so it counts as new");
}

// shared chat with one room muted: the muted room's copy is silent and must not use up the line's one ping
#[test]
fn a_muted_rooms_shared_copy_does_not_swallow_the_unmuted_rooms_ping() {
  fresh();
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen".into(), "xqc".into()], false);
  a.focused = false;
  login(&mut a, "mellen");
  let line = |id: &str, source: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, "");
    (m.kind, m.id, m.login, m.nick) = (MsgKind::Chat, id.into(), "bob".into(), "bob".into());
    m.body = format!("hey @mellen {source}").into();
    m.extra().source = source.into();
    m
  };
  let pings = |a: &mut App, room: &str, m: Message| {
    a.flash = None;
    a.on_chat(room, Chat::Msg(m));
    a.flash.is_some()
  };
  let rows = |a: &App| a.tabs.iter().find(|t| t.kind == Kind::Mentions).unwrap().view.msgs().count();
  a.pings.set_mute(Platform::Twitch, "forsen", true);
  assert!(!pings(&mut a, "forsen", line("a1", "src-1")), "muted copy is silent");
  assert!(pings(&mut a, "xqc", line("b1", "src-1")), "the unmuted room's copy still pings");
  assert_eq!(rows(&a), 1, "one row for the line");
  assert!(!pings(&mut a, "forsen", line("a2", "src-1")), "and only once");
  // both unmuted: still one ping for the line
  a.pings.set_mute(Platform::Twitch, "forsen", false);
  assert!(pings(&mut a, "forsen", line("a3", "src-2")));
  assert!(!pings(&mut a, "xqc", line("b3", "src-2")), "the second copy stays quiet");
  assert_eq!(rows(&a), 2);
}
