// a whisper or dm that lands while you are on another tab: a toast line with a per-sender count, and with
// the switch on a [dm] row in the chat you are on

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
  assert!(a.set.whisper_toast && !a.set.inline_dm, "the ext's defaults");
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
fn a_blocked_sender_gets_no_toast_and_no_row() {
  let mut a = app();
  a.set.inline_dm = true;
  crate::model::filter::edit(|f| f.block("Bob"));
  let before = lines(&a).len();
  a.on_whispers(vec![dm(1, "Bob", "hi")], true);
  assert_eq!(lines(&a).len(), before);
}

#[test]
fn with_the_inline_switch_a_dm_row_lands_in_the_chat_you_are_on() {
  let mut a = app();
  a.set.inline_dm = true;
  a.set.whisper_toast = false;
  let before = lines(&a).len();
  a.on_whispers(vec![dm(1, "Bob", "psst")], true);
  assert_eq!(lines(&a).len(), before + 1);
  let m = a.tabs[a.active].view.msgs().next_back().unwrap();
  assert_eq!((m.kind, &*m.body, &*m.nick), (MsgKind::Chat, "psst", "Bob"));
  let x = m.x.as_ref().unwrap();
  assert_eq!(x.edge, theme::YELLOW);
  assert_eq!((&*x.head[0].text, x.head[0].ink, x.head[0].bold), ("[dm] ", theme::YELLOW, true));
  assert!(m.login.is_empty() && m.user_id.is_empty(), "a heatsync dm names no twitch login");
}

#[test]
fn the_row_is_drawn_with_its_tag_and_edge_in_yellow() {
  let mut a = app();
  a.dpi = 1.0;
  a.res.set_scale(a.scale());
  a.resized(700, 400);
  a.set.inline_dm = true;
  a.set.whisper_toast = false;
  a.redraw();
  let before = a.canvas.inked(theme::YELLOW);
  a.on_whispers(vec![dm(1, "Bob", "psst")], true);
  a.redraw();
  assert!(a.canvas.inked(theme::YELLOW) > before + 10, "the tag and the edge");
}

#[test]
fn both_switches_are_in_the_settings_table_with_the_exts_keys() {
  use crate::model::schema::{DEFS, Wire};
  let d = |id: &str| DEFS.iter().find(|d| d.id == id).unwrap();
  assert_eq!(d("whisper_toast").wire, Wire::Bool("whisperToast"));
  assert_eq!(d("whisper_toast").cmd, Some("/whispertoast"));
  assert_eq!(d("inline_dm").wire, Wire::Flag("inlineNotifs", "dm"));
}
