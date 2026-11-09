// upload your own emote from the picker (alt-u): the form on the picker's top row, one upload, the grid showing it
use std::path::PathBuf;

use super::*;
use crate::model::emote_up::Stage;
use crate::model::picker::{Focus, Tab};
use crate::net::Cmd;
use crate::net::emote_ops::EmoteCmd;
use crate::net::upload::Upload;
use crate::net::upload::emote::{Done, Failed, Kept};
use winit::keyboard::{Key, ModifiersState, NamedKey};

type Asked = tokio::sync::mpsc::UnboundedReceiver<Cmd>;

fn boot(logged_in: bool) -> (App, Asked) {
  let (net, asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  if logged_in {
    app.me = Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(1),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
  }
  app.toggle_picker();
  (app, asked)
}

fn sent(asked: &mut Asked) -> Vec<Cmd> {
  std::iter::from_fn(|| asked.try_recv().ok()).collect()
}

fn emote_cmds(cmds: Vec<Cmd>) -> Vec<EmoteCmd> {
  cmds.into_iter().filter_map(|c| if let Cmd::Emote(e) = c { Some(e) } else { None }).collect()
}

fn alt_u(app: &mut App) {
  app.mods = ModifiersState::ALT;
  app.picker_key(Key::Character("u"), Some("u"));
  app.mods = ModifiersState::empty();
}

fn type_str(app: &mut App, s: &'static str) {
  for ch in s.char_indices() {
    let c = &s[ch.0..ch.0 + ch.1.len_utf8()];
    let k = if c == " " { Key::Named(NamedKey::Space) } else { Key::Character(c) };
    app.picker_key(k, Some(c));
  }
}

fn enter(app: &mut App) {
  app.picker_key(Key::Named(NamedKey::Enter), None);
}

fn stage(app: &App) -> Option<Stage> {
  app.picker.as_ref()?.up.as_ref().map(|u| u.stage)
}

fn text(app: &App) -> String {
  app.picker.as_ref().and_then(|p| p.up.as_ref()).map(|u| u.text.clone()).unwrap_or_default()
}

fn kept() -> Kept {
  Kept { url: "https://heatsync.org/uploads/pog.webp".into(), phash: Some("ab".into()), w: 128, h: 96, flagged: vec![] }
}

fn pick_file(app: &mut App, path: &str) {
  app.on_upload(Upload::EmotePicked(PathBuf::from(path)));
}

#[test]
fn logged_out_it_says_why_and_asks_nothing() {
  let (mut app, mut asked) = boot(false);
  alt_u(&mut app);
  assert_eq!(stage(&app), None);
  assert!(app.picker.as_ref().unwrap().note.contains("log in"));
  assert!(emote_cmds(sent(&mut asked)).is_empty());
  assert!(!app.uploading);
}

#[test]
fn alt_u_opens_the_file_dialog_once_and_swallows_keys_meanwhile() {
  let (mut app, mut asked) = boot(true);
  alt_u(&mut app);
  alt_u(&mut app);
  assert_eq!(stage(&app), Some(Stage::Choosing));
  assert!(app.uploading);
  assert!(matches!(emote_cmds(sent(&mut asked)).as_slice(), [EmoteCmd::PickFile]), "one dialog, not two");
  type_str(&mut app, "zz");
  assert!(app.picker.as_ref().unwrap().query.is_empty(), "typing waits for the dialog");
  assert_eq!(text(&app), "");
}

#[test]
fn a_picked_file_is_named_then_uploaded() {
  let (mut app, mut asked) = boot(true);
  alt_u(&mut app);
  sent(&mut asked);
  pick_file(&mut app, "/pics/Pog Champ-2x.png");
  assert_eq!((stage(&app), text(&app).as_str()), (Some(Stage::Name), "PogChamp"));
  assert!(!app.uploading, "choosing a name holds nothing up");
  // a space is not a name: nothing goes out, and it says so
  type_str(&mut app, " now");
  enter(&mut app);
  assert!(sent(&mut asked).is_empty());
  assert!(app.picker.as_ref().unwrap().note.contains("one word"));
  assert_eq!(stage(&app), Some(Stage::Name));
  app.mods = ModifiersState::CONTROL;
  app.picker_key(Key::Character("u"), Some("u"));
  app.mods = ModifiersState::empty();
  type_str(&mut app, "pog");
  enter(&mut app);
  match emote_cmds(sent(&mut asked)).as_slice() {
    [EmoteCmd::Upload { path, name }] => {
      assert_eq!((path, name.as_str()), (&PathBuf::from("/pics/Pog Champ-2x.png"), "pog"))
    }
    other => panic!("not one upload: {}", other.len()),
  }
  assert_eq!(stage(&app), Some(Stage::Busy));
  assert!(app.uploading);
  // while it is out, keys do nothing and esc waits
  type_str(&mut app, "x");
  app.picker_key(Key::Named(NamedKey::Escape), None);
  assert_eq!(stage(&app), Some(Stage::Busy));
  assert!(app.picker.is_some());
}

#[test]
fn a_finished_upload_is_in_the_grid_and_picked() {
  let (mut app, _asked) = boot(true);
  alt_u(&mut app);
  pick_file(&mut app, "/pics/pog.png");
  enter(&mut app);
  app.on_upload(Upload::EmoteDone(Done { name: "pog".into(), kept: kept() }));
  let p = app.picker.as_ref().unwrap();
  assert!(p.up.is_none() && p.focus == Focus::Grid && p.tab == Tab::Emotes);
  let it = p.selected().expect("the new emote is picked");
  assert_eq!((&*it.name, &*it.url), ("pog", "https://heatsync.org/uploads/pog.webp"));
  assert_eq!((it.w, it.h), (128, 96));
  assert!(p.note.contains("added pog"));
  assert!(!app.uploading);
  // the same name again replaces its tile, never doubles it
  app.picker.as_mut().unwrap().clear_query();
  let before = app.picker.as_ref().unwrap().len();
  app.on_upload(Upload::EmoteDone(Done { name: "pog".into(), kept: kept() }));
  app.picker.as_mut().unwrap().clear_query();
  assert_eq!(app.picker.as_ref().unwrap().len(), before);
}

#[test]
fn a_taken_name_asks_for_another_and_reuses_the_upload() {
  let (mut app, mut asked) = boot(true);
  alt_u(&mut app);
  pick_file(&mut app, "/pics/pog.png");
  enter(&mut app);
  sent(&mut asked);
  let why = "you already have an emote with that name, pick another".to_string();
  app.on_upload(Upload::EmoteFailed(Failed { why, kept: Some(kept()) }));
  assert_eq!(stage(&app), Some(Stage::Name));
  assert_eq!(app.picker.as_ref().unwrap().up.as_ref().unwrap().prompt(), "pick another name: ");
  assert!(app.picker.as_ref().unwrap().note.contains("pick another"));
  assert!(!app.uploading);
  type_str(&mut app, "2");
  enter(&mut app);
  match emote_cmds(sent(&mut asked)).as_slice() {
    [EmoteCmd::AddKept { kept: k, name }] => assert_eq!((k, name.as_str()), (&kept(), "pog2")),
    _ => panic!("the upload is added again, not sent again"),
  }
}

#[test]
fn a_failure_closes_the_form_and_says_why() {
  let (mut app, _asked) = boot(true);
  alt_u(&mut app);
  pick_file(&mut app, "/pics/pog.png");
  enter(&mut app);
  app.on_upload(Upload::EmoteFailed(Failed { why: "too big (10MB max)".into(), kept: None }));
  assert_eq!(stage(&app), None);
  assert!(app.picker.as_ref().unwrap().note.contains("too big"));
  assert!(!app.uploading);
  // and it can be started again
  alt_u(&mut app);
  assert_eq!(stage(&app), Some(Stage::Choosing));
}

#[test]
fn no_file_dialog_means_typing_the_path() {
  let (mut app, mut asked) = boot(true);
  alt_u(&mut app);
  sent(&mut asked);
  app.on_upload(Upload::NoChooser);
  assert_eq!(stage(&app), Some(Stage::Path));
  assert!(app.prompt.is_none(), "the chat's own path prompt is not opened");
  type_str(&mut app, "/tmp/cat.png");
  enter(&mut app);
  assert_eq!((stage(&app), text(&app).as_str()), (Some(Stage::Name), "cat"));
}

#[test]
fn a_closed_dialog_and_esc_leave_the_picker_as_it_was() {
  let (mut app, _asked) = boot(true);
  alt_u(&mut app);
  app.on_upload(Upload::Cancelled);
  assert_eq!(stage(&app), None);
  assert!(!app.uploading && app.prompt.is_none());
  alt_u(&mut app);
  pick_file(&mut app, "/pics/pog.png");
  app.picker_key(Key::Named(NamedKey::Escape), None);
  assert_eq!(stage(&app), None);
  assert!(app.picker.is_some(), "esc backs out of the form, not the picker");
  assert!(!app.uploading);
  app.picker_key(Key::Named(NamedKey::Escape), None);
  assert!(app.picker.is_none(), "the next esc closes it");
}

#[test]
fn an_answer_after_the_picker_closed_is_told_in_the_chat() {
  let (mut app, _asked) = boot(true);
  alt_u(&mut app);
  pick_file(&mut app, "/pics/pog.png");
  enter(&mut app);
  app.close_picker();
  app.on_upload(Upload::EmoteDone(Done { name: "pog".into(), kept: kept() }));
  let told = app.tabs[app.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default();
  assert!(told.contains("added pog"), "{told}");
  assert!(!app.uploading);
}

#[test]
fn a_file_picked_after_esc_is_dropped() {
  let (mut app, _asked) = boot(true);
  alt_u(&mut app);
  app.close_picker();
  pick_file(&mut app, "/pics/pog.png");
  assert!(!app.uploading && app.picker.is_none());
}
