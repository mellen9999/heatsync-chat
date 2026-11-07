// a whisper or dm that lands while you are on another tab: a toast line with a per-sender count

use super::*;
use crate::model::message::Kind as MsgKind;
use crate::model::whisper::{self, Whisper};
use crate::ui::theme;
use serde_json::json;

fn app() -> App {
  let (net, _rx) = Net::tap();
  App::new(net, vec!["forsen".into()], false)
}

fn dm(id: u32, from: &str, text: &str) -> Whisper {
  let v = json!({"id": id, "from_user_id": 100 + id, "from_display_name": from, "content": text});
  whisper::parse_dm(&v, None).unwrap()
}

fn lines(a: &App) -> Vec<String> {
  a.tabs[a.active].view.msgs().map(|m| m.body.to_string()).collect()
}

fn to_whispers(a: &mut App) {
  let i = a.tab_of(Kind::Whispers).unwrap();
  a.select(i);
}

#[test]
fn a_whisper_on_another_tab_toasts_its_name_and_first_80_chars() {
  let mut a = app();
  assert!(a.set.whisper_toast, "the ext's default");
  let before = lines(&a).len();
  a.on_whispers(vec![dm(1, "Bob", &"x".repeat(200))], true);
  let got = lines(&a);
  assert_eq!(got.len(), before + 1);
  assert_eq!(got.last().unwrap(), &format!("Bob whispered: {}", "x".repeat(80)));
  let m = a.tabs[a.active].view.msgs().next_back().unwrap();
  assert_eq!((m.kind, m.color), (MsgKind::System, theme::CYAN));
}

#[test]
fn a_burst_from_one_sender_counts_up_in_place_and_others_get_their_own() {
  let mut a = app();
  let before = lines(&a).len();
  a.on_whispers(vec![dm(1, "Bob", "one")], true);
  a.on_whispers(vec![dm(2, "Bob", "two")], true);
  a.on_whispers(vec![dm(3, "Bob", "three")], true);
  let got = lines(&a);
  assert_eq!(got.len(), before + 1, "one toast");
  assert_eq!(got.last().unwrap(), "Bob whispered: three ×3");
}

#[test]
fn another_sender_or_a_line_after_the_toast_starts_a_new_line() {
  let mut a = app();
  let before = lines(&a).len();
  a.on_whispers(vec![dm(1, "Bob", "one")], true);
  a.on_whispers(vec![dm(2, "Amy", "hey")], true);
  assert_eq!(lines(&a)[before..], ["Bob whispered: one", "Amy whispered: hey"]);
  // chat moved on under Bob's toast: his next whisper is a new line, still counting
  a.on_whispers(vec![dm(3, "Bob", "again")], true);
  assert_eq!(lines(&a).last().unwrap(), "Bob whispered: again ×2");
  assert_eq!(lines(&a).len(), before + 3);
}

#[test]
fn nothing_shows_on_the_whispers_tab_for_history_or_with_the_switch_off() {
  let mut a = app();
  let before = lines(&a).len();
  a.on_whispers(vec![dm(1, "Bob", "old")], false);
  assert_eq!(lines(&a).len(), before, "history is not news");
  a.set.whisper_toast = false;
  a.on_whispers(vec![dm(2, "Bob", "quiet")], true);
  assert_eq!(lines(&a).len(), before, "switch off");
  a.set.whisper_toast = true;
  to_whispers(&mut a);
  let mine = lines(&a).len();
  a.on_whispers(vec![dm(3, "Bob", "here")], true);
  assert_eq!(lines(&a).len(), mine + 1, "the row itself, no toast");
  assert!(!lines(&a).iter().any(|l| l.contains("whispered:")));
}

#[test]
fn a_blocked_sender_gets_no_toast() {
  let mut a = app();
  crate::model::filter::edit(|f| f.block("Bob"));
  let before = lines(&a).len();
  a.on_whispers(vec![dm(1, "Bob", "hi")], true);
  assert_eq!(lines(&a).len(), before);
}

#[test]
fn the_switch_is_in_the_settings_table_with_the_exts_key() {
  use crate::model::schema::{DEFS, Wire};
  let d = |id: &str| DEFS.iter().find(|d| d.id == id).unwrap();
  assert_eq!(d("whisper_toast").wire, Wire::Bool("whisperToast"));
  assert_eq!(d("whisper_toast").cmd, Some("/whispertoast"));
}

fn every_line(a: &App) -> String {
  let mut all = String::new();
  for t in &a.tabs {
    for m in t.view.msgs() {
      all.push_str(&m.body);
      all.push('\n');
    }
  }
  all
}

#[test]
fn streamer_mode_hides_the_toast_words_in_every_tab_and_off_brings_them_back() {
  use crate::model::schema::Fx;
  use crate::model::settings::Streamer as Mode;
  let mut a = app();
  a.on_whispers(vec![dm(1, "Bob", "secretwords")], true);
  // the first toast stays behind in its tab while you move on and get another in a second one
  let first = a.active;
  let other = (0..a.tabs.len()).find(|&i| i != first && a.tabs[i].kind != Kind::Whispers).unwrap();
  a.select(other);
  a.on_whispers(vec![dm(2, "Amy", "moresecret")], true);
  assert!(every_line(&a).contains("secretwords") && every_line(&a).contains("moresecret"));
  a.set.streamer = Mode::On;
  a.applied(Fx::Streamer);
  let hidden = every_line(&a);
  assert!(!hidden.contains("secretwords") && !hidden.contains("moresecret"), "{hidden}");
  assert!(hidden.contains("whisper from Bob") && hidden.contains("whisper from Amy"));
  a.set.streamer = Mode::Off;
  a.applied(Fx::Streamer);
  assert!(every_line(&a).contains("Bob whispered: secretwords"));
}
