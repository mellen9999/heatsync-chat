// your own slash commands, the whole app headless: they expand once, built-ins win a name, nothing loops,
// a missing word sends nothing, and the expansion still goes through the normal send
use super::lists::Kind;
use super::screen::{Page, Target};
use super::*;
use crate::net::Cmd;
use crate::net::auth::Me;
use winit::keyboard::{Key, NamedKey};

fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  let me = Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  };
  app.on_net(NetEvent::Me(Some(me)));
  app.redraw();
  while rx.try_recv().is_ok() {}
  (app, rx)
}

fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<String> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Send { text, .. } => Some(text),
      _ => None,
    })
    .collect()
}

fn say(app: &mut App, text: &str) {
  app.line.set_text(text.into());
  app.send();
}

fn status(app: &App) -> String {
  app.tabs[app.active].view.msgs().next_back().map(|m| m.body.to_string()).unwrap_or_default()
}

fn with(app: &mut App, entries: &[&str]) {
  for e in entries {
    app.ucmd_add(e).unwrap();
  }
}

#[test]
fn a_command_of_yours_sends_its_expansion() {
  let (mut app, mut rx) = boot();
  with(&mut app, &["gg good game {1}, from {user} in {channel}", "all {1+}"]);
  say(&mut app, "/gg bob");
  say(&mut app, "/ALL one  two three");
  assert_eq!(sent(&mut rx), ["good game bob, from mellen in forsen", "one  two three"]);
  assert!(app.line.text.is_empty());
}

#[test]
fn a_missing_word_sends_nothing_and_keeps_what_you_typed() {
  let (mut app, mut rx) = boot();
  with(&mut app, &["gg good game {1}"]);
  say(&mut app, "/gg");
  assert!(sent(&mut rx).is_empty());
  assert_eq!(app.line.text, "/gg");
  assert_eq!(status(&app), "/gg needs 1 word after it");
}

#[test]
fn a_built_in_always_wins_a_name_and_adding_says_so() {
  let (mut app, mut rx) = boot();
  let said = app.ucmd_add("ban nope {1}").unwrap();
  assert!(said.contains("built-in") && said.contains("/ban"), "{said}");
  // an alias that isn't in the / list is a built-in too
  assert!(app.ucmd_add("sl nope").unwrap().contains("built-in"));
  assert!(app.ucmd_add("fine ok").unwrap().starts_with("added"));
  say(&mut app, "/ban bob");
  assert!(!sent(&mut rx).iter().any(|t| t.contains("nope")));
  // /me is listed, so a command of yours called me never runs either
  app.ucmd_add("me hijack").unwrap();
  say(&mut app, "/me waves");
  assert_eq!(sent(&mut rx), ["/me waves"]);
}

#[test]
fn it_cannot_loop() {
  let (mut app, mut rx) = boot();
  with(&mut app, &["aa /bb {input}", "bb /aa {input}", "self /self {input}", "say {input}", "ok /shrug {input}"]);
  for line in ["/aa x", "/bb x", "/self x", "/say /say hi", "/say /aa x"] {
    say(&mut app, line);
    assert!(status(&app).contains("can't run"), "{line}: {}", status(&app));
  }
  assert!(sent(&mut rx).is_empty());
  // one built-in hop is fine, and it is the built-in's own path
  say(&mut app, "/ok hi");
  assert_eq!(sent(&mut rx), ["hi ¯\\_(ツ)_/¯"]);
  // typed words are data, never placeholders
  say(&mut app, "/say {user}");
  assert_eq!(sent(&mut rx), ["{user}"]);
}

#[test]
fn the_send_paths_limits_still_apply() {
  let (mut app, mut rx) = boot();
  with(&mut app, &["say {input}"]);
  say(&mut app, &format!("/say {}", "a".repeat(501)));
  assert!(sent(&mut rx).is_empty());
  assert!(status(&app).contains("over 500"), "{}", status(&app));
  // no network part: the line stays, as the normal send keeps it
  let (net, rx2) = Net::tap();
  drop(rx2);
  app.net = net;
  say(&mut app, "/say hi");
  assert_eq!(app.line.text, "/say hi");
}

#[test]
fn suggestions_list_yours_first_with_the_expansion() {
  let (mut app, _rx) = boot();
  with(&mut app, &["gg good game {1}", "ban nope"]);
  app.set_mode(crate::model::vi::Mode::Insert);
  app.line.set_text("/g".into());
  app.line.cursor = 2;
  app.sync_suggest();
  let names =
    |a: &App| -> Vec<String> { a.sug.list.as_ref().unwrap().items.iter().map(|p| p.name.to_string()).collect() };
  assert_eq!(names(&app).first().map(String::as_str), Some("/gg"));
  app.line.set_text("/ba".into());
  app.line.cursor = 3;
  app.sync_suggest();
  let n = names(&app);
  assert_eq!(n.iter().filter(|n| *n == "/ban").count(), 1, "{n:?}");
}

fn press(app: &mut App, k: &'static str) {
  app.on_key(Key::Character(k), Some(k));
}

fn type_in(app: &mut App, s: &str) {
  for c in s.chars() {
    let t = c.to_string();
    app.on_key(Key::Character(&t), Some(&t));
  }
}

fn rows(app: &App) -> Vec<String> {
  let s = app.screen.as_ref().unwrap();
  let (lines, _) = s.lines(&app.set);
  lines
    .into_iter()
    .filter_map(|l| if let crate::ui::sheet::L::Row { a, b, .. } = l { Some(format!("{a}|{b}")) } else { None })
    .collect()
}

#[test]
fn the_settings_row_opens_a_list_where_a_adds_and_x_removes() {
  let (mut app, mut rx) = boot();
  app.open_screen(Page::Settings);
  let s = app.screen.as_mut().unwrap();
  let (lines, targets) = s.lines(&app.set);
  let at = targets.iter().position(|t| matches!(t, Target::Def(d) if d.id == "my_commands")).unwrap();
  s.st.at = at;
  s.st.settle(&lines);
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert_eq!(app.screen.as_ref().unwrap().page(), Page::List);
  press(&mut app, "a");
  // a bad name is refused and stays in the box to fix
  type_in(&mut app, "bad! x");
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert!(app.ucmds.is_empty() && app.screen.as_ref().unwrap().typing());
  for _ in 0..6 {
    app.on_key(Key::Named(NamedKey::Backspace), None);
  }
  type_in(&mut app, "gg good game {1}");
  app.on_key(Key::Named(NamedKey::Enter), None);
  app.on_key(Key::Named(NamedKey::Escape), None);
  assert_eq!(rows(&app)[0], "/gg|good game {1}");
  app.close_screen();
  say(&mut app, "/gg all");
  assert_eq!(sent(&mut rx), ["good game all"]);
  // it is on disk, so a restart has it
  assert_eq!(crate::model::usercmd::Cmds::load().get("gg"), Some("good game {1}"));
  app.list_open(Kind::Commands);
  press(&mut app, "x");
  assert!(app.ucmds.is_empty());
  assert!(crate::model::usercmd::Cmds::load().is_empty());
}
