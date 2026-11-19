// a send that never confirms: told in its tab after the timeout, and alt-enter sends again only the legs that
// never confirmed, under the same send id (ext 7b9a530a)
use super::tests_sendto::{boot, own, type_and_send};
use super::*;
use crate::model::message::Platform::{Kick, Twitch, Youtube};
use crate::model::pending::TIMEOUT_MS;
use crate::net::Cmd;
use crate::net::irc::Chat;
use winit::keyboard::{Key, ModifiersState, NamedKey};

// every send on the wire: (platform, text, send id)
fn out(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(Platform, String, String)> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Send { platform, text, send, .. } => Some((platform, text, send.map(|s| s.to_string()).unwrap_or_default())),
      _ => None,
    })
    .collect()
}

fn notices(app: &App, word: &str) -> usize {
  app.tabs[app.active].view.bodies().iter().filter(|b| b.contains(word)).count()
}

fn leg(app: &mut App, id: &str, p: Platform, chan: &str, landed: bool) {
  app.on_net(NetEvent::Leg { send: id.into(), platform: p, chan: chan.into(), landed });
}

const LATER: i64 = TIMEOUT_MS + 1000;

#[test]
fn an_unconfirmed_send_is_told_and_only_the_missing_leg_goes_again() {
  let (mut app, mut rx) = boot();
  type_and_send(&mut app, "gg");
  let first = out(&mut rx);
  assert_eq!(first.len(), 3);
  let id = first[0].2.clone();
  // twitch echoes; the kick POST was refused (it never went out); youtube's went through
  app.on_chat("forsen", Chat::Msg(own(Twitch, "gg")));
  leg(&mut app, &id, Kick, "xqc", false);
  leg(&mut app, &id, Youtube, "@abc", true);
  // nothing is told before the timeout, and there is nothing to retry yet
  app.resend_tick(now_ms());
  assert_eq!(notices(&app, "never confirmed"), 0);
  assert!(!app.retry_send());
  // past it: one notice row, once
  app.resend_tick(now_ms() + LATER);
  app.resend_tick(now_ms() + LATER * 2);
  assert_eq!(notices(&app, "never confirmed"), 1);
  // alt-enter: only kick goes again (youtube's POST may be in chat), under the same id
  app.mods = ModifiersState::ALT;
  assert!(app.global_key(Key::Named(NamedKey::Enter)));
  assert_eq!(out(&mut rx), vec![(Kick, "gg".to_string(), id)]);
  // its echo comes: nothing is left to retry
  app.on_relay(Kick, "xqc", vec![own(Kick, "gg")], false);
  app.on_relay(Youtube, "@abc", vec![own(Youtube, "gg")], false);
  assert!(!app.retry_send());
}

#[test]
fn a_leg_whose_post_may_have_landed_is_never_posted_again() {
  let (mut app, mut rx) = boot();
  type_and_send(&mut app, "hello");
  let id = out(&mut rx)[0].2.clone();
  app.on_chat("forsen", Chat::Msg(own(Twitch, "hello")));
  app.on_relay(Youtube, "@abc", vec![own(Youtube, "hello")], false);
  // kick took the POST and never echoed it
  leg(&mut app, &id, Kick, "xqc", true);
  app.resend_tick(now_ms() + LATER);
  assert_eq!(notices(&app, "may be in chat"), 1);
  assert!(app.retry_send());
  assert!(out(&mut rx).is_empty(), "nothing is posted twice");
}

#[test]
fn a_twitch_leg_that_never_echoed_goes_again() {
  let (mut app, mut rx) = boot();
  type_and_send(&mut app, "hi");
  let id = out(&mut rx)[0].2.clone();
  app.on_relay(Kick, "xqc", vec![own(Kick, "hi")], false);
  leg(&mut app, &id, Youtube, "@abc", false);
  app.resend_tick(now_ms() + LATER);
  assert!(app.retry_send());
  let again = out(&mut rx);
  assert_eq!(again.iter().map(|l| l.0).collect::<Vec<_>>(), vec![Twitch, Youtube]);
  assert!(again.iter().all(|l| l.1 == "hi" && l.2 == id));
}
