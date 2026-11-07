// the plain-words status lines: where chat logs go, the one permission line, the reply button nudge
use super::*;
use crate::model::schema::Fx;
use crate::model::settings::Streamer as Mode;

fn app() -> App {
  let (net, _rx) = Net::tap();
  App::new(net, vec!["forsen".into()], false)
}

fn said(a: &App) -> String {
  a.tabs[a.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
}

#[test]
fn turning_chat_logs_on_says_where_they_go_unless_streamer_mode_is_up() {
  let mut a = app();
  a.set.chat_logs = true;
  a.applied(Fx::ChatLogs);
  assert!(said(&a).starts_with("saving chat logs to "), "{}", said(&a));
  assert!(said(&a).ends_with("logs"), "the folder, nothing after it");
  // streamer mode: the path stays off the screen
  let mut b = app();
  b.set.streamer = Mode::On;
  b.applied(Fx::Streamer);
  let before = said(&b);
  b.set.chat_logs = true;
  b.applied(Fx::ChatLogs);
  assert_eq!(said(&b), before, "nothing said about the folder");
  // turning them off says nothing
  a.set.chat_logs = false;
  let before = said(&a);
  a.applied(Fx::ChatLogs);
  assert_eq!(said(&a), before);
}

#[cfg(unix)]
#[test]
fn the_home_folder_reads_as_a_tilde() {
  let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap();
  let inside = super::screen::logs_said(Some(home.join(".local/share/x/logs")));
  assert_eq!(inside, "saving chat logs to ~/.local/share/x/logs");
  assert_eq!(super::screen::logs_said(None), "saving chat logs on this computer");
}

#[test]
fn every_permission_line_ends_the_same_way() {
  assert_eq!(
    super::feed::needs_twitch("moderating"),
    "moderating needs a twitch permission: allow it in your browser, then try again"
  );
  assert!(super::feed::needs_twitch("clipping").ends_with(super::feed::ALLOW_IT));
}

// the hover button is off by default: the first reply of a run says it can be turned on, once
#[test]
fn the_first_reply_mentions_the_hover_button_once() {
  use crate::model::message::{Kind as MsgKind, Message, Platform};
  use crate::net::irc::Chat;
  let mut a = app();
  assert!(!a.set.hover_reply);
  let mut m = Message::system(crate::util::time::now_ms(), Platform::Twitch, "hey");
  m.kind = MsgKind::Chat;
  m.login = "bob".into();
  m.nick = "bob".into();
  m.id = "m1".into();
  a.on_chat("forsen", Chat::Msg(m));
  a.tabs[a.active].view.select_start(&mut a.res, 300);
  a.reply_selected();
  assert!(a.reply.is_some());
  assert!(said(&a).contains("/hoverreply"), "{}", said(&a));
  // not again
  a.say("later");
  a.reply_selected();
  assert_eq!(said(&a), "later");
}
