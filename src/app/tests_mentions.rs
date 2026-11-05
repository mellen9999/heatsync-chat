// the mentions tab between runs, headless: written soon after a mention, restored as history for the same
// account only, cleared on logout (the scratch store keeps every test off the real config)

use super::*;
use crate::model::mention_file as file;
use crate::model::message::{Extra, Kind as MsgKind, Message, flag};
use crate::net::NetEvent;
use crate::net::auth::Me;

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
