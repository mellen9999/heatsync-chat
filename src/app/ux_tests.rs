// headless looks at the first-minute ux: the ? button, the key map's vi marks, the log in button in the empty box,
// the first run's form and the /login /logout /account commands. offline; set HSC_DUMP=<dir> to write each frame
// as <dir>/<name>.ppm for a look
use tokio::sync::mpsc::UnboundedReceiver;

use super::screen::{Page, Target};
use super::*;
use crate::model::settings::Local;
use crate::net::Cmd;
use crate::net::auth::{Me, Provider};
use crate::ui::canvas::Rect;
use crate::ui::sheet::L;
use crate::ui::theme;
use winit::keyboard::Key;
use winit::keyboard::NamedKey;

fn boot(chans: &[&str], w: i32, h: i32) -> (App, UnboundedReceiver<Cmd>) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, chans.iter().map(|c| (*c).into()).collect(), false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(w, h);
  app.redraw();
  // what starting up asked of the net is not under test
  while rx.try_recv().is_ok() {}
  (app, rx)
}

fn me(twitch: Option<&str>, kick: Option<&str>) -> Me {
  Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: twitch.map(Into::into),
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: kick.map(Into::into),
    youtube: None,
    packs: vec![],
  }
}

fn dump(app: &App, name: &str) {
  if let Some(dir) = std::env::var_os("HSC_DUMP") {
    let c = &app.canvas;
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.ppm")), out);
  }
}

fn mid(r: Rect) -> (i32, i32) {
  (r.x + r.w / 2, r.y + r.h / 2)
}

fn px(app: &App, x: i32, y: i32) -> u32 {
  app.canvas.px[(y * app.canvas.w + x) as usize]
}

fn typed(app: &mut App, s: &str) {
  app.line.set_text(s.into());
  app.send();
}

fn said(app: &App) -> String {
  app.tabs[app.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
}

fn asked(rx: &mut UnboundedReceiver<Cmd>) -> Vec<Cmd> {
  std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

#[test]
fn the_question_mark_button_sits_by_the_gear_and_opens_the_key_map() {
  let (mut app, _rx) = boot(&["forsen"], 420, 800);
  let n = app.bar_map.len();
  assert_eq!(app.bar_hits.len(), n + 3, "the tabs, then +, the gear and the ?");
  let (gear, keys) = (app.bar_hits[n + 1], app.bar_hits[n + 2]);
  assert_eq!((keys.x, keys.y, keys.w, keys.h), (gear.right(), gear.y, gear.w, gear.h), "the gear's twin");
  assert_eq!(px(&app, keys.x, keys.y), theme::BLACK, "quiet at rest");
  // the pointer on it lights it like the gear: white, with black ink
  app.pointer = mid(keys);
  app.bar_hover();
  app.redraw();
  assert!(app.hover.keys && !app.hover.gear);
  assert_eq!(px(&app, keys.x, keys.y), theme::WHITE, "hover reverses it");
  dump(&app, "keys_button_hover");
  app.click();
  assert!(app.screen_is(Page::Keys), "a click opens the key map");
  app.redraw();
  assert_eq!(px(&app, keys.x, keys.y), theme::WHITE, "lit while its screen is open");
  assert_eq!(px(&app, gear.x, gear.y), theme::BLACK, "the gear is not");
  // the gear swaps to settings, the ? swaps back, and a second click on its own closes it
  app.pointer = mid(gear);
  app.click();
  assert!(app.screen_is(Page::Settings));
  app.pointer = mid(keys);
  app.click();
  assert!(app.screen_is(Page::Keys));
  app.click();
  assert!(app.screen.is_none(), "the same button again closes");
  // the keys: ? in vi normal mode and f1 anywhere
  app.on_key(Key::Named(NamedKey::F1), None);
  assert!(app.screen_is(Page::Keys));
}

#[test]
fn the_key_map_marks_the_vi_rows_and_dims_them_until_vi_is_on() {
  let (mut app, _rx) = boot(&["forsen"], 420, 800);
  app.set.vi = false;
  app.local(Local::Keys);
  app.redraw();
  dump(&app, "keys_vi_off");
  let (lines, _) = app.screen.as_ref().expect("the key map").lines(&app.set);
  // the first row whose keys end so
  fn find<'a>(lines: &'a [L], keys: &str) -> Option<&'a L> {
    lines.iter().find(|l| matches!(l, L::Row { a, .. } | L::Tint { a, .. } if a.ends_with(keys)))
  }
  // vi off: the vi rows are dim tints and say vi; the plain ones do not
  assert!(matches!(find(&lines, "ctrl-w"), Some(L::Tint { a, ink, .. }) if a.starts_with("vi") && *ink == theme::DIM));
  assert!(lines.iter().any(|l| matches!(l, L::Head(h) if h == "a picked line · vi")), "a whole group says it once");
  assert!(lines.iter().any(|l| matches!(l, L::Head(h) if h == "chat")), "a mixed group keeps its name");
  assert!(
    matches!(find(&lines, "ctrl-f"), Some(L::Row { a, .. }) if a.starts_with(' ')),
    "plain rows line up under the vi tag"
  );
  // vi on: the same rows read like any other
  app.set.vi = true;
  app.redraw();
  dump(&app, "keys_vi_on");
  let (lines, _) = app.screen.as_ref().unwrap().lines(&app.set);
  assert!(matches!(find(&lines, "ctrl-w"), Some(L::Row { a, .. }) if a.starts_with("vi")));
  assert!(!lines.iter().any(|l| matches!(l, L::Tint { a, .. } if a.starts_with("vi "))), "no vi row is dim");
}

#[test]
fn the_empty_box_offers_log_in_as_a_button_while_logged_out() {
  let (mut app, _rx) = boot(&["forsen"], 420, 800);
  app.me = None;
  app.input_dirty = true;
  app.redraw();
  let r = app.login_link.expect("the hint has a button");
  assert!(app.input_area().contains(r.x, r.y) && app.hint().starts_with("ctrl-l log in"));
  dump(&app, "login_link");
  // the pointer on it reverses it; off it, it goes back
  app.pointer = mid(r);
  app.input_hover();
  app.redraw();
  assert!(app.hover.link);
  assert_eq!(px(&app, r.x + 1, r.y + 1), theme::WHITE, "hover is white");
  dump(&app, "login_link_hover");
  app.pointer = (r.x, r.y - 40);
  app.input_hover();
  assert!(!app.hover.link);
  // a click asks which platform
  app.pointer = mid(r);
  app.click();
  assert!(app.pop().is_some(), "the log in chooser");
  // typing something takes the hint (and the button) away; so does being logged in
  app.close_pop();
  app.me = Some(me(Some("mellen"), None));
  app.input_dirty = true;
  app.redraw();
  assert!(app.login_link.is_none(), "nothing to log in to");
}

#[test]
fn a_first_run_opens_the_add_channel_form_once() {
  let (mut app, _rx) = boot(&[], 420, 800);
  assert!(app.form.is_none(), "the window itself opens nothing");
  app.first_run();
  let f = app.form.as_ref().expect("the form is up");
  assert!(f.welcome && f.edit.is_none());
  app.redraw();
  dump(&app, "first_run");
  // esc leaves the plain empty window, which still says what to do; the form does not come back
  app.on_key(Key::Named(NamedKey::Escape), None);
  assert!(app.form.is_none());
  app.first_run();
  assert!(app.form.is_none(), "once per run");
  assert!(app.hint().contains("ctrl-t"));
  // any channel at all, and the window opens as it always did
  let (mut app, _rx) = boot(&["forsen"], 420, 800);
  app.first_run();
  assert!(app.form.is_none());
}

#[test]
fn login_logout_and_account_by_name() {
  let (mut app, mut rx) = boot(&["forsen"], 420, 800);
  let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(tab);
  // out: /logout has nothing to do, /login kick goes straight to the browser, /login alone asks
  typed(&mut app, "/logout");
  assert!(said(&app).contains("not logged in") && asked(&mut rx).is_empty());
  typed(&mut app, "/login kick");
  assert!(matches!(asked(&mut rx)[..], [Cmd::Login(Provider::Kick)]));
  typed(&mut app, "/login nope");
  assert!(said(&app).starts_with("usage: /login") && asked(&mut rx).is_empty());
  typed(&mut app, "/login");
  assert!(app.pop().is_some(), "the chooser");
  app.close_pop();
  // /account out: three ways in; the first picked row starts that login and the screen steps aside
  typed(&mut app, "/account");
  assert!(app.screen_is(Page::Account));
  app.redraw();
  dump(&app, "account_out");
  app.screen_key(Key::Character("j"), None);
  app.screen_key(Key::Named(NamedKey::Enter), None);
  assert!(app.screen.is_none());
  assert!(matches!(asked(&mut rx)[..], [Cmd::Login(Provider::Kick)]), "the second row is kick");
  // in: the account shows who you are; a platform you lack links, one you have says so, /logout goes
  app.me = Some(me(Some("mellen_tv"), None));
  typed(&mut app, "/login twitch");
  assert!(said(&app).contains("already linked as mellen_tv") && asked(&mut rx).is_empty());
  typed(&mut app, "/login kick");
  assert!(matches!(asked(&mut rx)[..], [Cmd::Link(Provider::Kick)]));
  typed(&mut app, "/account");
  app.redraw();
  dump(&app, "account_in");
  let s = app.screen.as_ref().unwrap();
  let (lines, marks) = s.lines(&app.set);
  assert!(lines.iter().any(|l| matches!(l, L::Row { a, b, .. } if a == "twitch" && b == "mellen_tv")));
  let link =
    marks.iter().position(|t| matches!(t, Target::Account(super::account::Step::Link(Provider::Kick)))).unwrap();
  assert!(matches!(lines[link], L::Row { .. }));
  // the last row is the way out
  app.screen_key(Key::Character("G"), None);
  app.screen_key(Key::Named(NamedKey::Enter), None);
  assert!(app.screen.is_none() && matches!(asked(&mut rx)[..], [Cmd::Logout]));
  typed(&mut app, "/logout");
  assert!(matches!(asked(&mut rx)[..], [Cmd::Logout]));
}

#[test]
fn open_adds_a_links_tabs_and_skips_what_is_open() {
  let (mut app, _rx) = boot(&["forsen"], 420, 800);
  let names = |a: &App| a.tabs.iter().filter(|t| !t.sources.is_empty()).map(|t| t.name.to_string()).collect::<Vec<_>>();
  typed(&mut app, "/open");
  assert!(said(&app).contains("paste a heatsync.org/m/ link"));
  typed(&mut app, "/open https://evil.example/m/t:xqc");
  assert!(said(&app).contains("isn't a heatsync.org/m/ link") && names(&app) == ["forsen"]);
  // forsen is open: only its kick leg is new; asmon is added; the youtube id can't open
  typed(&mut app, "/open https://heatsync.org/m/t:forsen,k:forsen+t:asmon+y:UC8yh-UI81uUteMDSOwQyx1g");
  assert!(said(&app).starts_with("added 2 tabs: kick:forsen, asmon"), "{}", said(&app));
  assert!(said(&app).contains("skipped 1"));
  assert_eq!(names(&app), ["forsen", "kick:forsen", "asmon"]);
  // the first new tab is the one you land on
  assert_eq!(&*app.tabs[app.active].name, "kick:forsen");
  // again: nothing new
  typed(&mut app, "/open t:forsen+t:asmon");
  assert_eq!(said(&app), "already open");
  assert_eq!(names(&app).len(), 3);
  // a pasted link after /join works too, and share has something to build
  typed(&mut app, "/join https://heatsync.org/m/t:xqc");
  assert_eq!(names(&app).len(), 4);
  typed(&mut app, "/share");
  assert!(!said(&app).contains("no channel tabs"), "{}", said(&app));
}
