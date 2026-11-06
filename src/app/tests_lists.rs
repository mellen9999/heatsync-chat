// the list screens (muted & blocked people, filter rules, ban reasons…): reached from settings, x removes
// and a adds through the slash commands' own paths
use super::lists::Kind;
use super::screen::{Page, Target};
use super::*;
use crate::model::filter;
use crate::ui::sheet::L;
use winit::keyboard::{Key, NamedKey};

fn boot() -> App {
  let (net, _rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  app
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
  lines.into_iter().filter_map(|l| if let L::Row { a, b, .. } = l { Some(format!("{a}|{b}")) } else { None }).collect()
}

// settings → the row → enter, the way a person gets there
fn via_settings(app: &mut App, id: &str) {
  app.open_screen(Page::Settings);
  let s = app.screen.as_mut().unwrap();
  let (lines, targets) = s.lines(&app.set);
  let at = targets.iter().position(|t| matches!(t, Target::Def(d) if d.id == id)).expect("a settings row");
  s.st.at = at;
  s.st.settle(&lines);
  app.on_key(Key::Named(NamedKey::Enter), None);
}

#[test]
fn muted_blocked_and_hidden_people_are_listed_and_x_lets_them_back() {
  let mut app = boot();
  app.mute("ann", None);
  app.mute("bob", Some(now_ms() + 2 * 3_600_000));
  app.block("cy");
  app.filter_cmd(crate::model::filtercmd::Cmd::Hide("dee".into(), true));
  via_settings(&mut app, "people");
  assert_eq!(app.screen.as_ref().unwrap().page(), Page::List);
  assert_eq!(rows(&app), ["ann|muted · forever", "bob|muted · 2h left", "cy|blocked", "dee|hidden in this tab"]);
  // x on each, top row each time
  press(&mut app, "x");
  assert!(!filter::with(|f| f.muted("ann", now_ms())) && filter::with(|f| f.muted("bob", now_ms())));
  press(&mut app, "x");
  assert!(!filter::with(|f| f.muted("bob", now_ms())));
  press(&mut app, "x");
  assert!(!filter::with(|f| f.blocked("cy")));
  press(&mut app, "x");
  assert!(!app.tabs[app.active].view.hidden_user("dee"));
  assert_eq!(rows(&app), ["nobody muted, blocked or hidden|"]);
  app.on_key(Key::Named(NamedKey::Escape), None);
  assert!(app.screen.is_none());
}

#[test]
fn enter_on_a_person_opens_their_card() {
  let mut app = boot();
  app.block("cy");
  app.list_open(Kind::People);
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert!(app.screen.is_none() && app.card().is_some_and(|c| &*c.who.login == "cy"));
}

#[test]
fn filter_rules_add_with_a_and_delete_with_x() {
  let mut app = boot();
  via_settings(&mut app, "filter_rules");
  assert_eq!(rows(&app), ["nothing yet|", "add…|a"]);
  press(&mut app, "a");
  type_in(&mut app, "highlight keyword Loot");
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert_eq!(filter::with(|f| f.rules().len()), 1);
  assert_eq!(rows(&app)[0], "on highlight keyword Loot|");
  // the field stays for the next one; esc, esc ends it
  app.on_key(Key::Named(NamedKey::Escape), None);
  assert!(!app.screen.as_ref().unwrap().typing());
  press(&mut app, "x");
  assert_eq!(filter::with(|f| f.rules().len()), 0);
  // enter on the add row starts typing too
  press(&mut app, "j");
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert!(app.screen.as_ref().unwrap().typing());
}

#[test]
fn rules_and_ban_reasons_each_have_a_list() {
  let mut app = boot();
  for (id, text, check) in [("ban_reasons", "spam", 0), ("filter_rules", "hide keyword nope", 1)] {
    via_settings(&mut app, id);
    press(&mut app, "a");
    type_in(&mut app, text);
    app.on_key(Key::Named(NamedKey::Enter), None);
    app.on_key(Key::Named(NamedKey::Escape), None);
    let n = filter::with(|f| [f.reasons().len(), f.rules().len()][check]);
    assert_eq!(n, 1, "{id} added");
    assert!(rows(&app).len() == 2, "{id}: {:?}", rows(&app));
    press(&mut app, "x");
    let n = filter::with(|f| [f.reasons().len(), f.rules().len()][check]);
    assert_eq!(n, 0, "{id} deleted");
    app.close_screen();
  }
}

#[test]
fn a_bad_rule_says_why_under_the_list() {
  let mut app = boot();
  app.list_open(Kind::Rules);
  press(&mut app, "a");
  type_in(&mut app, "zap");
  app.on_key(Key::Named(NamedKey::Enter), None);
  let tip = app.screen.as_ref().unwrap().ls.as_ref().unwrap().tip().to_owned();
  assert!(tip.starts_with("usage: /rule"), "{tip}");
}
