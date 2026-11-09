// alt-[ alt-] step the tabs and alt-r is row mode: the pick cursor without vi, whose letters never land in the box
use super::*;
use crate::model::message::{Kind as MsgKind, Message};
use winit::keyboard::{Key, ModifiersState, NamedKey};

fn boot() -> App {
  let (net, _rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  app
}

fn alt(app: &mut App, k: &'static str) {
  app.mods = ModifiersState::ALT;
  app.on_key(Key::Character(k), Some(k));
  app.mods = ModifiersState::empty();
}

fn press(app: &mut App, k: &'static str) {
  app.on_key(Key::Character(k), Some(k));
}

#[test]
fn alt_brackets_step_the_tabs_and_wrap() {
  let mut app = boot();
  let first = app.active;
  alt(&mut app, "]");
  assert_ne!(app.active, first);
  alt(&mut app, "[");
  assert_eq!(app.active, first);
  // all the way round comes back
  let n = (0..app.tabs.len()).filter(|&i| app.visible(i)).count();
  for _ in 0..n {
    alt(&mut app, "]");
  }
  assert_eq!(app.active, first);
}

#[test]
fn alt_r_picks_a_line_and_its_keys_stay_out_of_the_box() {
  let mut app = boot();
  let f = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(f);
  for i in 0..3 {
    let mut m = Message::system(i + 1, Platform::Twitch, "");
    (m.kind, m.login, m.nick, m.body) = (MsgKind::Chat, "bob".into(), "bob".into(), "hello".into());
    app.tabs[f].view.push(m);
  }
  assert!(!app.set.vi);
  app.line.set_text("draft".to_owned());
  alt(&mut app, "r");
  assert!(app.rows, "row mode is on");
  assert!(app.selecting(), "a line is picked");
  // j k move, an unknown letter is swallowed, the draft is untouched
  press(&mut app, "k");
  press(&mut app, "z");
  assert_eq!(app.line.text, "draft");
  // esc lets go, and typing is the box's again
  app.on_key(Key::Named(NamedKey::Escape), None);
  assert!(!app.selecting());
  assert!(!app.rows);
  press(&mut app, "x");
  assert_eq!(app.line.text, "xdraft");
}

#[test]
fn row_mode_g_and_home_jump_to_the_first_line_and_shift_g_and_end_to_the_last() {
  let mut app = boot();
  let f = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(f);
  for i in 0..5 {
    let mut m = Message::system(i + 1, Platform::Twitch, "");
    (m.kind, m.login, m.nick, m.body) = (MsgKind::Chat, "bob".into(), "bob".into(), format!("line {i}").into());
    app.tabs[f].view.push(m);
  }
  app.line.set_text("draft".to_owned());
  alt(&mut app, "r");
  let picked = |app: &App| app.tabs[app.active].view.selected().map(|m| m.body.to_string());
  assert_eq!(picked(&app).as_deref(), Some("line 4"));
  press(&mut app, "g");
  assert_eq!(picked(&app).as_deref(), Some("line 0"));
  press(&mut app, "G");
  assert_eq!(picked(&app).as_deref(), Some("line 4"));
  app.on_key(Key::Named(NamedKey::Home), None);
  assert_eq!(picked(&app).as_deref(), Some("line 0"));
  app.on_key(Key::Named(NamedKey::End), None);
  assert_eq!(picked(&app).as_deref(), Some("line 4"));
  assert!(app.rows, "still in row mode");
  assert_eq!(app.line.text, "draft", "the box is untouched");
}
