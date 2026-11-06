// the store keeps its trouble in plain words; the app says it once, in the chat on screen

use super::*;

// the store keeps its trouble in plain words; the app puts them in the chat on screen, once
#[test]
fn a_set_aside_file_and_a_failed_save_reach_the_chat() {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  let _ = store::take_notes();
  let said = |app: &App| app.tabs[app.active].view.last_msg().map(|m| m.body.to_string());
  store::write("settings", "not a settings file\n").unwrap();
  assert_eq!(Settings::load(), Settings::default());
  app.say_store_notes();
  assert_eq!(
    said(&app).as_deref(),
    Some("your settings file was unreadable; saved a copy as settings.bak and started fresh")
  );
  // a save that can't land: a file sits where the folder goes
  std::fs::remove_dir_all(store::folder()).unwrap();
  std::fs::write(store::folder(), "in the way").unwrap();
  Settings::default().save();
  store::flush();
  app.say_store_notes();
  assert!(said(&app).is_some_and(|s| s.starts_with("couldn't save your settings")), "{:?}", said(&app));
  let n = app.tabs[app.active].view.len();
  app.say_store_notes();
  assert_eq!(app.tabs[app.active].view.len(), n, "said once");
}
