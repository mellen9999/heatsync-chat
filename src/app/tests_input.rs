// the whole app, no window and no event loop: real network in over a channel, real layout + redraw
// into the canvas. run with HOME/XDG_CONFIG_HOME/XDG_CACHE_HOME pointed at an empty dir
// typing, menus and the live probe

use super::*;
use crate::cosmetics::id::PaintId;
use crate::model::message::Message;
use crate::model::message::flag;
use crate::net::Cmd;
use crate::net::Out;
use winit::keyboard::Key;
use winit::keyboard::NamedKey;

// ennortix's solid #ffb000 base (the [H] brand orange is #ff8700, so it never counts)
fn amber(c: &crate::ui::canvas::Canvas) -> usize {
  c.px.iter().filter(|p| (*p >> 16 & 255) > 200 && (165..=190).contains(&(*p >> 8 & 255)) && (*p & 255) < 60).count()
}

// twitch line from ennortix -> paint fetched -> stored -> looked up by the renderer -> amber pixels
#[test]
#[ignore]
fn live_app_paints_ennortix() {
  let _ = rustls::crypto::ring::default_provider().install_default();
  let (tx, rx) = std::sync::mpsc::channel();
  let net = Net::spawn_out(Out::Pipe(tx));
  let mut app = App::new(net, vec!["nl_kripp yt:@kripparrian".into()], false);
  app.dpi = std::env::var("HSC_PROBE_DPI").ok().and_then(|v| v.parse().ok()).unwrap_or(1.5);
  app.res.set_scale(app.scale());
  app.canvas.resize(420, 800);
  for t in &mut app.tabs {
    t.view.resize(420);
  }
  let id = PaintId::Twitch(37005319);
  let deadline = Instant::now() + Duration::from_secs(25);
  let (mut stored, mut line, mut sent) = (false, None::<(Box<str>, Message)>, false);
  let mut px = 0;
  while Instant::now() < deadline && !(sent && px > 30) {
    if let Ok(ev) = rx.recv_timeout(Duration::from_millis(250)) {
      if let NetEvent::Backfill(ch, evs) = &ev {
        let hit = evs.iter().rev().find_map(|e| match e {
          Chat::Msg(m) if PaintId::of_message(m).as_ref() == Some(&id) => Some(m.clone()),
          _ => None,
        });
        line = line.or(hit.map(|m| (ch.clone(), m)));
      }
      app.on_net(ev);
    }
    app.redraw();
    stored = stored || app.res.cosmetics.get(&id).is_some_and(|c| c.paint.is_some());
    // their paint is in: a fresh live line from them lands at the bottom of the tab
    if let (true, Some((ch, m)), false) = (stored, &line, sent) {
      let mut m = m.clone();
      m.id = "live-probe".into();
      m.flags &= !flag::HISTORY;
      app.on_net(NetEvent::Chat(ch.clone(), Chat::Msg(m)));
      sent = true;
      app.tabs[app.active].view.scroll_to_bottom();
    }
    app.redraw();
    px = amber(&app.canvas);
  }
  // heatsync says their paint changed: the refetch must not lose the look
  let frame = serde_json::json!({"type": "cosmetic:changed", "ids": [id.wire()]});
  app.on_net(NetEvent::CosmeticsChanged(crate::net::hs_ws::cosmetic_ids(&frame)));
  let again = Instant::now() + Duration::from_secs(4);
  while Instant::now() < again {
    if let Ok(ev) = rx.recv_timeout(Duration::from_millis(250)) {
      app.on_net(ev);
    }
    app.redraw();
  }
  app.tabs[app.active].view.scroll_to_bottom();
  app.redraw();
  let after = amber(&app.canvas);
  assert!(after > 30, "paint lost after cosmetic:changed (amber px {after})");
  // the old lines too: scroll the whole history past the viewport, a painted name shows somewhere
  let mut seen = 0;
  app.tabs[app.active].view.scroll_to_top(&mut app.res);
  for _ in 0..400 {
    app.redraw();
    seen = seen.max(app.tabs[app.active].view.animated_names());
    app.tabs[app.active].view.scroll_by(-400, &mut app.res);
  }
  assert!(seen > 0, "no painted ennortix line anywhere in the history");
  let n = app.tabs[app.active].view.len();
  eprintln!("rows {n} amber px {px} has paint: {:?}", app.res.cosmetics.get(&id).map(|c| c.paint.is_some()));
  if let Some(path) = std::env::var_os("HSC_DUMP") {
    let c = &app.canvas;
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(path, out);
  }
  assert!(stored, "ennortix paint never stored under {id:?}");
  assert!(sent, "no ennortix line came through the tab");
  let painted = app.tabs[app.active].view.animated_names();
  assert!(painted > 0 && px > 30, "paint stored but the renderer shows none (painted names {painted}, amber px {px})");
}

// hovering a stacked emote opens the whole stack, offline:
//   HSC_DUMP=$d/x.ppm cargo test -j2 --release headless_stack
#[test]
fn headless_stack_hover() {
  use crate::emotes::{Emote, Provider};
  use crate::images::decode::{Decoded, Frame};
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), true);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  let (base, ice) = ("https://cdn.7tv.app/emote/a/1x.webp", "https://cdn.betterttv.net/emote/b/1x.webp");
  let e = |url: &str, zero_width| Emote { url: url.into(), w: 28, h: 28, zero_width };
  app.res.emotes.set(
    None,
    Provider::SevenTv,
    [("Kappa".into(), e(base, false)), ("frozen0".into(), e(ice, true))].into(),
  );
  let mut m = Message::system(now_ms(), Platform::Twitch, "x");
  m.kind = crate::model::message::Kind::Chat;
  m.id = "1".into();
  m.login = "alice".into();
  m.nick = "alice".into();
  m.color = 0x00ff00;
  m.body = "so cold Kappa frozen0 brr".into();
  app.on_net(NetEvent::Chat(FLOOD_TAB.into(), Chat::Msg(m)));
  app.redraw();
  // a ring and a dot: the picture, not a flat square, so alignment shows
  let art = |w: u32, rgb: [u8; 3]| {
    let px: Vec<u8> = (0..w * w)
      .flat_map(|i| {
        let (x, y) = ((i % w) as i32 - w as i32 / 2, (i / w) as i32 - w as i32 / 2);
        let r2 = x * x + y * y;
        let on = r2 < (w as i32 / 2).pow(2) && (w as i32 > 14 || r2 > 4);
        if on { [rgb[0], rgb[1], rgb[2], 255] } else { [0, 0, 0, 0] }
      })
      .collect();
    Decoded { w, h: w, frames: vec![Frame { rgba: px.into(), delay_ms: 0 }] }
  };
  let area = app.chat_area();
  let spot = (area.x..area.right()).step_by(2).find_map(|x| {
    (area.y..area.bottom()).find_map(|y| {
      let res = &mut app.res;
      let h = app.tabs[app.active].view.emote_at(area, x, y, res)?;
      (&*h.name == "Kappa").then_some((x, y))
    })
  });
  app.pointer = spot.expect("the stack answers the pointer");
  app.tip_hover();
  let tip = app.tip.as_ref().expect("hovering the stack opens the popup");
  assert!(tip.is(
    "Kappa",
    base,
    &[crate::ui::chat_view::Layer {
      name: "frozen0".into(),
      url: ice.into(),
      scope: crate::emotes::block::Scope::Global
    }]
  ));
  // the chat's own pictures first, then the popup's big rungs
  // (the wanted height packs the max width above bit 16)
  let mut drawn = None;
  for _ in 0..4 {
    app.redraw();
    while let Ok(c) = asked.try_recv() {
      let Cmd::Image(id, url, h) = c else { continue };
      let n = ((h & 0xffff) / 28).max(1);
      let img = if url.contains("betterttv") { art(14 * n, [0, 255, 255]) } else { art(28 * n, [255, 0, 0]) };
      app.on_net(NetEvent::Image(id, h, Ok(img)));
    }
    drawn = app.draw_tip();
  }
  let r = drawn.expect("the popup paints once every piece is here");
  // both pieces show in the popup: the base's ring and the overlay's dot
  let has = |rgb| (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| app.canvas.px[(y * 720 + x) as usize] == rgb));
  assert!(has(0xff0000) && has(0x00ffff), "base and overlay both in the popup");
  if let Some(path) = std::env::var_os("HSC_DUMP") {
    let c = &app.canvas;
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(path, out);
  }
}

// tab-completed emotes draw as pictures in the box, an overlay typed after one stacks on it, a hand-typed
// channel emote stays a word, and backspace takes a picture whole:
//   cargo test -j2 --release headless_input_chips
#[test]
fn headless_input_chips() {
  use crate::emotes::{Emote, Provider};
  use crate::images::decode::{Decoded, Frame};
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), true);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  let (base, ice) = ("https://cdn.7tv.app/emote/a/1x.webp", "https://cdn.betterttv.net/emote/b/1x.webp");
  let e = |url: &str, zero_width| Emote { url: url.into(), w: 28, h: 28, zero_width };
  app.res.emotes.set(
    None,
    Provider::SevenTv,
    [("Kappa".into(), e(base, false)), ("frozen0".into(), e(ice, true))].into(),
  );
  let space = |app: &mut App| app.key_insert(Key::Named(NamedKey::Space), Some(" "));
  // typed by hand, a channel or global name stays a word
  app.line.insert("Kappa");
  space(&mut app);
  assert!(app.line.chip_ranges().is_empty());
  app.line.take();
  app.line.insert("hi Kap");
  app.tab_complete(false);
  assert_eq!(app.line.text, "hi Kappa ", "a pick comes with a space to keep typing");
  let tip = app.complete.as_ref().and_then(Complete::tip);
  // the first tab asks the catalogs too (ext): the tip says so until they answer
  assert_eq!(tip.as_deref(), Some("1/1 Kappa · global · 7tv users · searching…"));
  app.redraw();
  assert!(app.over_input, "the cycle's tooltip shows over the box");
  app.line.insert("frozen0");
  space(&mut app);
  assert_eq!(app.line.text, "hi Kappa frozen0 ", "the words sent are unchanged");
  assert_eq!(app.line.chip_ranges(), vec![3..8, 9..16]);
  let solid = |w: u32, rgb: [u8; 3]| {
    let px: Vec<u8> = (0..w * w).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
    Decoded { w, h: w, frames: vec![Frame { rgba: px.into(), delay_ms: 0 }] }
  };
  for _ in 0..3 {
    app.input_dirty = true;
    app.redraw();
    while let Ok(c) = asked.try_recv() {
      let Cmd::Image(id, url, h) = c else { continue };
      let img = if url.contains("betterttv") { solid(10, [0, 255, 255]) } else { solid(28, [255, 0, 0]) };
      app.on_net(NetEvent::Image(id, h & 0xffff, Ok(img)));
    }
  }
  let r = app.input_area();
  let count = |app: &App, rgb| {
    (r.y..r.bottom())
      .map(|y| (r.x..r.right()).filter(|&x| app.canvas.px[(y * 720 + x) as usize] == rgb).count())
      .sum::<usize>()
  };
  // the base's square with the overlay's smaller square on it: one stack, not two pictures
  assert!(count(&app, 0xff0000) > 28 * 20, "the base is drawn in the box");
  assert_eq!(count(&app, 0x00ffff), 100, "the overlay sits on the base");
  if let Some(path) = std::env::var_os("HSC_DUMP") {
    let c = &app.canvas;
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(path, out);
  }
  // a click puts the cursor there: before the first letter, past the end, on a picture's near side
  let hit = app.input_hit.as_ref().expect("the box remembers where its text is");
  let mid = r.y + r.h / 2;
  assert_eq!(hit.byte_at(r.x, mid), 0);
  assert_eq!(hit.byte_at(r.x + 400, mid), app.line.text.len());
  let red = (r.x..r.right()).find(|&x| app.canvas.px[(mid * 720 + x) as usize] == 0xff0000).expect("the base");
  assert_eq!(hit.byte_at(red + 1, mid), 3, "left half of the picture: before it");
  assert_eq!(hit.byte_at(red + 26, mid), 16, "right half: after the whole stack");
  // the trailing space, then the overlay whole, then the base whole
  for want in ["hi Kappa frozen0", "hi Kappa ", "hi Kappa", "hi "] {
    app.key_insert(Key::Named(NamedKey::Backspace), None);
    assert_eq!(app.line.text, want);
  }
  // vi: esc, hh lands on the whole stack (drawn as one reversed block), x takes it whole
  app.set.vi = true;
  app.mode = Mode::Insert;
  app.line.insert("Kappa frozen0 yo");
  app.key_insert(Key::Named(NamedKey::Escape), None);
  assert_eq!(app.mode, Mode::Normal);
  for _ in 0..3 {
    app.key_normal(Key::Character("h"));
  }
  assert_eq!(app.line.cursor, 3, "on the stack, not inside it");
  app.key_normal(Key::Character("x"));
  assert_eq!(app.line.text, "hi  yo");
}

// the line menu like the ext's, and private notes end to end (own HOME: it writes the notes file)
//   cargo test -j2 --release headless_line_menu
#[test]
fn headless_line_menu() {
  use crate::app::pop::Act;
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let mut m = Message::system(now_ms(), Platform::Twitch, "x");
  m.kind = crate::model::message::Kind::Chat;
  m.id = "1".into();
  m.login = "bob".into();
  m.nick = "Bob".into();
  m.color = 0x00ff00;
  m.body = "hello\tthere".into();
  app.on_chat("forsen", Chat::Msg(m));
  app.redraw();
  let labels = |app: &App| app.pop().map(|p| p.menu.items.iter().map(|r| r.1.clone()).collect::<Vec<_>>());
  let open = |app: &mut App| {
    app.tabs[app.active].view.select_start(&mut app.res, 300);
    app.open_line_menu((10, 10));
    labels(app).expect("a menu")
  };
  let rows = open(&mut app);
  for want in ["copy name", "copy message", "copy \u{2192} input", "hide in tab", "add note", "filter to Bob"] {
    assert!(rows.contains(&want.to_owned()), "{want} in {rows:?}");
  }
  // copy link sits right after copy message, and is the line's public log permalink
  let at = |l: &str| rows.iter().position(|r| r == l);
  assert_eq!(at("copy link"), at("copy message").map(|i| i + 1), "{rows:?}");
  let sel = app.tabs[app.active].view.selected().cloned().unwrap();
  let day = crate::model::logs::permalink(Platform::Twitch, "forsen", "1", sel.ts).unwrap();
  assert_eq!(app.link_of(&sel).as_deref(), Some(day.as_str()));
  assert!(day.starts_with("https://heatsync.org/search/logs/twitch/forsen/") && day.ends_with("?m=1"));
  // copy → input: the words, flat, a space after
  app.act(Act::Quote, None, None);
  assert_eq!(app.line.text, "hello there ");
  app.line.take();
  // /note: set, show, edit text comes back into the box, delete
  app.note_cmd(crate::model::notes::command("/note BOB likes cats").unwrap());
  assert_eq!(app.notes.text(Platform::Twitch, "bob"), "likes cats");
  assert!(open(&mut app).contains(&"edit note".to_owned()));
  app.act(Act::Note, app.pop().and_then(|p| p.subject.clone()), None);
  assert_eq!(app.line.text, "/note bob likes cats");
  app.line.take();
  app.note_cmd(crate::model::notes::command("/note bob").unwrap());
  assert_eq!(app.notes.text(Platform::Twitch, "bob"), "");
  // /delnote drops it too, and a second one finds nothing
  app.note_cmd(crate::model::notes::command("/note BOB again").unwrap());
  app.note_cmd(crate::model::notes::command("/delnote @bob").unwrap());
  assert_eq!(app.notes.text(Platform::Twitch, "bob"), "");
  // hide in tab flips the row
  let who = app.pop().and_then(|p| p.subject.clone());
  app.act(Act::HideInTab(true), who, None);
  assert!(app.tabs[app.active].view.hidden_user("bob"));
  // filter to opens the search on @bob, applied
  app.find_for("Bob");
  assert!(app.find.on && app.prompt.as_ref().is_some_and(|l| l.text == "@bob"));
}

// vi has two modes: normal edits the box, and once a line is picked r/m/c act on it; space leads chat
// commands:
//   cargo test -j2 --release headless_vi_normal
#[test]
fn headless_vi_normal() {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), true);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  app.set.vi = true;
  app.mode = Mode::Normal;
  let mut m = Message::system(now_ms(), Platform::Twitch, "x");
  m.kind = crate::model::message::Kind::Chat;
  (m.id, m.login, m.nick, m.color, m.body) = ("1".into(), "alice".into(), "alice".into(), 0x00ff00, "yo".into());
  app.on_net(NetEvent::Chat(FLOOD_TAB.into(), Chat::Msg(m)));
  app.redraw();
  let k = |app: &mut App, c: &'static str| app.key_normal(Key::Character(c));
  // nothing picked: r is vim's replace
  app.line.insert("hey");
  Vi::settle(&mut app.line);
  k(&mut app, "r");
  k(&mut app, "a");
  assert_eq!(app.line.text, "hea");
  assert!(app.reply.is_none());
  // a picked line takes r: reply to it, typing
  k(&mut app, "v");
  assert!(app.selecting());
  assert_eq!(app.mode_line().0.first().map(|s| s.0), Some("-- select --"), "short: the box holds text");
  k(&mut app, "r");
  assert_eq!(app.reply.as_ref().map(|r| &*r.nick), Some("alice"));
  assert_eq!(app.mode, Mode::Insert);
  // esc: back to normal (never a third mode), again: let go of the line, again: the reply goes
  app.key_insert(Key::Named(NamedKey::Escape), None);
  assert_eq!(app.mode, Mode::Normal);
  app.key_normal(Key::Named(NamedKey::Escape));
  assert!(!app.selecting() && app.reply.is_some());
  app.key_normal(Key::Named(NamedKey::Escape));
  assert!(app.reply.is_none());
  assert_eq!(app.mode, Mode::Normal);
  // space then x hides the pin, not a char; the leader shows while armed
  app.key_normal(Key::Named(NamedKey::Space));
  assert_eq!(app.mode_line().1, "space");
  k(&mut app, "x");
  assert_eq!(app.line.text, "hea");
  assert_eq!(app.mode_line().1, "");
  k(&mut app, "x");
  assert_eq!(app.line.text.len(), 2, "without space, x deletes a char");
}

// `:ka` lists emotes, `@` people, a lone `/b` commands; arrows move, enter picks into the tab cycle,
// esc shuts the list until the text moves:
//   cargo test -j2 --release headless_suggest
#[test]
fn headless_suggest() {
  use crate::emotes::{Emote, Provider};
  use crate::model::suggest::Kind as S;
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), true);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  let e = |url: &str| Emote { url: url.into(), w: 28, h: 28, zero_width: false };
  let set = [
    ("Kappa".into(), e("https://cdn.7tv.app/emote/a/1x.webp")),
    ("KappaPride".into(), e("https://cdn.7tv.app/emote/b/1x.webp")),
  ];
  app.res.emotes.set(None, Provider::SevenTv, set.into());
  let mut m = Message::system(now_ms(), Platform::Twitch, "x");
  m.kind = crate::model::message::Kind::Chat;
  (m.id, m.login, m.nick, m.color, m.body) = ("1".into(), "alice".into(), "alice".into(), 0x00ff00, "yo".into());
  app.on_net(NetEvent::Chat(FLOOD_TAB.into(), Chat::Msg(m)));
  let key = |app: &mut App, k: NamedKey| app.key_insert(Key::Named(k), None);
  let open = |app: &mut App| {
    app.input_dirty = true;
    app.redraw();
    app
      .sug
      .list
      .as_ref()
      .map(|s| (s.kind, s.items.iter().filter(|p| p.code.is_none()).map(|p| p.name.to_string()).collect::<Vec<_>>()))
  };
  app.line.insert("so :ka");
  assert_eq!(open(&mut app), Some((S::Emote, vec!["Kappa".into(), "KappaPride".into()])));
  assert!(app.over_input && !app.sug.rows.is_empty(), "the list is on screen");
  key(&mut app, NamedKey::ArrowDown);
  key(&mut app, NamedKey::Enter);
  assert_eq!(app.line.text, "so KappaPride ");
  assert_eq!(app.line.chip_ranges(), vec![3..13]);
  // the list carries on as the tab cycle
  key(&mut app, NamedKey::Tab);
  assert!(app.line.chip_ranges().is_empty() && app.line.text.starts_with("so "), "next comes the emoji: text, no chip");
  app.tab_complete(true);
  app.tab_complete(true);
  assert_eq!(app.line.text, "so Kappa ");
  assert_eq!(app.line.chip_ranges(), vec![3..8]);
  app.line.take();
  app.complete = None;
  // shift-tab on the open list goes backwards: from the top it wraps to the last row
  app.line.insert("so :ka");
  let last = open(&mut app).and_then(|_| app.sug.list.as_ref()?.items.last().map(|p| p.name.to_string())).unwrap();
  app.mods = winit::keyboard::ModifiersState::SHIFT;
  key(&mut app, NamedKey::Tab);
  app.mods = winit::keyboard::ModifiersState::empty();
  assert!(app.line.text.starts_with("so ") && !app.line.text.starts_with("so Kappa "), "{}", app.line.text);
  assert_eq!(app.complete.as_ref().map(|c| c.at), app.complete.as_ref().map(|c| c.matches.len() - 1), "{last}");
  app.line.take();
  app.complete = None;
  app.line.insert("hi @al");
  assert_eq!(open(&mut app).map(|o| o.0), Some(S::People));
  key(&mut app, NamedKey::Tab);
  assert_eq!(app.line.text, "hi @alice ");
  app.line.take();
  app.complete = None;
  app.line.insert("/ba");
  let cmds = open(&mut app).expect("a lone /word lists commands");
  assert!(cmds.0 == S::Command && cmds.1.contains(&"/ban".to_string()));
  if let Some(path) = std::env::var_os("HSC_DUMP") {
    let c = &app.canvas;
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(path, out);
  }
  key(&mut app, NamedKey::Escape);
  assert_eq!(open(&mut app), None, "esc shuts it");
  assert_eq!(app.line.text, "/ba");
  app.line.insert("n");
  assert!(open(&mut app).is_some(), "typing opens it again");
}

// past 500 characters (twitch) the box draws the rest red, like the ext; at the limit nothing is red
#[test]
fn words_past_the_limit_draw_red() {
  let (net, _rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(520, 300);
  let red = |app: &App| app.canvas.inked(crate::ui::theme::RED);
  // as typed: the cursor at the end, so the end is in view
  let typed = |app: &mut App, t: String| {
    app.line.set_text(t);
    app.line.cursor = app.line.text.len();
    app.input_dirty = true;
    app.redraw();
  };
  typed(&mut app, "wörd ".repeat(100));
  assert_eq!(red(&app), 0, "500 fits");
  typed(&mut app, format!("{}{}", "wörd ".repeat(100), "wwwwwwwwww"));
  assert!(red(&app) > 20, "the 10 past it are red");
}

// one word longer than the box wraps inside itself, so its end (and the red past the limit) is on screen
#[test]
fn one_long_word_wraps_and_its_end_shows() {
  let (net, _rx) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(520, 300);
  app.line.set_text("a".repeat(600));
  app.line.cursor = app.line.text.len();
  app.input_dirty = true;
  app.redraw();
  crate::bench::dump(&app.canvas);
  assert!(app.canvas.inked(crate::ui::theme::RED) > 0, "the tail past 500 is in view");
}

// an overlay typed after a lone emoji stacks on it in the box too (🏙 fog0), like a chat line:
//   HSC_DUMP=$d/x.ppm cargo test -j2 --release an_overlay_typed_after_an_emoji
#[test]
fn an_overlay_typed_after_an_emoji_stacks_on_it_in_the_box() {
  use crate::emotes::{Emote, Provider};
  use crate::images::decode::{Decoded, Frame};
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(520, 300);
  let fog = "https://cdn.7tv.app/emote/f/1x.webp";
  let e = Emote { url: fog.into(), w: 28, h: 28, zero_width: true };
  app.res.emotes.set(None, Provider::SevenTv, [("fog0".into(), e)].into());
  for t in ["🏙 ", "fog0 "] {
    app.line.insert(t);
    app.chip_spaced();
  }
  assert!(app.line.chips.contains("fog0"), "the overlay turns into its emote");
  let chips = app.input_chips();
  assert_eq!(chips.len(), 1);
  assert_eq!(chips[0].range, 0.."🏙 fog0".len(), "one stack from the emoji");
  assert_eq!(chips[0].glyph.as_deref(), Some("🏙"));
  // a translucent band, so the emoji under it shows through
  for _ in 0..3 {
    app.input_dirty = true;
    app.redraw();
    while let Ok(c) = asked.try_recv() {
      let Cmd::Image(id, _, h) = c else { continue };
      let px: Vec<u8> = (0..28 * 28).flat_map(|i| if (i / 28) % 7 < 2 { [0, 255, 255, 160] } else { [0; 4] }).collect();
      let img = Decoded { w: 28, h: 28, frames: vec![Frame { rgba: px.into(), delay_ms: 0 }] };
      app.on_net(NetEvent::Image(id, h, Ok(img)));
    }
  }
  crate::bench::dump(&app.canvas);
  assert!(app.canvas.inked(0x00ffff) > 0, "the overlay is drawn");
  // plain words never stack on an emoji
  app.line.set_text("🏙 what ".into());
  app.line.cursor = app.line.text.len();
  app.chip_spaced();
  assert!(!app.line.chips.contains("what"));
}
