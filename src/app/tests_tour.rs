// the first-run tour: after the first channel of a new install, six cards, keyboard-driven, once per install,
// /tour brings it back, the last card's switches are the settings' own
use super::*;
use crate::model::settings::Local;
use crate::model::tour::{CARDS, LAST, seen};
use crate::net::Cmd;
use tokio::sync::mpsc::UnboundedReceiver;
use winit::keyboard::{Key, NamedKey};

fn boot(chans: &[&str]) -> (App, UnboundedReceiver<Cmd>) {
  let (net, rx) = Net::tap();
  let mut app = App::new(net, chans.iter().map(|c| (*c).into()).collect(), false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(420, 800);
  app.redraw();
  (app, rx)
}

fn named(app: &mut App, k: NamedKey) {
  app.on_key(Key::Named(k), None);
}

fn typed(app: &mut App, s: &'static str) {
  for (i, ch) in s.char_indices() {
    let c = &s[i..i + ch.len_utf8()];
    app.on_key(Key::Character(c), Some(c));
  }
}

// a new install: no channels, the form, a name, enter
fn first_channel(app: &mut App) {
  app.first_run();
  assert!(app.form.is_some());
  typed(app, "forsen");
  named(app, NamedKey::Enter);
}

fn at(app: &App) -> Option<usize> {
  app.tour.as_ref().map(|t| t.at)
}

fn fresh() -> (App, UnboundedReceiver<Cmd>) {
  crate::store::remove("tour");
  boot(&[])
}

#[test]
fn the_first_channel_of_a_new_install_starts_the_tour_once() {
  let (mut app, _rx) = fresh();
  assert!(app.tour.is_none(), "the empty window shows no tour");
  first_channel(&mut app);
  assert_eq!(at(&app), Some(0));
  assert!(seen(), "remembered from the moment it shows");
  app.redraw();
  // a second install-less start (same config) never shows it again
  named(&mut app, NamedKey::Escape);
  let (mut again, _rx) = boot(&[]);
  first_channel(&mut again);
  assert!(again.tour.is_none(), "once per install");
  crate::store::remove("tour");
}

#[test]
fn an_account_with_channels_or_a_saved_tab_never_gets_it() {
  crate::store::remove("tour");
  let (mut app, _rx) = boot(&["forsen"]);
  app.first_run();
  assert!(app.tour.is_none() && app.form.is_none());
  assert!(!seen());
}

#[test]
fn enter_and_right_walk_the_cards_and_the_last_one_finishes() {
  let (mut app, _rx) = fresh();
  first_channel(&mut app);
  named(&mut app, NamedKey::Enter);
  assert_eq!(at(&app), Some(1));
  named(&mut app, NamedKey::ArrowRight);
  assert_eq!(at(&app), Some(2));
  named(&mut app, NamedKey::ArrowLeft);
  assert_eq!(at(&app), Some(1));
  for _ in 1..LAST {
    named(&mut app, NamedKey::ArrowRight);
  }
  assert_eq!(at(&app), Some(LAST));
  app.redraw();
  named(&mut app, NamedKey::Enter);
  assert!(app.tour.is_none(), "enter on the last card is done");
  crate::store::remove("tour");
}

#[test]
fn esc_skips_from_any_card_and_the_keys_never_reach_the_box() {
  let (mut app, _rx) = fresh();
  first_channel(&mut app);
  typed(&mut app, "hello");
  assert!(app.line.text.is_empty(), "the card owns the keys");
  named(&mut app, NamedKey::ArrowRight);
  named(&mut app, NamedKey::ArrowRight);
  named(&mut app, NamedKey::Escape);
  assert!(app.tour.is_none());
  typed(&mut app, "x");
  assert_eq!(app.line.text, "x", "typing is the box's again");
  crate::store::remove("tour");
}

#[test]
fn the_last_card_flips_the_real_settings() {
  let (mut app, _rx) = fresh();
  first_channel(&mut app);
  for _ in 0..LAST {
    named(&mut app, NamedKey::ArrowRight);
  }
  let before = (app.set.timestamps, app.set.vi, app.set.hover_reply);
  assert_eq!(before, (false, false, false), "they start off");
  // timestamps, then j to vi, then j to the hover reply
  typed(&mut app, " ");
  named(&mut app, NamedKey::Space);
  assert!(app.set.timestamps);
  typed(&mut app, "j");
  named(&mut app, NamedKey::Space);
  assert!(app.set.vi);
  named(&mut app, NamedKey::ArrowDown);
  named(&mut app, NamedKey::Space);
  assert!(app.set.hover_reply);
  // a second press turns it back off: it is a switch
  named(&mut app, NamedKey::Space);
  assert!(!app.set.hover_reply);
  // the same row settings shows: one switch, two doors
  let d = crate::model::schema::def("timestamps").unwrap();
  assert_eq!(d.on(&app.set), Some(true));
  app.redraw();
  named(&mut app, NamedKey::Enter);
  assert!(app.tour.is_none() && app.set.timestamps && app.set.vi, "done keeps what was turned on");
  crate::store::remove("tour");
}

#[test]
fn space_on_the_other_cards_does_nothing() {
  let (mut app, _rx) = fresh();
  first_channel(&mut app);
  named(&mut app, NamedKey::Space);
  assert_eq!(at(&app), Some(0));
  assert!(!app.set.timestamps && !app.set.vi);
  crate::store::remove("tour");
}

#[test]
fn slash_tour_brings_it_back_from_the_first_card() {
  let (mut app, _rx) = boot(&["forsen"]);
  crate::model::tour::mark_seen();
  app.local(Local::Tour);
  assert_eq!(at(&app), Some(0));
  named(&mut app, NamedKey::ArrowRight);
  named(&mut app, NamedKey::Escape);
  app.local(Local::Tour);
  assert_eq!(at(&app), Some(0), "from card 1 again");
  crate::store::remove("tour");
}

#[test]
fn the_slash_command_and_the_help_both_name_it() {
  assert_eq!(crate::model::settings::local_cmd("/tour"), Some(Local::Tour));
  assert!(crate::model::slash::all().iter().any(|c| c.name == "/tour"));
  let help = crate::model::keys::KEYS.iter().flat_map(|g| g.rows).any(|r| crate::i18n::t(r.does).contains("/tour"));
  assert!(help, "the help says where the tour is");
}

#[test]
fn the_buttons_work_with_the_mouse_too() {
  let (mut app, _rx) = boot(&["forsen"]);
  app.local(Local::Tour);
  app.redraw();
  let g = app.tour_geo().unwrap();
  app.pointer = (g.next.x + 2, g.next.y + 2);
  app.click();
  assert_eq!(at(&app), Some(1));
  let g = app.tour_geo().unwrap();
  app.pointer = (g.back.x + 2, g.back.y + 2);
  app.click();
  assert_eq!(at(&app), Some(0));
  let g = app.tour_geo().unwrap();
  app.pointer = (g.skip.x + 2, g.skip.y + 2);
  app.click();
  assert!(app.tour.is_none());
  crate::store::remove("tour");
}

#[test]
fn every_card_draws_and_the_card_covers_the_chat() {
  let (mut app, _rx) = boot(&["forsen"]);
  app.local(Local::Tour);
  for i in 0..CARDS.len() {
    app.redraw();
    let area = app.chat_area();
    // the panel's own border is white; the rest of the area is black behind it
    let g = app.tour_geo().unwrap();
    let px = |x: i32, y: i32| app.canvas.px[(y * app.canvas.w + x) as usize] & 0xffffff;
    assert_eq!(px(g.panel.x, g.panel.y), 0xffffff, "card {i} border");
    assert_eq!(px(area.x + 1, area.y + 1), 0, "card {i} backdrop");
    named(&mut app, NamedKey::ArrowRight);
  }
  crate::store::remove("tour");
}
