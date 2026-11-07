// the per-tab platform filter, headless: the tab menu's `show twitch: on` rows (merged tabs only, only the
// feeds the tab has), lines of a hidden platform neither drawn nor pinging nor lighting the tab, the choice
// kept between runs, and the mark on the tab
use super::*;
use crate::model::message::{Kind as MsgKind, Message};
use crate::net::irc::Chat;
use winit::keyboard::{Key as WKey, NamedKey};

fn app_of(tabs: &[&str]) -> App {
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, tabs.iter().map(|t| (*t).into()).collect(), false);
  a.dpi = 1.0;
  a.res.set_scale(a.scale());
  a.resized(720, 400);
  a.mentions.set(["mellen"]);
  a.set.mention_volume = crate::model::settings::Volume::V0;
  a.set.title_flash = true;
  a.focused = false;
  a
}

fn app() -> App {
  let _ = std::fs::remove_dir_all(store::folder());
  app_of(&["forsen kick:xqc", "pokimane"])
}

fn said(p: Platform, who: &str, body: &str) -> Message {
  let mut m = Message::system(now_ms(), p, "x");
  (m.kind, m.login, m.nick, m.body) = (MsgKind::Chat, who.into(), who.into(), body.into());
  m
}

fn say_twitch(a: &mut App, who: &str, body: &str) {
  a.on_chat("forsen", Chat::Msg(said(Platform::Twitch, who, body)));
}

fn say_kick(a: &mut App, who: &str, body: &str) {
  a.on_relay(Platform::Kick, "xqc", vec![said(Platform::Kick, who, body)], false);
}

// the merged tab's place (the built-in tabs come first)
fn at(a: &App, name: &str) -> usize {
  a.tabs.iter().position(|t| &*t.name == name).expect(name)
}

fn shown(a: &App) -> usize {
  a.tabs[at(a, "forsen")].view.found()
}

fn menu(a: &mut App, tab: usize) -> Vec<String> {
  a.close_tab_menu();
  a.tabs[tab].event = false;
  a.tabs[tab].unread = false;
  a.bar_dirty = true;
  a.redraw();
  let p = a.bar_map.iter().position(|&t| t == tab).expect("in the bar");
  let r = a.bar_hits[p];
  a.pointer = (r.x + r.w / 2, r.y + r.h / 2);
  a.right_click();
  a.tab_menu().map(|m| m.labels()).unwrap_or_default()
}

fn pick(a: &mut App, label: &str) {
  let labels = menu(a, at(a, "forsen"));
  let at = labels.iter().position(|l| l == label).unwrap_or_else(|| panic!("{label} in {labels:?}"));
  for _ in 0..at {
    a.tab_menu_key(WKey::Character("j"));
  }
  a.tab_menu_key(WKey::Named(NamedKey::Enter));
}

#[test]
fn the_menu_offers_a_show_row_per_feed_on_a_merged_tab_only() {
  let mut a = app();
  let m = {
    let i = at(&a, "forsen");
    menu(&mut a, i)
  };
  assert!(m.contains(&"show twitch: on".to_string()) && m.contains(&"show kick: on".to_string()), "{m:?}");
  assert!(!m.iter().any(|l| l.contains("youtube")), "a feed the tab lacks has no row: {m:?}");
  let single = {
    let i = at(&a, "pokimane");
    menu(&mut a, i)
  };
  assert!(!single.iter().any(|l| l.starts_with("show ")), "a one-feed tab has no filter: {single:?}");
}

#[test]
fn a_hidden_platform_is_not_drawn_and_comes_back() {
  let mut a = app();
  say_twitch(&mut a, "ann", "from twitch");
  say_kick(&mut a, "bob", "from kick");
  assert_eq!(shown(&a), 2);
  pick(&mut a, "show kick: on");
  assert_eq!(shown(&a), 1, "the kick line is gone from the tab");
  assert!(
    {
      let i = at(&a, "forsen");
      menu(&mut a, i)
    }
    .contains(&"show kick: off".to_string())
  );
  say_kick(&mut a, "bob", "more kick");
  say_twitch(&mut a, "ann", "more twitch");
  assert_eq!(shown(&a), 2, "new kick lines stay out, twitch stays");
  pick(&mut a, "show kick: off");
  assert_eq!(shown(&a), 4, "nothing was deleted: all four are back");
}

#[test]
fn notices_are_never_filtered() {
  let mut a = app();
  pick(&mut a, "show kick: on");
  let f = at(&a, "forsen");
  a.tabs[f].view.push(Message::system(now_ms(), Platform::Kick, "somebody was timed out"));
  assert!(a.tabs[at(&a, "forsen")].view.msgs().any(|m| &*m.body == "somebody was timed out"));
  assert_eq!(a.tabs[at(&a, "forsen")].view.len(), 1);
}

#[test]
fn the_last_feed_showing_cannot_be_hidden() {
  let mut a = app();
  pick(&mut a, "show kick: on");
  pick(&mut a, "show twitch: on");
  let m = {
    let i = at(&a, "forsen");
    menu(&mut a, i)
  };
  assert!(m.contains(&"show twitch: on".to_string()), "twitch is still on: {m:?}");
  assert!(a.tabs[at(&a, "forsen")].view.msgs().any(|m| m.body.contains("at least one")), "it says why");
}

#[test]
fn a_hidden_line_does_not_ping_or_light_the_tab() {
  let mut a = app();
  pick(&mut a, "show kick: on");
  a.close_tab_menu();
  a.active = at(&a, "pokimane");
  let f = at(&a, "forsen");
  a.tabs[f].unread = false;
  a.flash = None;
  say_kick(&mut a, "bob", "hey @mellen");
  assert!(a.flash.is_none(), "no title flash");
  assert!(!a.tabs[f].unread, "the tab stays quiet");
  let mentions = a.tabs.iter().find(|t| t.kind == Kind::Mentions).unwrap();
  assert_eq!(mentions.view.msgs().count(), 0, "nothing in the mentions tab");
  say_twitch(&mut a, "ann", "hey @mellen");
  assert!(a.flash.is_some() && a.tabs[f].unread, "a shown platform still pings");
}

#[test]
fn the_choice_is_kept_between_runs_and_wears_a_mark() {
  let mut a = app();
  assert!(!a.tab_filtered(at(&a, "forsen")));
  pick(&mut a, "show kick: on");
  assert!(a.tab_filtered(at(&a, "forsen")) && !a.tab_filtered(at(&a, "pokimane")));
  let mut b = app_of(&["forsen kick:xqc", "pokimane"]);
  assert!(b.tab_filtered(at(&a, "forsen")));
  say_kick(&mut b, "bob", "hi");
  assert_eq!(shown(&b), 0, "a restart keeps it");
}

fn rows(a: &App, r: crate::ui::canvas::Rect) -> Vec<u32> {
  let w = a.canvas.w as usize;
  a.canvas.px[r.y as usize * w..(r.y + r.h) as usize * w].to_vec()
}

// the bar draws a mark after a filtered tab's name: same bar, filter off, different pixels
#[test]
fn the_filtered_tab_is_marked_in_the_bar() {
  let mut a = app();
  a.redraw();
  let bar = a.bar();
  let plain = rows(&a, bar);
  pick(&mut a, "show kick: on");
  a.close_tab_menu();
  a.bar_dirty = true;
  a.redraw();
  assert_ne!(plain, rows(&a, bar), "the mark shows on the tab");
}

#[test]
fn an_unreadable_file_hides_nothing() {
  let _ = std::fs::remove_dir_all(store::folder());
  let _ = store::write("platforms", "forsen kick:xqc\t7\n\0\0garbage\nforsen kick:xqc\tzz\n");
  let mut a = app_of(&["forsen kick:xqc"]);
  say_kick(&mut a, "bob", "hi");
  assert_eq!(shown(&a), 1);
  assert!(!a.tab_filtered(at(&a, "forsen")));
}

#[test]
fn editing_a_tab_keeps_what_it_hid_for_the_feeds_that_stay() {
  let mut a = app();
  pick(&mut a, "show kick: on");
  a.close_tab_menu();
  let spec = crate::model::chan::Spec::from_line("forsen kick:xqc yt:@abcdef").unwrap();
  let f = at(&a, "forsen");
  a.edit_tab(f, &spec);
  assert!(a.tab_filtered(at(&a, "forsen")));
  assert!(
    {
      let i = at(&a, "forsen");
      menu(&mut a, i)
    }
    .contains(&"show kick: off".to_string())
  );
}
