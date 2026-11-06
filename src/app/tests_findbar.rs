// the filter bar at the top of the chat, headless: shown on channel and mentions tabs only, `/` and a click
// give it the keys, typing filters live (names, words, @name, /regex/), a bad pattern filters nothing, esc clears
//   cargo test --release findbar

use super::*;
use crate::model::message::{Kind as MKind, Message};
use crate::net::Cmd;
use crate::ui::theme;
use winit::keyboard::Key;
use winit::keyboard::NamedKey;

fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(700, 300);
  app.redraw();
  while rx.try_recv().is_ok() {}
  (app, rx)
}

fn say(app: &mut App, id: &str, who: &str, body: &str) {
  let mut m = Message::system(1, Platform::Twitch, "x");
  (m.kind, m.id, m.login, m.nick, m.body) = (MKind::Chat, id.into(), who.into(), who.into(), body.into());
  app.on_chat("forsen", Chat::Msg(m));
}

fn type_in(app: &mut App, s: &str) {
  for c in s.chars() {
    let t = c.to_string();
    app.find_key(Key::Character(&t), Some(&t));
  }
}

fn open(app: &mut App) {
  app.mode = crate::model::vi::Mode::Normal;
  app.on_key(Key::Character("/"), None);
}

fn px(app: &App, x: i32, y: i32) -> u32 {
  app.canvas.px[(y * app.canvas.w + x) as usize]
}

fn shown(app: &App) -> usize {
  app.tabs[app.active].view.found()
}

fn chatty() -> App {
  let (mut app, _rx) = boot();
  say(&mut app, "1", "bob", "foo is here");
  say(&mut app, "2", "alice", "FOOOD time");
  say(&mut app, "3", "carol", "nothing");
  app.redraw();
  app
}

#[test]
fn the_bar_is_drawn_on_chat_tabs_and_not_on_the_feed() {
  let mut app = chatty();
  let a = app.find_area();
  assert!(app.find.h > 0 && a.h == app.find.h, "a row of its own on a channel tab");
  assert_eq!(px(&app, a.x, a.y), theme::DIM, "a square dim box while the keys are elsewhere");
  assert_eq!(px(&app, a.right() - 1, a.bottom() - 1), theme::DIM);
  assert_eq!(app.chat_area().y, a.bottom(), "the chat starts under it");
  // the feed is not a chat: no bar, and the chat takes the row back
  let top = app.chat_area().y;
  let feed = app.tab_of(crate::model::tabs::Kind::Feed).unwrap();
  app.select(feed);
  app.redraw();
  assert_eq!(app.find.h, 0);
  assert_eq!(app.chat_area().y, top - a.h);
  assert!(!(0..a.w).any(|x| px(&app, x, a.y) == theme::DIM && px(&app, x, a.bottom() - 1) == theme::DIM));
  // mentions is a chat view: it has it
  let men = app.tab_of(crate::model::tabs::Kind::Mentions).unwrap();
  app.select(men);
  app.redraw();
  assert!(app.find.h > 0);
  // a settings screen covers the chat: not drawn under it
  app.open_screen(screen::Page::Settings);
  app.redraw();
  assert_eq!(app.find.h, 0);
  crate::bench::dump(&app.canvas);
}

#[test]
fn slash_gives_the_bar_the_keys_and_clicking_it_does_too() {
  let mut app = chatty();
  open(&mut app);
  assert!(app.bar_find() && app.find.on);
  app.redraw();
  let a = app.find_area();
  assert_eq!(px(&app, a.x, a.y), theme::WHITE, "a white box while it has the keys");
  assert!(app.input_area().h > 0 && app.mode_line().1.is_empty(), "the message box carries on below");
  // enter hands the keys back
  app.find_key(Key::Named(NamedKey::Enter), None);
  assert!(!app.bar_find());
  app.redraw();
  assert_eq!(px(&app, a.x, a.y), theme::DIM);
  // a click on the bar takes them
  app.pointer = (a.x + 20, a.y + 2);
  assert!(app.find_click() && app.bar_find());
  // a click on the chat is not the bar
  app.find_key(Key::Named(NamedKey::Escape), None);
  let c = app.chat_area();
  app.pointer = (c.x + 20, c.y + 2);
  assert!(!app.find_click() && !app.bar_find());
}

#[test]
fn typing_filters_live_and_esc_clears_it() {
  let mut app = chatty();
  open(&mut app);
  type_in(&mut app, "foo");
  assert_eq!(shown(&app), 2, "alice's FOOOD too: plain text ignores case");
  assert_eq!(app.find.note, "2 found");
  type_in(&mut app, "o");
  assert_eq!(shown(&app), 1, "fooo");
  // enter keeps the filter, n walks it, esc clears it and it is all back
  app.find_key(Key::Named(NamedKey::Enter), None);
  assert_eq!(shown(&app), 1);
  assert!(app.find_on());
  assert!(app.find_clear());
  assert_eq!(shown(&app), 3);
  assert!(!app.find_on() && app.find.query.is_empty() && app.find.note.is_empty());
  // esc while typing clears too
  open(&mut app);
  type_in(&mut app, "@car");
  assert_eq!(shown(&app), 1, "@name keeps one person");
  app.find_key(Key::Named(NamedKey::Escape), None);
  assert_eq!(shown(&app), 3);
  assert!(!app.bar_find());
  // new lines respect the filter as they land
  open(&mut app);
  type_in(&mut app, "zzz");
  say(&mut app, "4", "dave", "zzz!");
  say(&mut app, "5", "erin", "no");
  assert_eq!(shown(&app), 1);
}

#[test]
fn a_regex_filters_by_pattern_and_a_bad_one_filters_nothing() {
  let mut app = chatty();
  open(&mut app);
  type_in(&mut app, "/fo+/");
  assert_eq!(shown(&app), 1, "case-sensitive: bob's foo, not FOOOD");
  type_in(&mut app, "i");
  assert_eq!(shown(&app), 2, "`/fo+/i`");
  // half-typed, then broken: everything shows, flagged in the bar
  for _ in 0..6 {
    app.find_key(Key::Named(NamedKey::Backspace), None);
  }
  type_in(&mut app, "/[/");
  assert_eq!(shown(&app), 3);
  assert_eq!(app.find.note, "bad pattern");
  assert!(app.tabs[app.active].view.search_bad());
  // enter leaves it up, flagged; esc clears the flag
  app.find_key(Key::Named(NamedKey::Enter), None);
  assert_eq!(app.find.query, "/[/");
  app.redraw();
  assert!(app.find_clear());
  assert!(app.find.note.is_empty() && !app.tabs[app.active].view.search_bad());
}

#[test]
fn a_regex_in_the_bar_paints_for_a_look() {
  let mut app = chatty();
  open(&mut app);
  type_in(&mut app, "/fo+/i");
  app.redraw();
  let a = app.find_area();
  let lit = |x0, x1| (a.y + 1..a.bottom() - 1).any(|y| (x0..x1).any(|x| px(&app, x, y) != theme::BLACK));
  assert!(lit(a.x + 2, a.x + 80), "the query");
  assert!(lit(a.right() - 80, a.right() - 1), "the count");
  crate::bench::dump(&app.canvas);
}
