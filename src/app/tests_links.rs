// auto-multichat asks for a twitch channel's linked kick/youtube once, when you make the tab. a restored or
// edited tab is what you made it: a leg you took off stays off, and sends stop going to it. and a merged send
// leaves out a feed your account isn't on, instead of a refusal line under every message

use super::*;
use crate::model::chan::Spec;
use crate::net::Cmd;

fn boot(saved: &[&str]) -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, rx) = Net::tap();
  let mut app = App::new(net, saved.iter().map(|s| (*s).into()).collect(), false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 300);
  app.redraw();
  (app, rx)
}

fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Box<str>> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Links(ch) => Some(ch),
      _ => None,
    })
    .collect()
}

fn at(app: &App, name: &str) -> usize {
  app.tabs.iter().position(|t| &*t.name == name).unwrap()
}

fn kicks(app: &App, i: usize) -> Vec<&str> {
  app.tabs[i].sources.iter().filter(|s| s.0 == Platform::Kick).map(|s| &*s.1).collect()
}

#[test]
fn a_restored_tab_is_not_asked_again() {
  let (_app, mut rx) = boot(&["forsen", "xqc kick:xqc"]);
  assert!(asked(&mut rx).is_empty(), "boot lays out what you saved, it does not re-merge it");
}

#[test]
fn a_new_twitch_tab_asks_once() {
  let (mut app, mut rx) = boot(&["forsen"]);
  let _ = asked(&mut rx);
  app.add_tab(&Spec::from_line("xqc").unwrap());
  assert_eq!(asked(&mut rx), [Box::from("xqc")]);
  // every feed already named: nothing to ask
  app.add_tab(&Spec::from_line("big kick:big yt:@big").unwrap());
  assert!(asked(&mut rx).is_empty());
}

#[test]
fn the_answer_fills_the_tab_and_is_saved() {
  let (mut app, mut rx) = boot(&["forsen"]);
  app.add_tab(&Spec::from_line("xqc").unwrap());
  let _ = asked(&mut rx);
  app.on_links("xqc", Some("xqc".into()), None);
  let i = at(&app, "xqc");
  assert_eq!(kicks(&app, i), ["xqc"]);
  let lines: Vec<String> = crate::store::load_tabs().iter().map(|l| l.to_string()).collect();
  assert!(lines.iter().any(|l| l.contains("kick:xqc")), "the leg is the tab's now: {lines:?}");
}

#[test]
fn a_kick_leg_taken_off_stays_off() {
  let (mut app, mut rx) = boot(&["xqc kick:xqc"]);
  let i = at(&app, "xqc");
  app.edit_tab(i, &Spec::from_line("xqc").unwrap());
  assert!(asked(&mut rx).is_empty(), "an edit is not a new tab");
  assert!(kicks(&app, i).is_empty());
  app.activate(i);
  let legs: Vec<Platform> = app.legs().iter().map(|l| l.0).collect();
  assert_eq!(legs, [Platform::Twitch], "nothing is sent to kick");
}

#[test]
fn a_late_answer_never_adds_a_second_kick_leg() {
  let (mut app, mut rx) = boot(&["forsen"]);
  app.add_tab(&Spec::from_line("xqc kick:other").unwrap());
  let _ = asked(&mut rx);
  app.on_links("xqc", Some("xqc".into()), None);
  assert_eq!(kicks(&app, at(&app, "xqc")), ["other"], "the kick you named is the tab's");
}

fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Platform> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Send { platform, .. } => Some(platform),
      _ => None,
    })
    .collect()
}

fn twitch_only(app: &mut App) {
  app.me = Some(crate::net::auth::Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: Some(1),
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  });
  app.hs_authed = true;
}

#[test]
fn a_merged_send_leaves_out_a_feed_you_are_not_on() {
  let (mut app, mut rx) = boot(&["xqc kick:xqc"]);
  twitch_only(&mut app);
  app.activate(at(&app, "xqc"));
  let _ = sent(&mut rx);
  app.line.set_text("hello".into());
  app.send();
  assert_eq!(sent(&mut rx), [Platform::Twitch], "no kick leg to bounce");
}

#[test]
fn a_feed_sent_to_alone_still_goes_so_the_reason_shows() {
  let (mut app, mut rx) = boot(&["kick:xqc"]);
  twitch_only(&mut app);
  let i = app.tabs.iter().position(|t| t.sources == [(Platform::Kick, "xqc".into())]).unwrap();
  app.activate(i);
  let _ = sent(&mut rx);
  app.line.set_text("hello".into());
  app.send();
  assert_eq!(sent(&mut rx), [Platform::Kick]);
}
