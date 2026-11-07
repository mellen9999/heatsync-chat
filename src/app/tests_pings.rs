// per-channel notification rules, headless: the go-live bell (default / on / off) and muted pings,
// reachable from the tab menu, kept between runs
use super::*;
use crate::model::message::Message;
use crate::model::pings::Bell;
use crate::model::stream_events::Line;
use winit::keyboard::{Key as WKey, NamedKey};

fn app() -> App {
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["forsen".into()], false);
  a.dpi = 1.0;
  a.res.set_scale(a.scale());
  a.resized(720, 400);
  a.mentions.set(["mellen"]);
  a.set.mention_volume = crate::model::settings::Volume::V0;
  a.set.title_flash = true;
  a.focused = false;
  a
}

fn fresh() {
  let _ = std::fs::remove_dir_all(store::folder());
}

fn ping(a: &mut App, room: &str) {
  let mut m = Message::system(1, Platform::Twitch, "x");
  (m.kind, m.login, m.nick, m.body) =
    (crate::model::message::Kind::Chat, "bob".into(), "bob".into(), "hi @mellen".into());
  a.scan_mention(Platform::Twitch, room, &mut m);
}

fn menu(a: &mut App) -> Vec<String> {
  let i = a.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  a.tabs[i].event = false;
  a.tabs[i].unread = false;
  a.close_tab_menu();
  a.bar_dirty = true;
  a.redraw();
  let p = a.bar_map.iter().position(|&t| t == i).expect("in the bar");
  let r = a.bar_hits[p];
  a.pointer = (r.x + r.w / 2, r.y + r.h / 2);
  a.right_click();
  a.tab_menu().map(|m| m.labels()).unwrap_or_default()
}

fn pick(a: &mut App, label: &str) {
  let labels = menu(a);
  let at = labels.iter().position(|l| l == label).unwrap_or_else(|| panic!("{label} in {labels:?}"));
  for _ in 0..at {
    a.tab_menu_key(WKey::Character("j"));
  }
  a.tab_menu_key(WKey::Named(NamedKey::Enter));
}

#[test]
fn the_tab_menu_has_a_bell_row_and_a_mute_row_and_they_stick() {
  fresh();
  let mut a = app();
  let m = menu(&mut a);
  assert!(m.contains(&"bell: default".to_string()) && m.contains(&"mute pings".to_string()), "{m:?}");
  pick(&mut a, "bell: default");
  assert_eq!(a.pings.bell(Platform::Twitch, "forsen"), Bell::On);
  assert!(menu(&mut a).contains(&"bell: on".to_string()));
  pick(&mut a, "bell: on");
  assert_eq!(a.pings.bell(Platform::Twitch, "forsen"), Bell::Off);
  pick(&mut a, "mute pings");
  assert!(a.pings.muted(Platform::Twitch, "Forsen"));
  assert!(menu(&mut a).contains(&"unmute pings".to_string()));
  // a restart keeps both
  let b = app();
  assert_eq!(b.pings.bell(Platform::Twitch, "forsen"), Bell::Off);
  assert!(b.pings.muted(Platform::Twitch, "forsen"));
}

#[test]
fn a_muted_tab_still_gets_the_mention_but_no_flash() {
  fresh();
  let mut a = app();
  ping(&mut a, "forsen");
  assert!(a.flash.is_some(), "away and not muted: the title flashes");
  a.flash = None;
  a.pings.set_mute(Platform::Twitch, "forsen", true);
  ping(&mut a, "forsen");
  assert!(a.flash.is_none(), "muted: silent");
  let t = a.tabs.iter().find(|t| t.kind == Kind::Mentions).unwrap();
  assert_eq!(t.view.msgs().count(), 2, "both landed in the mentions tab");
  ping(&mut a, "other");
  assert!(a.flash.is_some(), "another room still pings");
}

// the taskbar entry lights up for a mention while the window is away, through the one door the ping and the
// popup use, so it holds the same rules: not while you look, not for a muted channel, not while streamer mode
// holds sounds back, not when its own switch is off, and once per time away
#[test]
fn a_mention_while_away_lights_the_taskbar_by_the_ping_rules() {
  use crate::model::schema::Fx;
  use crate::model::settings::Streamer as Mode;
  fresh();
  let mut a = app();
  assert!(a.set.taskbar_flash, "on by default");
  a.focused = true;
  ping(&mut a, "forsen");
  assert!(!a.attention, "you are looking: nothing to light");
  a.focused = false;
  a.pings.set_mute(Platform::Twitch, "forsen", true);
  ping(&mut a, "forsen");
  assert!(!a.attention, "a muted channel stays dark");
  a.pings.set_mute(Platform::Twitch, "forsen", false);
  a.set.taskbar_flash = false;
  ping(&mut a, "forsen");
  assert!(!a.attention, "its own switch is off");
  a.set.taskbar_flash = true;
  a.set.streamer = Mode::On;
  a.applied(Fx::Streamer);
  ping(&mut a, "forsen");
  assert!(!a.attention, "streamer mode holds it like the sound");
  a.set.streamer_sounds = true;
  ping(&mut a, "forsen");
  assert!(a.attention, "keep sounds lets the ping and the flash through");
  // coming back puts it down, and the next time away lights it again
  a.attention = false;
  a.set.streamer = Mode::Off;
  a.applied(Fx::Streamer);
  let mut m = Message::system(1, Platform::Twitch, "x");
  (m.kind, m.login, m.nick, m.body) =
    (crate::model::message::Kind::Chat, "bob".into(), "bob".into(), "hi @mellen".into());
  m.flags |= crate::model::message::flag::HISTORY;
  a.scan_mention(Platform::Twitch, "forsen", &mut m);
  assert!(!a.attention, "backlog is not a new ping");
  ping(&mut a, "forsen");
  assert!(a.attention);
  a.on_window_focus(true);
  assert!(!a.attention, "looking at it again puts the flag down");
  a.on_window_focus(false);
  ping(&mut a, "forsen");
  assert!(a.attention, "away again: lit again");
}

#[test]
fn the_bell_decides_a_go_live_popup_per_channel() {
  fresh();
  let mut a = app();
  a.set.notify = false;
  let live = |n: &str| Line::went_live(Platform::Twitch, n, "chess");
  assert!(a.stream_alert(&live("xqc")).is_none(), "default follows the global switch");
  a.pings.set_bell(Platform::Twitch, "xqc", Bell::On);
  assert!(a.stream_alert(&live("xqc")).is_some(), "on: this channel always tells you");
  a.set.notify = true;
  a.pings.set_bell(Platform::Twitch, "xqc", Bell::Off);
  assert!(a.stream_alert(&live("xqc")).is_none(), "off: never");
  assert!(a.stream_alert(&live("pokimane")).is_some(), "others follow the global switch");
  a.focused = true;
  a.pings.set_bell(Platform::Twitch, "xqc", Bell::On);
  assert!(a.stream_alert(&live("xqc")).is_none(), "you are looking at the window");
}

#[test]
fn a_muted_tab_wears_a_mark() {
  fresh();
  let mut a = app();
  let i = a.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  assert!(!a.tab_muted(i));
  a.pings.set_mute(Platform::Twitch, "forsen", true);
  assert!(a.tab_muted(i));
}
