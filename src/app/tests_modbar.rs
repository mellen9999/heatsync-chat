// the mod hover toolbar, headless: it shows on the line under the pointer only in a room we moderate, a
// click runs the line menu's own action, and the settings file keeps only buttons it knows.
// HSC_DUMP=$d/x.ppm ... cargo test --release headless_modbar_look

use super::*;
use crate::model::message::{Kind as MsgKind, Message};
use crate::model::modbar;
use crate::model::modcmd::Verb;
use crate::model::settings::Settings;
use crate::net::Cmd;
use crate::net::auth::Me;
use crate::net::irc::Chat;
use crate::ui::theme;

type Rx = tokio::sync::mpsc::UnboundedReceiver<Cmd>;

fn boot() -> (App, Rx) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen kick:xqc".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(560, 300);
  app.on_net(NetEvent::Me(Some(Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: Some("mellen_k".into()),
    youtube: None,
    packs: vec![],
  })));
  app.redraw();
  while rx.try_recv().is_ok() {}
  (app, rx)
}

fn said(p: Platform, login: &str, id: &str) -> Message {
  let mut m = Message::system(now_ms(), p, "hello there");
  m.kind = MsgKind::Chat;
  m.login = login.into();
  m.nick = login.into();
  m.id = id.into();
  m
}

fn mods(rx: &mut Rx) -> Vec<crate::net::modact::Req> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Mod(r) => Some(r),
      _ => None,
    })
    .collect()
}

// a twitch room we moderate with one line of bob's at the bottom, the pointer resting on it
fn on_bob(buttons: &str) -> (App, Rx) {
  let (mut app, rx) = boot();
  app.on_chat("forsen", Chat::Mod(true));
  app.set.mod_buttons = modbar::parse(buttons).0;
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "bob", "m1")));
  app.redraw();
  rest(&mut app);
  (app, rx)
}

fn rest(app: &mut App) {
  let a = app.chat_area();
  app.pointer = (a.x + 40, a.bottom() - 5);
  app.modbar_hover();
  app.redraw();
}

// the status line a click says lands as a newest line: bob speaks again and the pointer rests on him
fn again(app: &mut App) -> crate::ui::mod_bar::Bar {
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "bob", "m1")));
  app.redraw();
  rest(app);
  bar(app).expect("on bob")
}

fn bar(app: &App) -> Option<crate::ui::mod_bar::Bar> {
  app.modbar.shown.map(|s| s.0)
}

fn labels(app: &App) -> Vec<&'static str> {
  bar(app).map_or_else(Vec::new, |b| (0..b.n).map(|i| b.label(i)).collect())
}

fn over(app: &mut App, b: crate::ui::mod_bar::Bar, label: &str) {
  let i = (0..b.n).find(|&i| b.label(i) == label).expect("that button is on the bar");
  let r = b.cell[i];
  app.pointer = (r.x + r.w / 2, r.y + r.h / 2);
  app.modbar_hover();
  app.redraw();
}

#[test]
fn hidden_unless_a_mod_has_picked_buttons_and_the_line_is_someones() {
  // not a mod here: nothing, though buttons are picked
  let (mut app, _rx) = boot();
  app.set.mod_buttons = modbar::parse("ban x").0;
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "bob", "m1")));
  app.redraw();
  rest(&mut app);
  assert!(bar(&app).is_none());
  // a mod, nothing picked: nothing
  let (mut app, _rx) = on_bob("");
  assert!(bar(&app).is_none());
  // picked: the strip, buttons in the ext's order
  app.set.mod_buttons = modbar::parse("ban x 10m").0;
  rest(&mut app);
  assert_eq!(labels(&app), ["x", "10m", "\u{26d4}"]);
  // the pointer off the chat takes it away, and so does leaving the window
  app.pointer = (5, 0);
  app.modbar_hover();
  app.redraw();
  assert!(bar(&app).is_none());
  rest(&mut app);
  assert!(bar(&app).is_some());
  app.modbar_leave();
  app.redraw();
  assert!(bar(&app).is_none());
  // an event line has no person; our own line is not for banning
  let (mut app, _rx) = on_bob("ban");
  app.on_chat("forsen", Chat::Msg(Message::system(now_ms(), Platform::Twitch, "bob was timed out")));
  app.redraw();
  rest(&mut app);
  assert!(bar(&app).is_none(), "the newest line is an event");
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "mellen", "m3")));
  app.redraw();
  rest(&mut app);
  assert!(bar(&app).is_none(), "our own line");
}

#[test]
fn a_line_without_an_id_gets_no_delete() {
  let (mut app, _rx) = on_bob("x 1m");
  assert_eq!(labels(&app), ["x", "1m"]);
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "amy", "")));
  app.redraw();
  rest(&mut app);
  assert_eq!(labels(&app), ["1m"]);
}

#[test]
fn kick_has_no_purge() {
  let (mut app, _rx) = boot();
  app.set.mod_buttons = modbar::parse("purge 1m").0;
  // we moderate xqc on kick (our line wears the badge) and forsen on twitch
  let mut me = said(Platform::Kick, "mellen-k", "k0");
  me.badges = vec![("moderator".into(), "1".into())].into();
  app.on_net(NetEvent::Relay { platform: Platform::Kick, key: "xqc".into(), msgs: vec![me], history: false });
  app.on_chat("forsen", Chat::Mod(true));
  app.on_net(NetEvent::Relay {
    platform: Platform::Kick,
    key: "xqc".into(),
    msgs: vec![said(Platform::Kick, "bob", "k1")],
    history: false,
  });
  app.redraw();
  rest(&mut app);
  assert_eq!(labels(&app), ["1m"], "kick's shortest timeout is a minute");
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "bob", "m1")));
  app.redraw();
  rest(&mut app);
  assert_eq!(labels(&app), ["1m", "purge"]);
}

#[test]
fn a_click_runs_the_line_menus_action_and_leaves_the_line_alone() {
  let (mut app, mut rx) = on_bob("x 10m purge ban");
  let b = again(&mut app);
  over(&mut app, b, "10m");
  assert_eq!(bar(&app).map(|b| b.n), Some(4));
  assert!(app.modbar.shown.unwrap().1.is_some(), "the cell under the pointer lights");
  app.click();
  let got = mods(&mut rx);
  assert_eq!(got.len(), 1);
  assert_eq!((got[0].verb, &*got[0].login, &*got[0].chan), (Verb::Timeout(600), "bob", "forsen"));
  assert!(app.card().is_none() && app.pop().is_none(), "the line's own click stays out");
  // delete goes by the line's id (the status line may have moved the chat: look again)
  let b = again(&mut app);
  over(&mut app, b, "x");
  app.click();
  let got = mods(&mut rx);
  assert_eq!((got[0].verb, got[0].msg_id.as_deref()), (Verb::Delete, Some("m1")));
  let b = again(&mut app);
  over(&mut app, b, "purge");
  app.click();
  assert_eq!(mods(&mut rx)[0].verb, Verb::Purge);
  let b = again(&mut app);
  // a click in the gap between cells is eaten and sends nothing
  let gap = (b.cell[0].right(), b.cell[0].y + 2);
  app.pointer = gap;
  app.click();
  assert!(mods(&mut rx).is_empty() && app.card().is_none());
  // ban asks first when confirming is on; nothing goes until yes
  app.set.confirm_ban = true;
  let b = again(&mut app);
  over(&mut app, b, "\u{26d4}");
  app.click();
  assert!(mods(&mut rx).is_empty() && app.pop().is_some());
  app.pop_key(winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter));
  let got = mods(&mut rx);
  assert_eq!((got[0].verb, &*got[0].login), (Verb::Ban, "bob"));
  // off the strip the line is clickable as ever: its name opens the card
  app.close_pop();
  rest(&mut app);
  assert!(app.card().is_none());
}

#[test]
fn the_file_keeps_only_known_buttons() {
  let s = Settings::parse("mod_buttons=ban,evil,timeout_1m,mt,../../x,delete_message\n");
  assert_eq!(modbar::ids(s.mod_buttons), "delete_message,timeout_1m,ban");
  let again = Settings::parse(&s.render());
  assert_eq!(again.mod_buttons, s.mod_buttons);
  assert_eq!(Settings::parse("mod_buttons=\n").mod_buttons, 0);
  assert_eq!(Settings::default().mod_buttons, 0, "opt in, like the ext");
  // the command replaces the picks; unknown words are told, and all-unknown changes nothing
  let (mut app, _rx) = boot();
  app.local(crate::model::settings::local_cmd("/modbuttons 1m ban").unwrap());
  assert_eq!(app.set.mod_buttons, modbar::parse("1m ban").0);
  app.local(crate::model::settings::local_cmd("/modbuttons nope").unwrap());
  assert_eq!(app.set.mod_buttons, modbar::parse("1m ban").0);
  app.local(crate::model::settings::local_cmd("/modbuttons off").unwrap());
  assert_eq!(app.set.mod_buttons, 0);
}

#[test]
fn headless_modbar_look() {
  let (mut app, _rx) = on_bob("x 1m 10m 1h 24h 7d purge ban unban");
  for i in 0..4 {
    app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "amy", &format!("a{i}"))));
  }
  app.redraw();
  let a = app.chat_area();
  app.pointer = (a.x + 40, a.bottom() - 25);
  app.modbar_hover();
  app.redraw();
  let b = bar(&app).expect("on a line");
  over(&mut app, b, "10m");
  assert!(app.canvas.px.iter().any(|&p| p & 0xffffff == theme::DIM), "the edge is drawn");
  let r = b.cell[2];
  assert_eq!(app.canvas.px[(r.y + 1) as usize * app.canvas.w as usize + r.x as usize + 1] & 0xffffff, theme::WHITE);
  crate::bench::dump(&app.canvas);
  // a line arriving while the strip shows scrolls the chat: no ghost of the strip is left behind
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "cy", "z9")));
  app.redraw();
  let b2 = bar(&app).expect("on a line");
  let mut seen = 0;
  for y in a.y..a.bottom() {
    for x in b.rect.x..b.rect.right() {
      if app.canvas.px[y as usize * app.canvas.w as usize + x as usize] & 0xffffff == theme::DIM
        && !b2.rect.contains(x, y)
      {
        seen += 1;
      }
    }
  }
  assert!(seen < b.rect.w as usize, "only chat text is left over the old strip, not its border");
}

// ---- the reply button (hoverReply) rides the same strip ----

// a twitch room we do not moderate, bob's line at the bottom, the pointer resting on it
fn on_bob_replying() -> (App, Rx) {
  let (mut app, rx) = boot();
  app.set.hover_reply = true;
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "bob", "m1")));
  app.redraw();
  rest(&mut app);
  (app, rx)
}

fn replies(app: &App) -> Option<(Box<str>, Box<str>)> {
  app.reply.as_ref().map(|r| (r.id.clone(), r.nick.clone()))
}

#[test]
fn the_reply_button_is_off_unless_asked_for_and_wants_a_line_it_can_answer() {
  assert!(!Settings::default().hover_reply, "off by default, like the site");
  // off: nothing on the line, mod room or not
  let (mut app, _rx) = boot();
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "bob", "m1")));
  app.redraw();
  rest(&mut app);
  assert!(bar(&app).is_none());
  // on: one cell, the hook arrow, though we moderate nothing here
  app.set.hover_reply = true;
  rest(&mut app);
  assert_eq!(labels(&app), ["\u{21aa}"]);
  // our own line can be answered too (the line menu offers it)
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "mellen", "m2")));
  app.redraw();
  rest(&mut app);
  assert_eq!(labels(&app), ["\u{21aa}"]);
  // a line with no platform id has nothing to reply to; an event line is nobody's
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "amy", "")));
  app.redraw();
  rest(&mut app);
  assert!(bar(&app).is_none());
  app.on_chat("forsen", Chat::Msg(Message::system(now_ms(), Platform::Twitch, "bob was timed out")));
  app.redraw();
  rest(&mut app);
  assert!(bar(&app).is_none());
  // switched back off while showing: it goes
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "bob", "m3")));
  app.redraw();
  rest(&mut app);
  assert!(bar(&app).is_some());
  app.set.hover_reply = false;
  rest(&mut app);
  assert!(bar(&app).is_none());
}

#[test]
fn the_button_is_drawn_over_the_row_without_moving_anything_else() {
  let (mut app, _rx) = boot();
  app.set.hover_reply = true;
  for i in 0..4 {
    app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "amy", &format!("a{i}"))));
  }
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "bob", "m1")));
  app.redraw();
  let a = app.chat_area();
  // the pointer far from any line: the chat as it is without the button
  app.pointer = (5, 0);
  app.modbar_hover();
  app.redraw();
  assert!(bar(&app).is_none());
  let before = app.canvas.px.clone();
  let h = |app: &mut App| {
    let t = app.tabs.get_mut(app.active).unwrap();
    (a.y..a.bottom()).map(|y| t.view.line_at(a, y, &mut app.res)).collect::<Vec<_>>()
  };
  let lines = h(&mut app);
  // the pointer on the newest line
  rest(&mut app);
  let b = bar(&app).expect("the button shows on hover");
  assert_eq!(b.n, 1);
  assert_eq!(h(&mut app), lines, "every line keeps its top, bottom and place: nothing re-wrapped");
  let (w, mut outside, mut inside) = (app.canvas.w, 0, 0);
  for (i, (&was, &now)) in before.iter().zip(&app.canvas.px).enumerate() {
    if was != now {
      let (x, y) = (i as i32 % w, i as i32 / w);
      if b.rect.contains(x, y) {
        inside += 1;
      } else {
        outside += 1;
      }
    }
  }
  assert_eq!(outside, 0, "not one pixel moved outside the button");
  assert!(inside > 0, "the button is drawn");
  assert!(b.rect.bottom() <= a.bottom() && b.rect.right() <= a.right(), "inside the chat");
  // the pointer leaving takes it away and the row is exactly as it was
  app.pointer = (5, 0);
  app.modbar_hover();
  app.redraw();
  assert!(bar(&app).is_none());
  assert!(app.canvas.px == before, "nothing of the button is left behind");
}

#[test]
fn white_on_black_and_the_hovered_arrow_inverts() {
  let (mut app, _rx) = on_bob_replying();
  let b = bar(&app).expect("on bob");
  let r = b.cell[0];
  let at = |app: &App, x: i32, y: i32| app.canvas.px[y as usize * app.canvas.w as usize + x as usize] & 0xffffff;
  // resting: black ground, white arrow, a thin edge
  assert_eq!(at(&app, r.x + 1, r.y + 1), theme::BLACK);
  assert!(app.canvas.px.contains(&theme::DIM));
  let lit = |app: &App, ink: u32| {
    (r.y..r.bottom()).flat_map(|y| (r.x..r.right()).map(move |x| (x, y))).filter(|&(x, y)| at(app, x, y) == ink).count()
  };
  let plain = lit(&app, theme::WHITE);
  assert!(plain > 0, "the arrow is white");
  // under the pointer: a white block with the arrow black
  over(&mut app, b, "\u{21aa}");
  assert!(app.modbar.shown.unwrap().1.is_some());
  assert!(lit(&app, theme::WHITE) > (r.w * r.h) as usize / 2, "a white block");
  assert!(lit(&app, theme::BLACK) > 0, "the arrow black on it");
  crate::bench::dump(&app.canvas);
}

#[test]
fn a_click_starts_the_reply_the_line_menu_would() {
  let (mut app, mut rx) = on_bob_replying();
  assert!(app.reply.is_none());
  let b = bar(&app).expect("on bob");
  over(&mut app, b, "\u{21aa}");
  app.click();
  assert_eq!(replies(&app), Some(("m1".into(), "bob".into())));
  assert_eq!(app.mode, crate::model::vi::Mode::Insert, "the box is ready to type");
  assert!(app.card().is_none() && app.pop().is_none(), "the line's own click stays out");
  assert!(mods(&mut rx).is_empty());
  // the menu's reply on the same line gives the same
  let was = app.reply.take().unwrap();
  let a = app.chat_area();
  let y = a.bottom() - 5;
  assert!(app.tabs[app.active].view.select_at(a, y, &mut app.res));
  app.reply_selected();
  let now = app.reply.as_ref().expect("the menu's reply");
  assert_eq!((&now.id, &now.nick, &now.text, now.platform), (&was.id, &was.nick, &was.text, was.platform));
}

#[test]
fn it_sits_beside_the_mod_buttons_and_each_clicks_alone() {
  let (mut app, mut rx) = on_bob("x 10m");
  app.set.hover_reply = true;
  let b = again(&mut app);
  assert_eq!(labels(&app), ["\u{21aa}", "x", "10m"]);
  for i in 1..b.n {
    assert!(b.cell[i - 1].right() < b.cell[i].x, "cells apart: nothing overlaps");
  }
  assert_eq!(b.rect.right(), app.chat_area().right() - 4, "the mod buttons keep their place at the right");
  // the mod button still does its job and starts no reply
  over(&mut app, b, "10m");
  app.click();
  let got = mods(&mut rx);
  assert_eq!((got.len(), got[0].verb), (1, Verb::Timeout(600)));
  assert!(app.reply.is_none());
  // the reply starts a reply and sends no mod action
  let b = again(&mut app);
  over(&mut app, b, "\u{21aa}");
  app.click();
  assert!(mods(&mut rx).is_empty());
  assert_eq!(replies(&app), Some(("m1".into(), "bob".into())));
  // in a room we moderate the reply shows on our own line (no mod buttons there) and alone
  app.on_chat("forsen", Chat::Msg(said(Platform::Twitch, "mellen", "m2")));
  app.redraw();
  rest(&mut app);
  assert_eq!(labels(&app), ["\u{21aa}"]);
}

#[test]
fn the_switch_is_kept_in_the_file_and_flipped_with_its_command() {
  let s = Settings::parse("hover_reply=true\n");
  assert!(s.hover_reply);
  assert!(Settings::parse(&s.render()).hover_reply);
  assert!(!Settings::parse("hover_reply=false\n").hover_reply);
  let (mut app, _rx) = boot();
  app.local(crate::model::settings::local_cmd("/hoverreply").unwrap());
  assert!(app.set.hover_reply);
  app.local(crate::model::settings::local_cmd("/hoverreply").unwrap());
  assert!(!app.set.hover_reply);
}
