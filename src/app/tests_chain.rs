// reply threads, headless: t opens the chain over the tab, new replies land in it, it closes the way it
// opened, and copy thread says what it copied

use super::*;
use crate::model::message::{Kind, Message};
use crate::net::Cmd;
use winit::keyboard::Key;
use winit::keyboard::NamedKey;

fn line(id: &str, who: &str, parent: &str, root: &str) -> Message {
  let mut m = Message::system(1, Platform::Twitch, "x");
  (m.kind, m.id, m.login, m.nick, m.body) = (Kind::Chat, id.into(), who.into(), who.into(), format!("say {id}").into());
  if !parent.is_empty() {
    let x = m.extra();
    (x.parent, x.root) = (parent.into(), root.into());
  }
  m
}

fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(900, 600);
  app.redraw();
  while rx.try_recv().is_ok() {}
  (app, rx)
}

fn say(app: &mut App, m: Message) {
  app.on_chat("forsen", Chat::Msg(m));
}

fn bodies(app: &App) -> Vec<String> {
  app.tabs[app.active].view.msgs().map(|m| m.body.to_string()).collect()
}

fn pick(app: &mut App, id: &str) {
  let h = app.chat_area().h;
  let res = &mut app.res;
  assert!(app.tabs[app.active].view.select_id(id, res, h));
}

fn press(app: &mut App, k: Key<&'static str>) {
  app.mode = crate::model::vi::Mode::Normal;
  app.on_key(k, None);
}

#[test]
fn t_covers_the_tab_with_the_chain_and_new_replies_land_in_it() {
  let (mut app, _rx) = boot();
  for m in [line("a", "ann", "", ""), line("z", "zed", "", ""), line("b", "bob", "a", "a"), line("c", "cy", "b", "a")] {
    say(&mut app, m);
  }
  let tab = app.active;
  pick(&mut app, "b");
  press(&mut app, Key::Character("t"));
  assert!(app.chain_shown());
  assert_eq!(bodies(&app), ["say a", "say b", "say c"], "the chain, oldest first, without the stranger");
  assert_eq!(app.tabs[tab].view.selected().map(|m| &*m.id), Some("b"), "the line you opened it on stays picked");
  assert!(app.hint().starts_with("reply thread"));
  // a reply to the chain lands at the bottom, a stranger's line does not, and the tab's own lines keep filling
  say(&mut app, line("d", "dee", "c", "a"));
  say(&mut app, line("y", "yan", "", ""));
  assert_eq!(bodies(&app), ["say a", "say b", "say c", "say d"]);
  app.redraw();
  crate::bench::dump(&app.canvas);
  assert_eq!(app.tabs[tab].under.as_ref().map(|v| v.len()), Some(6));
  // t again, or esc, goes back to the tab as it was
  press(&mut app, Key::Character("t"));
  assert!(!app.chain_shown() && app.chain.is_none() && app.tabs[tab].under.is_none());
  assert_eq!(bodies(&app).len(), 6);
  pick(&mut app, "c");
  press(&mut app, Key::Character("t"));
  assert!(app.chain_shown());
  press(&mut app, Key::Named(NamedKey::Escape));
  assert!(!app.chain_shown() && app.tabs[tab].under.is_none());
}

#[test]
fn a_line_with_no_thread_says_so_and_covers_nothing() {
  let (mut app, _rx) = boot();
  say(&mut app, line("a", "ann", "", ""));
  pick(&mut app, "a");
  press(&mut app, Key::Character("t"));
  assert!(!app.chain_shown());
  assert_eq!(bodies(&app).last().map(String::as_str), Some("no thread on that line"));
}

#[test]
fn g_then_t_still_moves_between_tabs_with_a_line_picked() {
  let (mut app, _rx) = boot();
  for m in [line("a", "ann", "", ""), line("b", "bob", "a", "a")] {
    say(&mut app, m);
  }
  pick(&mut app, "b");
  press(&mut app, Key::Character("g"));
  press(&mut app, Key::Character("t"));
  assert!(!app.chain_shown(), "gt is the tab chord, not a thread");
}

#[test]
fn copy_thread_copies_oldest_first_one_line_each() {
  let (mut app, mut rx) = boot();
  for m in [line("a", "ann", "", ""), line("b", "bob", "a", "a")] {
    say(&mut app, m);
  }
  pick(&mut app, "b");
  app.copy_thread();
  let copies: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).filter(|c| matches!(c, Cmd::Copy { .. })).collect();
  assert!(
    matches!(copies.as_slice(), [Cmd::Copy { text, done }] if text == "ann: say a\nbob: say b" && done == "thread copied")
  );
}

#[test]
fn the_line_menu_offers_copy_thread_only_inside_a_thread() {
  let (mut app, _rx) = boot();
  for m in [line("a", "ann", "", ""), line("b", "bob", "a", "a"), line("z", "zed", "", "")] {
    say(&mut app, m);
  }
  let rows = |app: &mut App, id: &str| {
    pick(app, id);
    app.line_menu_key();
    let r: Vec<_> = app.pop().unwrap().menu.items.iter().map(|i| i.1.clone()).collect();
    app.close_pop();
    r
  };
  assert!(rows(&mut app, "b").contains(&"copy thread".to_owned()));
  assert!(!rows(&mut app, "z").contains(&"copy thread".to_owned()));
}
