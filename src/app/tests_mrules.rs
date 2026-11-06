// mention rules, headless: read at login, kept in step by the server's push, listed / added / switched /
// removed from the settings list screen, and a server hit is a row of the mentions tab like any ping
use super::lists::Kind as ListKind;
use super::screen::{Page, Target};
use super::*;
use crate::model::message::{Message, flag};
use crate::model::mrules::{Rule, Ty};
use crate::net::mrules::{Fail, Got, Op};
use crate::net::{Cmd, social};
use crate::ui::sheet::L;
use serde_json::json;
use winit::keyboard::{Key, NamedKey};

type Asked = tokio::sync::mpsc::UnboundedReceiver<Cmd>;

fn me() -> crate::net::auth::Me {
  crate::net::auth::Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: None,
    twitch_id: Some(77),
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  }
}

fn boot() -> (App, Asked) {
  let (net, asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  // a test never pings or notifies
  app.focused = true;
  (app, asked)
}

fn sent(asked: &mut Asked) -> Vec<Cmd> {
  std::iter::from_fn(|| asked.try_recv().ok()).collect()
}

// the mention-rule ops among what was sent
fn ops(asked: &mut Asked) -> Vec<Op> {
  sent(asked)
    .into_iter()
    .filter_map(|c| if let Cmd::Social(social::Cmd::MentionRules(op)) = c { Some(op) } else { None })
    .collect()
}

fn rule(id: u64, pattern: &str) -> Rule {
  Rule { id, ty: Ty::Word, pattern: pattern.into(), channels: vec![], platforms: vec![], on: true, dormant: false }
}

fn login(app: &mut App) {
  app.on_net(NetEvent::Me(Some(me())));
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

fn tip(app: &App) -> String {
  app.screen.as_ref().and_then(|s| s.ls.as_deref()).map(|l| l.tip().to_owned()).unwrap_or_default()
}

fn rows(app: &App) -> Vec<String> {
  let s = app.screen.as_ref().unwrap();
  let (lines, _) = s.lines(&app.set);
  lines.into_iter().filter_map(|l| if let L::Row { a, b, .. } = l { Some(format!("{a}|{b}")) } else { None }).collect()
}

// settings → the row → enter, the way a person gets there
fn via_settings(app: &mut App) {
  app.open_screen(Page::Settings);
  let s = app.screen.as_mut().unwrap();
  let (lines, targets) = s.lines(&app.set);
  let at = targets
    .iter()
    .position(|t| matches!(t, Target::Def(d) if d.id == "mention_rules"))
    .expect("a mention rules row in settings");
  s.st.at = at;
  s.st.settle(&lines);
  app.on_key(Key::Named(NamedKey::Enter), None);
}

fn frame(app: &mut App, v: serde_json::Value) {
  let (tx, rx) = std::sync::mpsc::channel();
  crate::net::hs_ws::feed_frame(&v, &crate::net::Out::Pipe(tx));
  for ev in rx.try_iter() {
    app.on_net(ev);
  }
}

fn mention_rows(app: &App) -> Vec<(String, String, u8, String)> {
  let t = app.tabs.iter().find(|t| t.kind == Kind::Mentions).unwrap();
  t.view
    .msgs()
    .map(|m| {
      (m.login.to_string(), m.body.to_string(), m.flags, m.x.as_ref().map_or(String::new(), |x| x.chan.to_string()))
    })
    .collect()
}

fn hit(user: &str, snippet: &str) -> serde_json::Value {
  json!({"type": "mention:rule-match", "data": {"ruleId": 1, "pattern": "gg", "channel": "forsen",
    "platform": "twitch", "username": user, "snippet": snippet}})
}

#[test]
fn rules_are_read_at_login_and_dropped_at_logout() {
  let (mut app, mut asked) = boot();
  login(&mut app);
  assert!(matches!(ops(&mut asked).as_slice(), [Op::List]), "asked once at login");
  app.on_net(NetEvent::MentionRules(Ok(Got::List(vec![rule(1, "gg"), rule(2, "hype")]))));
  assert_eq!(app.mrules.len(), 2);
  // another device edited them: the push asks again, once however loud
  for _ in 0..50 {
    frame(&mut app, json!({"type": "mention-rules:changed"}));
  }
  assert_eq!(ops(&mut asked).len(), 1, "one ask at once, the rest wait for the gap");
  assert!(app.pushed_due().is_some());
  // an answer that outlived its login seeds nothing
  app.on_net(NetEvent::Me(None));
  assert!(app.mrules.is_empty());
  app.on_net(NetEvent::MentionRules(Ok(Got::List(vec![rule(3, "late")]))));
  assert!(app.mrules.is_empty());
  // logged out, a push asks nothing
  frame(&mut app, json!({"type": "mention-rules:changed"}));
  assert!(ops(&mut asked).is_empty());
}

#[test]
fn the_list_screen_shows_the_rules_and_says_when_it_cannot() {
  let (mut app, mut asked) = boot();
  via_settings(&mut app);
  assert_eq!(app.screen.as_ref().unwrap().page(), Page::List);
  assert_eq!(rows(&app), ["log in to keep mention rules|", "add…|a"]);
  assert!(ops(&mut asked).is_empty(), "signed out: nothing is asked");
  app.close_screen();
  login(&mut app);
  ops(&mut asked);
  via_settings(&mut app);
  assert!(matches!(ops(&mut asked).as_slice(), [Op::List]), "opening it reads the list fresh");
  assert_eq!(rows(&app), ["reading your rules…|", "add…|a"]);
  let mut forsen = rule(2, "big news");
  (forsen.ty, forsen.channels) = (Ty::Phrase, vec!["forsen".into()]);
  app.on_net(NetEvent::MentionRules(Ok(Got::List(vec![rule(1, "gg"), forsen]))));
  assert_eq!(rows(&app), ["gg|word · everywhere", "big news|phrase · #forsen", "add…|a"]);
  // a failure is said; the list stays
  app.on_net(NetEvent::MentionRules(Err(Fail::Other)));
  assert_eq!(rows(&app).len(), 3);
}

#[test]
fn a_adds_enter_switches_x_removes() {
  let (mut app, mut asked) = boot();
  login(&mut app);
  app.on_net(NetEvent::MentionRules(Ok(Got::List(vec![rule(1, "gg"), rule(2, "hype")]))));
  via_settings(&mut app);
  ops(&mut asked);
  // add: what is typed becomes the server's body
  press(&mut app, "a");
  type_in(&mut app, "phrase big news #Forsen");
  app.on_key(Key::Named(NamedKey::Enter), None);
  let got = ops(&mut asked);
  assert!(
    matches!(got.as_slice(), [Op::Add(b)] if b.ty == Ty::Phrase && b.pattern == "big news" && b.channels == ["forsen"]),
    "{}",
    got.len()
  );
  app.on_key(Key::Named(NamedKey::Escape), None);
  // enter on a row switches it: the row shows at once, the server is told
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert!(matches!(ops(&mut asked).as_slice(), [Op::Toggle { id: 1, on: false }]));
  assert_eq!(rows(&app)[0], "gg|word · everywhere · off");
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert!(matches!(ops(&mut asked).as_slice(), [Op::Toggle { id: 1, on: true }]));
  // x removes the picked one
  press(&mut app, "x");
  assert!(matches!(ops(&mut asked).as_slice(), [Op::Remove(1)]));
  assert_eq!(rows(&app), ["hype|word · everywhere", "add…|a"]);
}

#[test]
fn a_typed_rule_the_server_would_refuse_never_leaves_and_a_signed_out_add_says_why() {
  let (mut app, mut asked) = boot();
  app.list_open(ListKind::Alerts);
  press(&mut app, "a");
  type_in(&mut app, "gg");
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert!(ops(&mut asked).is_empty(), "signed out: nothing is sent");
  assert!(tip(&app).contains("log in"), "{}", tip(&app));
  login(&mut app);
  ops(&mut asked);
  app.on_net(NetEvent::MentionRules(Ok(Got::List(vec![]))));
  let long = "x".repeat(300);
  type_in(&mut app, &long);
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert!(ops(&mut asked).is_empty(), "too long: said, not sent");
  assert!(tip(&app).contains("too long"));
}

#[test]
fn a_change_settles_with_a_fresh_read_and_a_refusal_is_said() {
  let (mut app, mut asked) = boot();
  login(&mut app);
  ops(&mut asked);
  app.on_net(NetEvent::MentionRules(Ok(Got::Changed)));
  assert!(matches!(ops(&mut asked).as_slice(), [Op::List]));
  app.list_open(ListKind::Alerts);
  ops(&mut asked);
  app.on_net(NetEvent::MentionRules(Err(Fail::Said("free accounts run 5 alerts; plus runs 50".into()))));
  assert!(tip(&app).contains("free accounts run 5"));
}

#[test]
fn a_server_hit_is_a_mention_row() {
  let (mut app, _asked) = boot();
  // signed out: nothing
  frame(&mut app, hit("bob", "gg wp"));
  assert!(mention_rows(&app).is_empty());
  login(&mut app);
  frame(&mut app, hit("Bob", "gg wp"));
  let got = mention_rows(&app);
  assert_eq!(got.len(), 1);
  assert_eq!((got[0].0.as_str(), got[0].1.as_str(), got[0].3.as_str()), ("bob", "gg wp", "forsen"));
  assert!(got[0].2 & flag::ALERT != 0, "it wears the mention look");
  let m = app.tabs.iter().position(|t| t.kind == Kind::Mentions).unwrap();
  assert!(app.tabs[m].unread, "it lights the tab");
  // the same line again within moments (the server re-evaluating) is one row
  frame(&mut app, hit("bob", "gg wp"));
  assert_eq!(mention_rows(&app).len(), 1);
  // a different line is another
  frame(&mut app, hit("bob", "gg again"));
  assert_eq!(mention_rows(&app).len(), 2);
  // a blocked person never pings you, rules or not
  app.block("troll");
  frame(&mut app, hit("troll", "gg"));
  assert_eq!(mention_rows(&app).len(), 2);
  // junk frames add nothing
  frame(&mut app, json!({"type": "mention:rule-match", "data": {"channel": "a b"}}));
  frame(&mut app, json!({"type": "mention:rule-match", "data": null}));
  assert_eq!(mention_rows(&app).len(), 2);
}

#[test]
fn a_line_that_already_pinged_you_is_not_a_second_row_for_its_rule() {
  let (mut app, _asked) = boot();
  login(&mut app);
  let mut m = Message::system(now_ms(), Platform::Twitch, "");
  (m.kind, m.id, m.login, m.nick) = (crate::model::message::Kind::Chat, "1".into(), "bob".into(), "bob".into());
  m.body = "gg @mellen wp".into();
  app.scan_mention(Platform::Twitch, "forsen", &mut m);
  assert_eq!(mention_rows(&app).len(), 1);
  // the server's snippet may be cut short or stripped: the same line all the same
  frame(&mut app, hit("bob", "gg @mellen"));
  assert_eq!(mention_rows(&app).len(), 1, "one line, one row");
  // the other way round: the rule's row first, then the irc line
  let mut n = Message::system(now_ms(), Platform::Twitch, "");
  (n.kind, n.id, n.login, n.nick) = (crate::model::message::Kind::Chat, "2".into(), "al".into(), "al".into());
  n.body = "@mellen gg".into();
  frame(&mut app, hit("al", "@mellen gg"));
  assert_eq!(mention_rows(&app).len(), 2);
  app.scan_mention(Platform::Twitch, "forsen", &mut n);
  assert_eq!(mention_rows(&app).len(), 2, "still one row for it");
  assert!(n.flags & flag::ALERT != 0, "the line itself is still tinted in its tab");
}
