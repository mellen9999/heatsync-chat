// heatsync's word that a line went through heatsync (`chat:origin`, the room's seed), for anyone's lines: keyed
// by message id, born [H] or flipped in place, one send on two feeds one row, and never matched by words
use super::*;
use crate::model::mark::Mark;
use crate::model::message::{Kind as MsgKind, Message};
use crate::net::irc::Chat;

fn app() -> App {
  let _ = std::fs::remove_dir_all(store::folder());
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen kick:xqc".into()], false);
  a.dpi = 1.0;
  a.res.set_scale(a.scale());
  a.resized(720, 400);
  a
}

fn said(p: Platform, id: &str, who: &str, body: &str) -> Message {
  let mut m = Message::system(now_ms(), p, "x");
  (m.kind, m.id, m.login, m.nick, m.body) = (MsgKind::Chat, id.into(), who.into(), who.into(), body.into());
  m
}

fn tw(a: &mut App, id: &str, body: &str) {
  a.on_chat("forsen", Chat::Msg(said(Platform::Twitch, id, "amy", body)));
}

fn kick(a: &mut App, id: &str, body: &str) {
  a.on_relay(Platform::Kick, "xqc", vec![said(Platform::Kick, id, "amy", body)], false);
}

fn push(a: &mut App, p: Platform, key: &str, id: &str, sid: &str) {
  a.on_net(NetEvent::HsOrigin { platform: p, key: key.into(), ids: vec![(id.into(), sid.into())] });
}

fn marks(a: &App) -> Vec<Mark> {
  let i = a.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  a.tabs[i].view.msgs().map(Mark::of).collect()
}

fn shown(a: &App) -> usize {
  let i = a.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  a.tabs[i].view.found()
}

#[test]
fn a_line_on_screen_flips_to_h_when_the_push_lands() {
  let mut a = app();
  tw(&mut a, "t1", "gg");
  kick(&mut a, "k1", "gg");
  assert_eq!(marks(&a), [Mark::Twitch, Mark::Kick]);
  push(&mut a, Platform::Kick, "xqc", "k1", "");
  assert_eq!(marks(&a), [Mark::Twitch, Mark::Heatsync], "only the named line, only upward");
}

#[test]
fn a_push_before_the_line_makes_it_born_h() {
  let mut a = app();
  push(&mut a, Platform::Twitch, "forsen", "t1", "");
  tw(&mut a, "t1", "gg");
  tw(&mut a, "t2", "gg");
  assert_eq!(marks(&a), [Mark::Heatsync, Mark::Twitch], "by id: the same words on another line stay [T]");
}

#[test]
fn words_alone_never_make_someone_elses_line_h() {
  let mut a = app();
  push(&mut a, Platform::Twitch, "forsen", "t1", "");
  tw(&mut a, "t9", "same words");
  assert_eq!(marks(&a), [Mark::Twitch]);
}

#[test]
fn one_send_on_two_feeds_is_one_row() {
  let mut a = app();
  tw(&mut a, "t1", "gg");
  kick(&mut a, "k1", "gg");
  assert_eq!(shown(&a), 2);
  push(&mut a, Platform::Twitch, "forsen", "t1", "S");
  push(&mut a, Platform::Kick, "xqc", "k1", "S");
  assert_eq!(shown(&a), 1, "equal send ids fold");
  tw(&mut a, "t2", "again");
  kick(&mut a, "k2", "again");
  push(&mut a, Platform::Twitch, "forsen", "t2", "S2");
  push(&mut a, Platform::Kick, "xqc", "k2", "S3");
  assert_eq!(shown(&a), 3, "different send ids never fold");
}

#[test]
fn the_seed_marks_what_it_names_and_a_born_copy_folds() {
  let mut a = app();
  tw(&mut a, "t1", "gg");
  a.on_net(NetEvent::HsOrigin {
    platform: Platform::Twitch,
    key: "forsen".into(),
    ids: vec![("t1".into(), "S".into())],
  });
  push(&mut a, Platform::Kick, "xqc", "k1", "S");
  kick(&mut a, "k1", "gg");
  assert_eq!(shown(&a), 1, "the kick copy is born [H] with the same send id: folded");
}

// a socket that came back re-reads the origins of every open room: a push missed in the drop must not leave
// a line [T]/[K]/[Y]
#[test]
fn a_reconnect_reseeds_every_open_room_once() {
  let (net, mut rx) = Net::tap();
  let _ = std::fs::remove_dir_all(store::folder());
  let mut a = App::new(net, vec!["forsen kick:xqc".into(), "forsen".into()], false);
  a.resized(720, 400);
  while rx.try_recv().is_ok() {}
  a.on_net(NetEvent::HsUp);
  let mut got: Vec<(Platform, String)> = std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| if let crate::net::Cmd::Seed(p, k) = c { Some((p, k.to_string())) } else { None })
    .collect();
  got.sort_by_key(|(p, k)| (p.name(), k.clone()));
  assert_eq!(got, [(Platform::Kick, "xqc".into()), (Platform::Twitch, "forsen".into())], "each room once");
}
