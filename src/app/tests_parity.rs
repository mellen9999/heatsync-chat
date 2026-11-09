// ext parity, headless: the rule list's edit / notify / sound / volume keys with the live count, a notify rule's
// line in the mentions tab, the mention sound switch, the mod log across a restart, the starter mod buttons and
// the sent history across a restart (the scratch store keeps every test off the real config)
use super::lists::Kind as ListKind;
use super::*;
use crate::model::filter;
use crate::model::message::{Kind as MsgKind, Message};
use crate::net::auth::Me;
use crate::net::irc::Chat;
use serde_json::json;
use winit::keyboard::{Key, NamedKey};

fn me(name: &str) -> Me {
  Me {
    username: name.into(),
    platform: "twitch".into(),
    twitch: Some(name.into()),
    twitch_id: Some(1),
    names: vec![name.to_lowercase().into()],
    kick: None,
    youtube: None,
    packs: vec![],
  }
}

fn boot() -> App {
  let (net, _rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  app.focused = true;
  app.set.mention_volume = crate::model::settings::Volume::V0;
  app
}

fn fresh() {
  let _ = std::fs::remove_dir_all(store::folder());
  filter::edit(|f| *f = Default::default());
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
  app.screen.as_ref().unwrap().ls.as_ref().unwrap().tip().to_owned()
}

fn rows(app: &App) -> Vec<String> {
  use crate::ui::sheet::L;
  let s = app.screen.as_ref().unwrap();
  let (lines, _) = s.lines(&app.set);
  lines.into_iter().filter_map(|l| if let L::Row { a, b, .. } = l { Some(format!("{a}|{b}")) } else { None }).collect()
}

fn said(body: &str, login: &str) -> Message {
  let mut m = Message::system(crate::util::time::now_ms(), Platform::Twitch, "");
  (m.kind, m.login, m.nick, m.body) = (MsgKind::Chat, login.into(), login.into(), body.into());
  m
}

fn raw_rules() -> Vec<serde_json::Value> {
  serde_json::from_str(&filter::with(|f| f.rules().json())).unwrap()
}

fn set_rules(v: serde_json::Value) {
  assert!(filter::edit(|f| f.set_rules_text(&v.to_string())));
}

fn hl(id: &str, value: &str) -> serde_json::Value {
  json!({"id": id, "enabled": true, "scope": "all", "action": "highlight", "color": "#00ff00",
    "match": {"type": "keyword", "value": value, "caseSensitive": false}})
}

// ---- filter rules: edit in place, the live count, the keys on a row

#[test]
fn e_rewrites_the_picked_rule_in_place_and_counts_what_it_would_catch() {
  fresh();
  let mut app = boot();
  set_rules(json!([hl("a", "one"), hl("b", "two"), hl("c", "three")]));
  let t = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  for (i, b) in ["gg wp", "gg", "no", "so gg"].iter().enumerate() {
    let mut m = said(b, "bob");
    m.ts = i as i64 + 1;
    app.tabs[t].view.push(m);
  }
  app.select(t);
  app.list_open(ListKind::Rules);
  press(&mut app, "j");
  press(&mut app, "e");
  assert!(app.screen.as_ref().unwrap().typing(), "the box is open");
  assert_eq!(app.screen.as_ref().unwrap().ls.as_ref().unwrap().add.text, "highlight keyword two");
  assert!(tip(&app).starts_with("matches none of the last "), "{}", tip(&app));
  // rewriting the words moves the count as you type
  for _ in 0..3 {
    app.on_key(Key::Named(NamedKey::Backspace), None);
  }
  type_in(&mut app, "gg");
  assert!(tip(&app).starts_with("matches 3 of the last "), "{}", tip(&app));
  type_in(&mut app, " (");
  type_in(&mut app, "x");
  app.on_key(Key::Named(NamedKey::Enter), None);
  // a keyword rule with those words is fine; it saved in place
  let r = raw_rules();
  assert_eq!(r.len(), 3);
  assert_eq!((r[1]["id"].as_str(), r[1]["match"]["value"].as_str()), (Some("b"), Some("gg (x")));
  assert_eq!(r[0]["match"]["value"], "one");
  assert_eq!(r[2]["match"]["value"], "three");
  assert!(!app.screen.as_ref().unwrap().typing(), "saved: the box closes");
  assert_eq!(r[1]["color"], "#00ff00", "the colour stayed");
}

#[test]
fn a_bad_edit_stays_in_the_box_with_why_and_esc_leaves_the_rule_alone() {
  fresh();
  let mut app = boot();
  set_rules(json!([hl("a", "one")]));
  app.list_open(ListKind::Rules);
  press(&mut app, "e");
  app.screen.as_mut().unwrap().ls.as_mut().unwrap().add.set_text("highlight expr && &&".into());
  app.on_key(Key::Named(NamedKey::Enter), None);
  assert!(app.screen.as_ref().unwrap().typing(), "kept so it can be fixed");
  assert_eq!(raw_rules()[0]["match"]["value"], "one");
  assert!(!tip(&app).is_empty());
  app.on_key(Key::Named(NamedKey::Escape), None);
  assert!(!app.screen.as_ref().unwrap().typing());
  assert_eq!(raw_rules()[0]["match"]["value"], "one", "nothing changed");
  // the edit leaves the add box free for a new rule afterwards
  press(&mut app, "a");
  assert_eq!(app.screen.as_ref().unwrap().ls.as_ref().unwrap().editing, None);
}

#[test]
fn n_s_and_plus_minus_set_notify_the_sound_and_the_volume_on_a_highlight() {
  fresh();
  let mut app = boot();
  set_rules(json!([
    hl("a", "gg"),
    json!({"id": "h", "enabled": true, "scope": "all", "action": "hide",
    "match": {"type": "keyword", "value": "no", "caseSensitive": false}})
  ]));
  app.list_open(ListKind::Rules);
  press(&mut app, "n");
  assert_eq!(raw_rules()[0]["notify"], true);
  assert!(rows(&app)[0].ends_with("|notify"), "{:?}", rows(&app));
  press(&mut app, "n");
  assert!(raw_rules()[0].get("notify").is_none());
  // s walks off -> ping -> blip -> knock -> chime -> off
  let sound = || raw_rules()[0].get("sound").and_then(|s| s.as_str().map(str::to_owned));
  let mut seen = Vec::new();
  for _ in 0..5 {
    press(&mut app, "s");
    seen.push(sound());
  }
  assert_eq!(seen, [Some("ping".into()), Some("blip".into()), Some("knock".into()), Some("chime".into()), None]);
  // volume needs a sound; + steps ten from the shared volume
  press(&mut app, "+");
  assert!(raw_rules()[0].get("soundVolume").is_none(), "no sound, no volume");
  press(&mut app, "s");
  press(&mut app, "+");
  assert_eq!(raw_rules()[0]["soundVolume"], 10, "the shared volume is 0 here, plus ten");
  for _ in 0..20 {
    press(&mut app, "+");
  }
  assert_eq!(raw_rules()[0]["soundVolume"], 100, "capped");
  press(&mut app, "-");
  assert_eq!(raw_rules()[0]["soundVolume"], 90);
  assert!(rows(&app)[0].contains("\u{266a} ping 90"), "{:?}", rows(&app));
  // a hide rule has none of it
  press(&mut app, "j");
  press(&mut app, "n");
  assert!(raw_rules()[1].get("notify").is_none());
  assert_eq!(rows(&app)[1].split('|').nth(1), Some(""));
}

#[test]
fn rule_slash_commands_set_a_wav_per_rule_with_the_mention_files_bounds() {
  fresh();
  let mut app = boot();
  set_rules(json!([hl("a", "gg")]));
  // a good wav: 100 samples of silence
  let dir = store::scratch().join("wavs");
  std::fs::create_dir_all(&dir).unwrap();
  let good = dir.join("ding.wav");
  std::fs::write(&good, crate::util::wav::encode(8000, 1, &[0i16; 100])).unwrap();
  let bad = dir.join("bad.wav");
  std::fs::write(&bad, b"not a wav at all").unwrap();
  let run = |app: &mut App, c: &str| app.filter_cmd(crate::model::filtercmd::command(c).unwrap());
  run(&mut app, &format!("/rule sound 1 {}", bad.display()));
  assert!(raw_rules()[0].get("sound").is_none(), "a file that won't play is refused");
  run(&mut app, &format!("/rule sound 1 {}", good.display()));
  let r = &raw_rules()[0];
  assert_eq!((r["sound"].as_str(), r["soundName"].as_str()), (Some("custom"), Some("ding.wav")));
  assert!(!r.to_string().contains("wavs"), "the path never goes into the rule that syncs");
  assert_eq!(filter::with(|f| f.rule_file("a").map(str::to_owned)), Some(good.display().to_string()));
  run(&mut app, "/rule volume 1 40");
  let cue = filter::with(|f| f.rule_cue(&said("gg", "bob"), "forsen"));
  assert_eq!(cue.sound, Some((crate::util::sound::Voice::File(good.display().to_string()), Some(40))));
  // back to a preset: the file is forgotten here
  run(&mut app, "/rule sound 1 chime");
  assert_eq!(filter::with(|f| f.rule_file("a").map(str::to_owned)), None);
  run(&mut app, "/rule sound 1 off");
  assert!(raw_rules()[0].get("sound").is_none() && raw_rules()[0].get("soundVolume").is_none());
  // a rule that is not there
  run(&mut app, "/rule sound 9 chime");
  run(&mut app, "/rule volume 9 10");
}

// ---- a rule that notifies is a ping: a row in the mentions tab, once

#[test]
fn a_notify_rule_lands_the_line_in_the_mentions_tab() {
  fresh();
  let mut app = boot();
  let mentions = |app: &App| {
    let t = app.tabs.iter().find(|t| t.kind == Kind::Mentions).unwrap();
    t.view.msgs().map(|m| m.body.to_string()).collect::<Vec<_>>()
  };
  let mut hit = hl("a", "gg");
  set_rules(json!([hit.clone()]));
  let mut m = said("gg wp", "bob");
  app.scan_mention(Platform::Twitch, "forsen", &mut m);
  assert!(mentions(&app).is_empty(), "a plain highlight is not a ping");
  hit["notify"] = json!(true);
  set_rules(json!([hit]));
  let mut m = said("gg wp", "bob");
  app.scan_mention(Platform::Twitch, "forsen", &mut m);
  assert_eq!(mentions(&app), ["gg wp"]);
  // not for backlog, your own lines, or a blocked person
  let mut old = said("gg old", "bob");
  old.flags |= crate::model::message::flag::HISTORY;
  app.scan_mention(Platform::Twitch, "forsen", &mut old);
  let mut mine = said("gg mine", "bob");
  mine.flags |= crate::model::message::flag::OWN;
  app.scan_mention(Platform::Twitch, "forsen", &mut mine);
  filter::edit(|f| f.block("troll"));
  let mut troll = said("gg troll", "troll");
  app.scan_mention(Platform::Twitch, "forsen", &mut troll);
  assert_eq!(mentions(&app), ["gg wp"]);
  // a line another rule hides never pings
  let both = json!([{"id": "h", "enabled": true, "scope": "all", "action": "hide", "match": {"type": "keyword", "value": "gg"}},
    {"id": "a", "enabled": true, "scope": "all", "action": "highlight", "notify": true, "match": {"type": "keyword", "value": "gg"}}]);
  set_rules(both);
  let mut hidden = said("gg hidden", "bob");
  app.scan_mention(Platform::Twitch, "forsen", &mut hidden);
  assert_eq!(mentions(&app), ["gg wp"]);
}

// ---- the mention sound switch: always also sounds while you look; the rest still needs you away

#[test]
fn the_mention_sound_switch_defaults_to_away_and_only_always_sounds_in_front() {
  use crate::model::settings::MentionWhen;
  fresh();
  let mut app = boot();
  assert_eq!(app.set.mention_sound_when, MentionWhen::Away);
  assert_eq!(MentionWhen::CHOICE.usage, "<away|always>");
  assert_eq!(
    crate::model::settings::Settings::parse("mention_sound_when=always\n").mention_sound_when,
    MentionWhen::Always
  );
  assert_eq!(crate::model::settings::Settings::parse("mention_sound_when=x\n").mention_sound_when, MentionWhen::Away);
  assert!(crate::model::schema::def("mention_sound_when").unwrap().matches("mentionSoundWhen"));
  // looking at the window: no title flash either way (the sound is off in tests: volume 0)
  app.set.title_flash = true;
  app.mention_alert("t", "bob", "x");
  assert!(app.flash.is_none());
  app.set.mention_sound_when = MentionWhen::Always;
  app.mention_alert("t", "bob", "x");
  assert!(app.flash.is_none(), "always changes the sound only");
  // away: the flash comes, as ever
  app.focused = false;
  app.mention_alert("t", "bob", "x");
  assert!(app.flash.is_some());
}

// ---- the mod log across a restart, naming who acted

#[test]
fn the_mod_log_survives_a_restart_and_names_the_mod_where_it_is_known() {
  fresh();
  let mut app = boot();
  app.me = Some(me("mellen"));
  app.on_chat("forsen", Chat::ClearUser { login: "bob".into(), secs: Some(600) });
  // an action of ours: twitch's echo of it is named
  app.mod_expect("forsen", "amy");
  app.on_chat("forsen", Chat::ClearUser { login: "amy".into(), secs: None });
  let i = app.tab_of(Kind::ModLog).unwrap();
  let bodies: Vec<String> = app.tabs[i].view.msgs().map(|m| m.body.to_string()).collect();
  assert_eq!(bodies.len(), 2);
  assert!(!bodies[0].contains(" by "), "someone else's action is not named: {}", bodies[0]);
  assert!(bodies[1].ends_with(" \u{b7} by mellen"), "{}", bodies[1]);
  store::flush();
  // a new run reads it back, oldest first, as history
  let again = boot();
  let j = again.tab_of(Kind::ModLog).unwrap();
  let back: Vec<String> = again.tabs[j].view.msgs().map(|m| m.body.to_string()).collect();
  assert_eq!(back, bodies);
  assert!(again.tabs[j].view.msgs().all(|m| m.platform == Platform::Twitch));
}

#[test]
fn a_mod_log_file_that_is_not_ours_is_set_aside_not_trusted() {
  fresh();
  store::write("modlog", "garbage\nmore garbage\n").unwrap();
  let app = boot();
  let i = app.tab_of(Kind::ModLog).unwrap();
  assert_eq!(app.tabs[i].view.len(), 0);
  assert!(store::read("modlog").is_none(), "moved aside");
  // junk among good lines: the good ones come back and the file is written again clean
  let good = json!({"t": 5, "p": "t", "x": "forsen \u{b7} bob was banned"}).to_string();
  store::write("modlog", &format!("{good}\nnope\n")).unwrap();
  let app = boot();
  let i = app.tab_of(Kind::ModLog).unwrap();
  assert_eq!(app.tabs[i].view.len(), 1);
}

// ---- the starter mod buttons: once, on the first confirmed mod, never over a choice

#[test]
fn a_new_mod_gets_the_starter_buttons_once_and_an_edit_is_never_undone() {
  fresh();
  let mut app = boot();
  app.me = Some(me("mellen"));
  assert_eq!(app.set.mod_buttons, 0);
  app.on_mod_state("forsen", true);
  let want = crate::model::modbar::parse("delete_message timeout_10m ban").0;
  assert_eq!(app.set.mod_buttons, want, "delete, 10m, ban");
  assert!(store::read("settings").is_some_and(|s| s.contains("mod_buttons=delete_message,timeout_10m,ban")));
  // the user clears them: a second room never puts them back, nor does a new run
  app.buttons_to(crate::model::schema::def("mod_buttons").unwrap(), "off");
  assert_eq!(app.set.mod_buttons, 0);
  app.on_mod_state("xqc", true);
  assert_eq!(app.set.mod_buttons, 0);
  let mut again = boot();
  again.me = Some(me("mellen"));
  again.on_mod_state("forsen", true);
  assert_eq!(again.set.mod_buttons, 0, "seeded once, ever");
}

#[test]
fn buttons_set_before_the_first_mod_room_are_a_choice_and_so_is_clearing_first() {
  fresh();
  let mut app = boot();
  app.me = Some(me("mellen"));
  app.set.mod_buttons = crate::model::modbar::parse("ban").0;
  app.on_mod_state("forsen", true);
  assert_eq!(app.set.mod_buttons, crate::model::modbar::parse("ban").0, "yours stay");
  // a clear typed before ever modding
  fresh();
  let mut b = boot();
  b.me = Some(me("mellen"));
  b.buttons_to(crate::model::schema::def("mod_buttons").unwrap(), "off");
  b.on_mod_state("forsen", true);
  assert_eq!(b.set.mod_buttons, 0);
}

// ---- the sent history across a restart, with a switch and a clear

#[test]
fn the_sent_history_survives_a_restart_and_can_be_switched_off_and_cleared() {
  fresh();
  let mut app = boot();
  for m in ["first", "second"] {
    app.line.insert(m);
    app.line.take();
    app.after_send();
  }
  store::flush();
  let mut again = boot();
  again.line.older();
  assert_eq!(again.line.text, "second", "up recalls the last run's line");
  again.line.older();
  assert_eq!(again.line.text, "first");
  // switched off: the saved copy goes, memory stays, new sends are not written
  again.line.newer();
  again.line.newer();
  let d = crate::model::schema::def("remember_sent").unwrap();
  assert_eq!(d.on(&again.set), Some(true), "on by default, like the ext");
  again.flip(d);
  assert!(store::read("sent").is_none());
  again.line.insert("third");
  again.line.take();
  again.after_send();
  store::flush();
  assert!(store::read("sent").is_none(), "not written while off");
  // on again: a fresh list
  again.flip(d);
  assert_eq!(again.line.sent().count(), 0);
  again.line.insert("fourth");
  again.line.take();
  again.after_send();
  store::flush();
  assert!(store::read("sent").is_some_and(|s| s.contains("fourth") && !s.contains("first")));
  // /clearsent forgets it all, on disk too, whatever the switch says
  again.local(crate::model::settings::local_cmd("/clearsent").unwrap());
  store::flush();
  assert!(store::read("sent").is_none() && again.line.sent().count() == 0);
  assert!(boot().line.sent().count() == 0);
}

#[test]
fn a_hostile_sent_file_is_cut_down_to_clean_lines() {
  fresh();
  let mut body = String::new();
  for i in 0..200 {
    body.push_str(&format!("{}\n", json!(format!("line {i}\u{7}"))));
  }
  body.push_str("not json\n");
  store::write("sent", &body).unwrap();
  let app = boot();
  let sent: Vec<&str> = app.line.sent().collect();
  assert!(sent.len() <= 50 && sent.last() == Some(&"line 199") && sent.iter().all(|l| !l.contains('\u{7}')));
  // a file that is all junk is set aside
  store::write("sent", "junk\njunk\n").unwrap();
  let app = boot();
  assert_eq!(app.line.sent().count(), 0);
  assert!(store::read("sent").is_none());
}
