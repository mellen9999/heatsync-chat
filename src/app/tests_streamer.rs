// streamer mode: auto follows obs (a fake /proc), and every surface that could show something private
// holds it back while it is on

use super::*;
use crate::model::message::{Kind as MsgKind, Message, Platform, flag};
use crate::model::schema::Fx;
use crate::model::settings::Streamer as Mode;
use crate::model::streamer::{HIDDEN, HIDDEN_SHORT, fake_proc};
use crate::model::whisper::{self, Whisper};
use crate::ui::theme;
use serde_json::json;
use std::time::Duration;

fn app() -> App {
  let (net, _rx) = Net::tap();
  App::new(net, vec!["forsen".into()], false)
}

fn dm(id: u32, from: &str, text: &str) -> Whisper {
  let v = json!({"id": id, "from_user_id": 100 + id, "from_display_name": from, "content": text});
  whisper::parse_dm(&v, None).unwrap()
}

fn set(a: &mut App, m: Mode) {
  a.set.streamer = m;
  a.applied(Fx::Streamer);
}

fn bodies(a: &App) -> Vec<String> {
  a.tabs[a.active].view.msgs().map(|m| m.body.to_string()).collect()
}

fn whisper_rows(a: &App) -> Vec<String> {
  let t = a.tabs.iter().find(|t| t.kind == Kind::Whispers).unwrap();
  t.view.msgs().map(|m| m.body.to_string()).collect()
}

#[test]
fn off_by_default_and_on_means_on() {
  let mut a = app();
  assert!(!a.streamer_hot() && a.streamer_mark().is_none() && a.streamer.due.is_none());
  set(&mut a, Mode::On);
  assert!(a.streamer_hot() && a.streamer_mark().is_some());
  assert!(a.streamer.due.is_none(), "no timer unless auto");
  set(&mut a, Mode::Off);
  assert!(!a.streamer_hot() && a.streamer_mark().is_none());
}

#[test]
fn auto_follows_obs_and_only_ticks_while_auto() {
  let mut a = app();
  a.streamer.root = fake_proc("auto-a", &[("1", "systemd")]);
  set(&mut a, Mode::Auto);
  assert!(!a.streamer_hot(), "no obs yet");
  let due = a.streamer.due.expect("auto looks again soon");
  // obs starts: the next look turns it on
  std::fs::create_dir_all(a.streamer.root.join("90")).unwrap();
  std::fs::write(a.streamer.root.join("90/comm"), "obs\n").unwrap();
  a.streamer_tick(due - Duration::from_secs(1));
  assert!(!a.streamer_hot(), "not due yet: no look");
  a.streamer_tick(due);
  assert!(a.streamer_hot());
  // obs quits
  std::fs::remove_dir_all(a.streamer.root.join("90")).unwrap();
  a.streamer_tick(a.streamer.due.unwrap());
  assert!(!a.streamer_hot());
  // leaving auto stops the clock and forgets obs
  set(&mut a, Mode::Off);
  assert!(a.streamer.due.is_none() && !a.streamer.obs);
  let _ = std::fs::remove_dir_all(&a.streamer.root);
}

#[test]
fn whispers_tab_rows_hide_their_words_until_shown() {
  let mut a = app();
  a.on_whispers(vec![dm(1, "Bob", "secret words")], false);
  assert_eq!(whisper_rows(&a), ["secret words"]);
  set(&mut a, Mode::On);
  assert_eq!(whisper_rows(&a), [HIDDEN], "rebuilt the moment it is on");
  // a new live one is hidden too
  a.on_whispers(vec![dm(2, "Amy", "more secrets")], true);
  assert_eq!(whisper_rows(&a).len(), 2);
  assert!(whisper_rows(&a).iter().all(|b| b == HIDDEN));
  // space s shows them, again hides
  a.streamer_reveal();
  assert_eq!(whisper_rows(&a), ["secret words", "more secrets"]);
  a.streamer_reveal();
  assert!(whisper_rows(&a).iter().all(|b| b == HIDDEN));
  // shown, then off and on again: hidden again
  a.streamer_reveal();
  set(&mut a, Mode::Off);
  assert_eq!(whisper_rows(&a), ["secret words", "more secrets"]);
  set(&mut a, Mode::On);
  assert!(whisper_rows(&a).iter().all(|b| b == HIDDEN));
}

#[test]
fn reveal_does_nothing_when_the_mode_is_off() {
  let mut a = app();
  a.on_whispers(vec![dm(1, "Bob", "hi")], false);
  let before = bodies(&a);
  a.streamer_reveal();
  assert_eq!(bodies(&a), before, "no status line, nothing to reveal");
  assert!(!a.streamer.shown);
}

#[test]
fn a_failed_send_keeps_its_retry_hint_while_hidden() {
  let mut w = dm(1, "Bob", "private");
  w.status = whisper::Status::Failed(whisper::Fail::Relink);
  let m = whisper::message(&w, true);
  assert!(m.body.starts_with(HIDDEN) && m.body.contains("enter to re-link") && !m.body.contains("private"));
  assert!(m.has(flag::ALERT));
}

#[test]
fn the_toast_names_the_sender_and_never_the_words() {
  let mut a = app();
  set(&mut a, Mode::On);
  let before = bodies(&a).len();
  a.on_whispers(vec![dm(1, "Bob", "my address is")], true);
  a.on_whispers(vec![dm(2, "Bob", "something else")], true);
  let got = bodies(&a);
  assert_eq!(got.len(), before + 1);
  assert_eq!(got.last().unwrap(), "whisper from Bob ×2");
  let m = a.tabs[a.active].view.msgs().next_back().unwrap();
  assert_eq!((m.kind, m.color), (MsgKind::System, theme::CYAN));
}

#[test]
fn the_inline_dm_row_hides_its_words() {
  let mut a = app();
  a.set.inline_dm = true;
  a.set.whisper_toast = false;
  set(&mut a, Mode::On);
  a.on_whispers(vec![dm(1, "Bob", "psst")], true);
  let m = a.tabs[a.active].view.msgs().next_back().unwrap();
  assert_eq!((&*m.body, &*m.nick), (HIDDEN_SHORT, "Bob"));
}

#[test]
fn link_previews_and_inline_pictures_go_quiet() {
  let mut a = app();
  assert!(a.res.media);
  a.res.previews.get("https://a.example/x");
  assert_eq!(a.res.previews.take_wanted().len(), 1, "asked while off");
  set(&mut a, Mode::On);
  assert!(!a.res.media && a.res.previews.hush);
  assert!(a.res.previews.get("https://a.example/y").is_none());
  assert!(a.res.previews.take_wanted().is_empty(), "nothing is even asked while on");
  set(&mut a, Mode::Off);
  assert!(a.res.media && !a.res.previews.hush);
  // the media switch still rules when the mode is off
  a.set.media = false;
  a.applied(Fx::Media);
  assert!(!a.res.media);
}

#[test]
fn popups_lose_their_text_and_sounds_are_muted() {
  let mut a = app();
  assert_eq!(a.popup_body("hello @me"), "hello @me");
  assert!(!a.sound_muted());
  set(&mut a, Mode::On);
  assert_eq!(a.popup_body("hello @me"), "", "the title says who and where, not what");
  assert!(a.sound_muted());
  a.set.streamer_sounds = true;
  assert!(!a.sound_muted(), "keep sounds");
}

#[test]
fn a_rule_sound_is_silent_in_streamer_mode() {
  use crate::util::sound::Preset;
  let rules = json!([{ "id": "a", "enabled": true, "scope": "all", "sound": "knock", "action": "highlight",
    "match": { "type": "keyword", "value": "gg" } }]);
  let mut a = app();
  assert!(crate::model::filter::edit(|f| f.set_rules_text(&rules.to_string())));
  let mut m = Message::system(crate::util::time::now_ms(), Platform::Twitch, "");
  (m.kind, m.login, m.nick, m.body) = (MsgKind::Chat, "bob".into(), "bob".into(), "gg".into());
  assert_eq!(a.line_sound("forsen", &m), Some(Preset::Knock));
  set(&mut a, Mode::On);
  assert_eq!(a.line_sound("forsen", &m), None);
  a.set.streamer_sounds = true;
  assert_eq!(a.line_sound("forsen", &m), Some(Preset::Knock));
}

#[test]
fn the_marker_shows_in_the_bar() {
  let mut a = app();
  a.bar_dirty = false;
  set(&mut a, Mode::On);
  assert_eq!(a.streamer_mark(), Some("● streamer"));
  assert!(a.bar_dirty, "the bar redraws to show it");
}
