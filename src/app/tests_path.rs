// tab-completion in the upload prompt: the folder is read off the ui thread, and an answer that no
// longer matches what is typed is dropped

use super::*;
use crate::model::line::Line;
use crate::net::Cmd;
use crate::net::upload::Upload;
use winit::keyboard::{Key, NamedKey};

fn boot(typed: &str) -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  while rx.try_recv().is_ok() {}
  let mut l = Line::default();
  l.insert(typed);
  app.prompt = Some(l);
  (app, rx)
}

fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(u64, String)> {
  let mut v = Vec::new();
  while let Ok(c) = rx.try_recv() {
    if let Cmd::Complete { seq, input, .. } = c {
      v.push((seq, input));
    }
  }
  v
}

fn typed(app: &App) -> &str {
  &app.prompt.as_ref().unwrap().text
}

fn answer(seq: u64, input: &str, text: &str) -> Upload {
  Upload::Completed { seq, input: input.into(), text: text.into(), many: Vec::new() }
}

#[test]
fn tab_asks_the_net_thread_instead_of_reading_the_folder_itself() {
  let (mut app, mut rx) = boot("/mnt/slow/ph");
  app.prompt_key(Key::Named(NamedKey::Tab), None);
  assert_eq!(asked(&mut rx), [(1, "/mnt/slow/ph".to_string())]);
  assert_eq!(typed(&app), "/mnt/slow/ph", "nothing changes until the answer is back");
  app.on_upload(answer(1, "/mnt/slow/ph", "/mnt/slow/photo.png"));
  assert_eq!(typed(&app), "/mnt/slow/photo.png");
}

#[test]
fn an_answer_for_old_text_or_an_old_press_is_dropped() {
  let (mut app, mut rx) = boot("/mnt/slow/ph");
  app.prompt_key(Key::Named(NamedKey::Tab), None);
  assert_eq!(asked(&mut rx).len(), 1);
  // typed on while the folder was slow: the answer is for words that are gone
  app.prompt.as_mut().unwrap().insert("o");
  app.on_upload(answer(1, "/mnt/slow/ph", "/mnt/slow/photo.png"));
  assert_eq!(typed(&app), "/mnt/slow/pho");
  // pressed twice with the same text: only the newest ask counts
  app.prompt_key(Key::Named(NamedKey::Tab), None);
  app.prompt_key(Key::Named(NamedKey::Tab), None);
  assert_eq!(asked(&mut rx).iter().map(|a| a.0).collect::<Vec<_>>(), [2, 3]);
  app.on_upload(answer(2, "/mnt/slow/pho", "/mnt/slow/old.png"));
  assert_eq!(typed(&app), "/mnt/slow/pho");
  app.on_upload(answer(3, "/mnt/slow/pho", "/mnt/slow/photo.png"));
  assert_eq!(typed(&app), "/mnt/slow/photo.png");
  // and with the prompt closed there is nothing to fill
  app.close_prompt();
  app.on_upload(answer(3, "/mnt/slow/photo.png", "x"));
  assert!(app.prompt.is_none());
}

#[test]
fn several_matches_are_listed_in_the_chat() {
  let (mut app, mut rx) = boot("/p/c");
  app.prompt_key(Key::Named(NamedKey::Tab), None);
  let _ = asked(&mut rx);
  let many = vec!["cat1.png".to_string(), "cat2.png".to_string()];
  app.on_upload(Upload::Completed { seq: 1, input: "/p/c".into(), text: "/p/cat".into(), many });
  assert_eq!(typed(&app), "/p/cat");
  let said = app.tabs[app.active].view.last_msg().map(|m| m.body.to_string());
  assert_eq!(said.as_deref(), Some("cat1.png  cat2.png"));
}
