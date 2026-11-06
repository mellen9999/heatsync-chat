// features that used to be slash-only, reached from the account screen and the tab menu
use super::screen::{Page, Target};
use super::*;
use crate::app::account::Step;
use crate::net::Cmd;
use winit::keyboard::{Key, NamedKey};

fn me(twitch: &str) -> crate::net::auth::Me {
  crate::net::auth::Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some(twitch.into()),
    twitch_id: Some(1),
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  }
}

fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  (app, rx)
}

fn tab_menu(app: &mut App) -> Vec<String> {
  let i = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.tabs[i].event = false;
  app.tabs[i].unread = false;
  app.close_tab_menu();
  app.bar_dirty = true;
  app.redraw();
  let p = app.bar_map.iter().position(|&t| t == i).expect("in the bar");
  let r = app.bar_hits[p];
  app.pointer = (r.x + r.w / 2, r.y + r.h / 2);
  app.right_click();
  app.tab_menu().map(|m| m.labels()).unwrap_or_default()
}

// open the menu, walk to the row by its label, enter
fn pick(app: &mut App, label: &str) {
  let labels = tab_menu(app);
  let at = labels.iter().position(|l| l == label).unwrap_or_else(|| panic!("{label} in {labels:?}"));
  for _ in 0..at {
    app.tab_menu_key(Key::Character("j"));
  }
  app.tab_menu_key(Key::Named(NamedKey::Enter));
}

#[test]
fn the_tab_menu_offers_layouts_and_clearing_to_everyone() {
  let (mut app, _rx) = boot();
  assert_eq!(tab_menu(&mut app), ["edit", "remove", "clear lines", "share layout", "use a shared layout…"]);
}

#[test]
fn channel_tools_in_the_tab_menu_follow_who_you_are_in_that_room() {
  let (mut app, _rx) = boot();
  // a stranger in the room: none
  app.me = Some(me("someoneelse"));
  let m = tab_menu(&mut app);
  assert!(!m.contains(&"announce…".to_owned()) && !m.contains(&"poll…".to_owned()), "{m:?}");
  // a moderator: announce only
  app.on_chat("forsen", crate::net::irc::Chat::Mod(true));
  let m = tab_menu(&mut app);
  assert!(m.contains(&"announce…".to_owned()) && !m.contains(&"poll…".to_owned()), "{m:?}");
  // the broadcaster: all three
  app.me = Some(me("forsen"));
  let m = tab_menu(&mut app);
  assert!(m.contains(&"announce…".to_owned()) && m.contains(&"poll…".to_owned()), "{m:?}");
  assert!(m.contains(&"prediction…".to_owned()));
  // picking one leaves the command typed, ready to finish
  pick(&mut app, "poll…");
  assert_eq!(app.line.text, "/poll ");
  app.line.set_text(String::new());
  app.line.home();
  pick(&mut app, "prediction…");
  assert_eq!(app.line.text, "/prediction ");
  app.line.set_text(String::new());
  app.line.home();
  pick(&mut app, "announce…");
  assert_eq!(app.line.text, "/announce ");
}

#[test]
fn use_a_shared_layout_types_the_command_and_share_copies_the_link() {
  let (mut app, mut rx) = boot();
  pick(&mut app, "use a shared layout…");
  assert_eq!(app.line.text, "/layout ");
  while rx.try_recv().is_ok() {}
  pick(&mut app, "share layout");
  let copied: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| if let Cmd::Copy { text, .. } = c { Some(text) } else { None })
    .collect();
  assert!(copied.iter().any(|t| t.contains("/m/")), "{copied:?}");
}

#[test]
fn clear_lines_empties_the_tab_it_was_picked_on() {
  let (mut app, _rx) = boot();
  app.say("hello");
  assert!(app.tabs[app.active].view.msgs().count() > 0);
  pick(&mut app, "clear lines");
  assert_eq!(app.tabs[app.active].view.msgs().filter(|m| &*m.body == "hello").count(), 0);
}

#[test]
fn the_account_screen_imports_follows_and_sets_a_color() {
  let (mut app, _rx) = boot();
  app.me = Some(me("mellen"));
  let go = |app: &mut App, step: Step| {
    app.account_open();
    let s = app.screen.as_mut().unwrap();
    let (lines, targets) = s.lines(&app.set);
    s.st.at = targets.iter().position(|t| matches!(t, Target::Account(x) if *x == step)).expect("a row");
    s.st.settle(&lines);
    app.on_key(Key::Named(NamedKey::Enter), None);
  };
  go(&mut app, Step::Import);
  assert!(app.screen_is(Page::Import));
  app.close_screen();
  go(&mut app, Step::Color);
  assert!(app.screen.is_none() && app.line.text == "/color ");
}
