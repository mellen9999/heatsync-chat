// the whole app headless, part 4: the account and look screens, pictures, previews, the feed

use super::*;
use crate::model::message::Message;
use crate::model::settings::Counter;
use crate::model::settings::Local;
use crate::model::settings::Side;
use crate::net::Cmd;
use winit::keyboard::Key;
use winit::keyboard::NamedKey;

// the account's settings reach the window the way a flip does (look, tabs, vi, side), are saved, and
// nothing goes back out; a flip here goes out once the socket is signed in
#[test]
fn headless_account_settings() {
  use crate::model::tabs::Kind;
  use serde_json::json;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), false);
  app.canvas.resize(640, 240);
  app.me = Some(crate::net::auth::Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: None,
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  });
  app.hs_authed = true;
  let state =
    json!({"zebra": false, "viMode": true, "tabPosition": "left", "hiddenTabs": ["pinned", "feed"], "junk": [1]});
  let (inb, _) = crate::model::sync::pluck(&state);
  app.on_net(NetEvent::UiState(inb));
  assert!(!app.set.zebra && app.set.vi && !app.set.tab_feed && app.set.tab_whispers);
  assert_eq!(app.set.tab_side, Side::Left);
  assert!(app.tabs.iter().position(|t| t.kind == Kind::Feed).is_some_and(|i| !app.visible(i)), "feed went away");
  assert!(Settings::load().vi && !Settings::load().zebra, "saved");
  app.ui_state_tick();
  std::thread::sleep(std::time::Duration::from_millis(150));
  app.ui_state_tick();
  let out = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
    let mut n = 0;
    while let Ok(c) = asked.try_recv() {
      n += usize::from(matches!(c, Cmd::Hs(crate::net::hs_ws::Cmd::UiSync(_))));
    }
    n
  };
  assert_eq!(out(&mut asked), 0, "applying the account's state sends nothing back");
  app.flip(crate::model::schema::def("timestamps").unwrap());
  app.ui_state_tick();
  std::thread::sleep(std::time::Duration::from_millis(150));
  app.ui_state_tick();
  assert_eq!(out(&mut asked), 1, "your flip goes out once");
}

// tabs on each side: the bar sits there, the chat and input take the rest, a click on a tab picks it.
// HSC_DUMP=$d/x.ppm (the last side, left) ... cargo test -j2 --release headless_tab_sides
#[test]
fn headless_tab_sides() {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(640, 240);
  app.relayout_body();
  let (forsen, xqc) = (0..app.tabs.len()).fold((0, 0), |(f, x), i| match &*app.tabs[i].name {
    "forsen" => (i, x),
    "xqc" => (f, i),
    _ => (f, x),
  });
  for n in 0..30 {
    app.on_chat("forsen", Chat::Msg(Message::system(0, Platform::Twitch, format!("line {n}"))));
  }
  for side in [Side::Bottom, Side::Right, Side::Top, Side::Left] {
    app.select(forsen);
    app.side_to(side);
    app.redraw();
    let (bar, body) = (app.bar(), app.body());
    assert_eq!(bar.w * bar.h + body.w * body.h, 640 * 240, "{side:?}: bar + body tile the window");
    assert!(app.chat_area().intersect(bar).is_empty() && app.input_area().intersect(bar).is_empty());
    assert_eq!(app.tabs[forsen].view.width, body.w, "{side:?}: the chat is as wide as the body");
    assert!(app.bar_hits.iter().all(|r| bar.intersect(*r) == *r || r.is_empty()), "{side:?}: hits in the bar");
    let p = app.bar_map.iter().position(|&i| i == xqc).unwrap();
    let r = app.bar_hits[p];
    app.pointer = (r.x + 2, r.y + 2);
    app.click();
    assert_eq!(app.active, xqc, "{side:?}: a click picks the tab");
  }
  app.select(forsen);
  app.redraw();
  crate::bench::dump(&app.canvas);
}

// a picture in chat: the magnifier over it, a click fills the pane, + zooms, esc goes back.
// HSC_DUMP=$d/x.ppm (the open view) ... cargo test -j2 --release headless_img_view
#[test]
fn headless_img_view() {
  use crate::images::decode::{Decoded, Frame};
  use winit::window::CursorIcon;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(640, 400);
  app.relayout_body();
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let mut m = Message::system(0, Platform::Twitch, "look https://heatsync.org/uploads/cat.png");
  m.kind = crate::model::message::Kind::Chat;
  m.nick = "bob".into();
  m.login = "bob".into();
  app.on_chat("forsen", Chat::Msg(m));
  // every picture asked for lands as a 400x300 blue block
  let land = |app: &mut App, asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
    for _ in 0..3 {
      app.redraw();
      let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
        .filter_map(|c| if let Cmd::Image(id, _, target) = c { Some((id, target)) } else { None })
        .collect();
      if wanted.is_empty() {
        break;
      }
      for (id, target) in wanted {
        let rgba = [0, 0, 255, 255].repeat(400 * 300).into();
        app.res.images.loaded(id, target, Ok(Decoded { w: 400, h: 300, frames: vec![Frame { rgba, delay_ms: 0 }] }));
      }
      for t in &mut app.tabs {
        t.view.relayout();
      }
    }
  };
  land(&mut app, &mut asked);
  let find = |app: &mut App| {
    let area = app.chat_area();
    (area.y..area.bottom())
      .step_by(4)
      .find_map(|y| {
        (area.x..area.right())
          .step_by(4)
          .find(|&x| {
            let res = &mut app.res;
            app.tabs[chat].view.pic_at(area, x, y, res).is_some()
          })
          .map(|x| (x, y))
      })
      .expect("the picture is under the pointer somewhere")
  };
  let at = find(&mut app);
  app.pointer = at;
  app.cursor_sync();
  assert_eq!(app.cursor, CursorIcon::ZoomIn, "a magnifier over a picture");
  app.click();
  assert!(app.img_view.is_some(), "a click opens it");
  land(&mut app, &mut asked);
  app.bar_dirty = true;
  app.redraw();
  assert_eq!(app.input_h, 0, "no input bar under the picture");
  let blue = |app: &App| app.canvas.px.iter().filter(|&&p| p == 0x0000ff).count();
  assert!(blue(&app) >= 390 * 290, "the picture at its own size");
  app.cursor_sync();
  assert_eq!(app.cursor, CursorIcon::ZoomOut);
  let p = app.bar_map.iter().position(|&i| i == chat).unwrap();
  let r = app.bar_hits[p];
  crate::bench::dump(&app.canvas);
  app.img_key(Key::Character("+"));
  app.redraw();
  assert!(blue(&app) > 400 * 300, "+ makes it bigger");
  app.img_key(Key::Character("0"));
  app.img_key(Key::Named(NamedKey::Escape));
  assert!(app.img_view.is_none(), "esc closes it");
  // ← back closes it too
  app.redraw();
  app.pointer = find(&mut app);
  app.click();
  assert!(app.img_view.is_some());
  app.pointer = (r.x + 2, r.y + 2);
  app.click();
  assert!(app.img_view.is_none() && app.active == chat, "← back closes it, the tab stays");
}

// the chat look switches take effect on lines already shown, and persist.
// cargo test -j2 --release headless_chat_look
#[test]
fn headless_chat_look() {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["aaa".into()], false);
  // start from the defaults
  app.set = crate::model::settings::Settings::default();
  app.res.apply_look(&app.set);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(640, 300);
  app.relayout_body();
  let pos = app.tabs.iter().position(|t| &*t.name == "aaa").unwrap();
  app.select(pos);
  let mut m = Message::system(1_700_000_000_000, Platform::Twitch, "hi");
  m.kind = crate::model::message::Kind::Chat;
  m.nick = "bob".into();
  m.login = "bob".into();
  app.on_chat("aaa", Chat::Msg(m));
  app.redraw();
  // the ext's defaults: no clock, stripes on
  assert!(!app.res.look.timestamps && app.res.look.zebra);
  app.local(Local::Toggle("timestamps"));
  app.local(Local::Pick("time_fmt", Some(1)));
  app.local(Local::Pick("emote_size", Some(2)));
  app.local(Local::Pick("line_height", Some(2)));
  app.redraw();
  assert!(app.res.look.timestamps && app.res.look.twelve && app.res.look.emote_mul == 4);
  assert!((app.res.look.spacing - 1.3).abs() < 1e-6);
  let saved = Settings::parse(&app.set.render());
  assert!(saved.timestamps && saved.time_fmt == crate::model::settings::TimeFmt::H12);
  assert_eq!(saved.emote_size.mul(), 4);
  // a pick answers to its slash command both ways
  app.local(Local::Pick("animate_emotes", Some(1)));
  assert!(!app.res.look.animate_emotes);
  app.local(Local::Pick("animate_emotes", Some(0)));
  assert!(app.res.look.animate_emotes);
}

// the unread number after a channel's name, and tab mode: the cursor moves without switching, K
// carries a channel up, enter opens. HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_tab_mode
#[test]
fn headless_tab_mode() {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["aaa".into(), "bbb".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(640, 300);
  app.side_to(Side::Top);
  app.relayout_body();
  let pos = |app: &App, n: &str| app.tabs.iter().position(|t| &*t.name == n).unwrap();
  app.select(pos(&app, "aaa"));
  for _ in 0..3 {
    let mut m = Message::system(0, Platform::Twitch, "hi");
    m.kind = crate::model::message::Kind::Chat;
    m.nick = "bob".into();
    m.login = "bob".into();
    app.on_chat("bbb", Chat::Msg(m));
  }
  let b = pos(&app, "bbb");
  assert_eq!(app.tabs[b].count.text(Counter::Unread, false, now_ms() / 1000), "3");
  app.bar_dirty = true;
  app.redraw();
  crate::bench::dump(&app.canvas);
  // tab mode: the cursor starts on the open tab and moves without switching
  app.leader_key(Key::Character("t"));
  assert_eq!(app.rover.as_ref().map(|r| r.at), Some(pos(&app, "aaa")));
  app.rover_key(Key::Character("j"));
  assert_eq!(app.rover.as_ref().map(|r| r.at), Some(pos(&app, "bbb")));
  assert_eq!(&*app.tabs[app.active].name, "aaa", "moving never switches");
  assert!(app.mode_line().1.contains("bbb"), "the input bar says where the cursor is");
  // K carries bbb above aaa, and the cursor follows it
  let before = pos(&app, "aaa");
  app.rover_key(Key::Character("K"));
  assert_eq!(pos(&app, "bbb"), before);
  assert_eq!(app.rover.as_ref().map(|r| r.at), Some(pos(&app, "bbb")));
  assert_eq!(&*app.tabs[app.active].name, "aaa", "the open tab stays open through the swap");
  app.rover_key(Key::Named(NamedKey::Enter));
  assert!(app.rover.is_none());
  assert_eq!(&*app.tabs[app.active].name, "bbb");
  assert_eq!(app.tabs[app.active].count.text(Counter::Unread, false, now_ms() / 1000), "", "opening reads it");
}

// a reddit link: the resolver is asked once, its answer draws as a card under the line, and the card
// is a link to the post. HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_preview_card
#[test]
fn headless_preview_card() {
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(640, 400);
  app.relayout_body();
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let link = "https://www.reddit.com/r/rust/comments/1c0rgn3/x/";
  for _ in 0..2 {
    let mut m = Message::system(0, Platform::Twitch, format!("lol {link}"));
    m.kind = crate::model::message::Kind::Chat;
    m.nick = "bob".into();
    m.login = "bob".into();
    app.on_chat("forsen", Chat::Msg(m));
  }
  app.redraw();
  let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
    .filter_map(|c| if let Cmd::Preview(u) = c { Some(u) } else { None })
    .collect();
  assert_eq!(wanted, vec![Box::<str>::from(link)], "asked once for two lines");
  let v = serde_json::json!({"type": "rich", "title": "a post title", "author": "r/rust"});
  app.on_net(NetEvent::Preview(link.into(), crate::model::preview::parse(link, &v)));
  app.redraw();
  crate::bench::dump(&app.canvas);
  let cyan = app.canvas.inked(0x00ffff);
  assert!(cyan > 40, "the [reddit] tag is drawn ({cyan})");
  let area = app.chat_area();
  let hit = (area.y..area.bottom()).rev().step_by(2).find_map(|y| {
    let res = &mut app.res;
    let x = (area.x..area.right())
      .step_by(3)
      .find(|&x| Canvas::shade(app.canvas.px[(y * app.canvas.w + x) as usize], 0x00ffff))?;
    app.tabs[chat].view.link_at(area, x, y, res)
  });
  assert_eq!(hit.as_deref(), Some(link), "the card opens the post");
  // a chat-log permalink quotes the line: the speaker in their color
  let log = "https://heatsync.org/search/logs/twitch/zubatlel/2026-11-03?m=f6dd6ca7-86c2-487e-9324-d39b7109281a";
  let mut m = Message::system(0, Platform::Twitch, log);
  m.kind = crate::model::message::Kind::Chat;
  m.nick = "bob".into();
  m.login = "bob".into();
  app.on_chat("forsen", Chat::Msg(m));
  app.redraw();
  let v = serde_json::json!({"message": {"display_name": "Nightbot", "message": "quoted words",
    "timestamp": "2026-11-03T01:00:34.985Z", "color": "#00ff00"}});
  app.on_net(NetEvent::Preview(log.into(), crate::model::preview::parse(log, &v)));
  app.redraw();
  crate::bench::dump(&app.canvas);
  assert!(app.canvas.inked(0x00ff00) > 40, "the quoted speaker in green");
}

// a youtube link: its still under the line with a ▶, a hand over it, a click plays it (never the
// picture view). HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_video_still
#[test]
fn headless_video_still() {
  use crate::images::decode::{Decoded, Frame};
  use winit::window::CursorIcon;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(640, 400);
  app.relayout_body();
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let link = "https://youtu.be/dQw4w9WgXcQ";
  let mut m = Message::system(0, Platform::Twitch, format!("watch {link}"));
  m.kind = crate::model::message::Kind::Chat;
  m.nick = "bob".into();
  m.login = "bob".into();
  app.on_chat("forsen", Chat::Msg(m));
  for _ in 0..3 {
    app.redraw();
    let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| if let Cmd::Image(id, url, target) = c { Some((id, url, target)) } else { None })
      .collect();
    for (id, url, target) in wanted {
      assert!(url.contains("i.ytimg.com%2Fvi%2FdQw4w9WgXcQ"), "the still, through the proxy: {url}");
      let rgba = [0, 0, 255, 255].repeat(320 * 180).into();
      app.res.images.loaded(id, target, Ok(Decoded { w: 320, h: 180, frames: vec![Frame { rgba, delay_ms: 0 }] }));
    }
    for t in &mut app.tabs {
      t.view.relayout();
    }
  }
  app.redraw();
  crate::bench::dump(&app.canvas);
  let area = app.chat_area();
  let blue = app.canvas.px.iter().filter(|&&p| p == 0x0000ff).count();
  assert!(blue > 20_000, "the still is drawn ({blue})");
  let at = (area.y..area.bottom())
    .step_by(4)
    .find_map(|y| {
      (area.x..area.right())
        .step_by(4)
        .find(|&x| app.canvas.px[(y * app.canvas.w + x) as usize] == 0x0000ff)
        .map(|x| (x, y))
    })
    .expect("the still is on screen");
  app.pointer = (at.0 + 8, at.1 + 8);
  app.cursor_sync();
  assert_eq!(app.cursor, CursorIcon::Pointer, "a hand over a video, not the magnifier");
  while asked.try_recv().is_ok() {}
  app.click();
  assert!(app.img_view.is_none(), "a video never opens the picture view");
  let played: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
    .filter_map(|c| if let Cmd::Play(u) = c { Some(u) } else { None })
    .collect();
  assert_eq!(played, vec![Box::<str>::from(link)]);
}

// feed media, offline: a held post's box and show, the withheld files fetched, a picture under a reply,
// the [img] tag's peek. HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_feed_media
#[test]
fn headless_feed_media() {
  use crate::images::decode::{Decoded, Frame};
  use crate::model::post::parse;
  use crate::net::social;
  crate::heatsync::content::set_shown([]);
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), true);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 520);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  let op = parse(&serde_json::json!({"base36_id": "00004a", "username": "mellen9", "content": "porn test",
    "has_media": true, "media_count": "1", "media": [], "media_type": "image", "media_held": ["nsfw"]}))
  .unwrap();
  let re =
    parse(&serde_json::json!({"base36_id": "00004b", "username": "bob", "content": "nice", "reply_to": "00004a",
    "media_url": "https://heatsync.org/uploads/b.webp", "media_type": "image"}))
    .unwrap();
  app.feed.posts = vec![op.clone()];
  app.feed.loaded = true;
  let feed = app.tabs.iter().position(|t| t.kind == Kind::Feed).expect("a feed tab");
  app.select(feed);
  app.feed_open("00004a");
  app.on_net(NetEvent::Thread { id: "00004a".into(), res: Ok((op, vec![re])) });
  // every picture asked for lands as a flat 160x120 block, red
  let land = |app: &mut App, asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
    for _ in 0..3 {
      app.redraw();
      let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
        .filter_map(|c| if let Cmd::Image(id, _, target) = c { Some((id, target)) } else { None })
        .collect();
      if wanted.is_empty() {
        break;
      }
      for (id, target) in wanted {
        let d = Decoded {
          w: 160,
          h: 120,
          frames: vec![Frame { rgba: [255, 0, 0, 255].repeat(160 * 120).into(), delay_ms: 0 }],
        };
        app.res.images.loaded(id, target, Ok(d));
      }
      for t in &mut app.tabs {
        t.view.relayout();
      }
    }
  };
  land(&mut app, &mut asked);
  let spot = |app: &mut App, f: fn(&mut ChatView, Rect, i32, i32, &mut Res) -> Option<Box<str>>| {
    let area = app.chat_area();
    (area.y..area.bottom()).step_by(2).find_map(|y| {
      (area.x..area.right()).step_by(2).find_map(|x| {
        let res = &mut app.res;
        f(&mut app.tabs[feed].view, area, x, y, res).map(|id| ((x, y), id))
      })
    })
  };
  let red = |app: &App| app.canvas.px.iter().filter(|&&p| p == 0xff0000).count();
  assert!(red(&app) >= 150 * 110, "the reply's picture draws under its words");
  let cyan = app.canvas.px.iter().filter(|&&p| p == 0x00ffff).count();
  assert!(cyan > 50, "the held box wears its dashed cyan");
  let (at, id) = spot(&mut app, ChatView::reveal_at).expect("the held box has a show button");
  assert_eq!(&*id, "00004a");
  // the button is a plain word until the pointer is on it, then white behind black
  let white = |app: &App| app.canvas.px.iter().filter(|&&p| p == 0xffffff).count();
  app.redraw();
  let plain = white(&app);
  app.pointer = at;
  app.chat_hover();
  app.redraw();
  assert!(white(&app) > plain + 100, "hover reverses the button: {} vs {plain}", white(&app));
  app.pointer = (0, 0);
  app.chat_hover();
  app.redraw();
  assert_eq!(white(&app), plain, "and lets go");
  while asked.try_recv().is_ok() {}
  app.pointer = at;
  app.click();
  let asked_media = std::iter::from_fn(|| asked.try_recv().ok())
    .any(|c| matches!(c, Cmd::Social(social::Cmd::Media { ref id }) if &**id == "00004a"));
  assert!(asked_media, "show asks for the withheld files");
  let files = crate::model::post::media_of(&serde_json::json!({"media_url": "/uploads/a.webp", "media_type": "image"}));
  app.on_net(NetEvent::Media { id: "00004a".into(), res: Ok(files) });
  land(&mut app, &mut asked);
  assert!(red(&app) >= 2 * 150 * 110, "shown: the op's picture too");
  let (at, _) = spot(&mut app, ChatView::reveal_at).expect("a hide over the shown picture");
  app.pointer = at;
  app.click();
  land(&mut app, &mut asked);
  assert!(red(&app) < 2 * 150 * 110, "hide folds it back into the box");
  // the op's [img] tag, held again: its peek is the box's words
  let (at, id) = spot(&mut app, ChatView::media_at).expect("the op's [img] tag");
  assert_eq!(&*id, "00004a");
  app.pointer = at;
  app.tip_hover();
  assert!(app.media_peek.is_some(), "hovering the tag opens the peek");
  land(&mut app, &mut asked);
  crate::bench::dump(&app.canvas);
}

// the hover popup and colored mentions, offline:
//   HSC_DUMP=$d/x.ppm cargo test -j2 --release headless_hover
#[test]
fn headless_hover_and_mentions() {
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), true);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  let line = |login: &str, color: u32, body: &str, id: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, "x");
    m.kind = crate::model::message::Kind::Chat;
    m.id = id.into();
    m.login = login.into();
    m.nick = login.into();
    m.color = color;
    m.body = body.into();
    Chat::Msg(m)
  };
  let tab: &str = FLOOD_TAB;
  // the flood tab is synthetic: nobody in it counts as heard unless the probe says so
  for (who, color) in [("alice", 0x00ff00), ("carol", 0xff00ff), ("bob", 0xffff00)] {
    if let Chat::Msg(m) = line(who, color, "", "") {
      app.res.chatters.heard(&m);
    }
  }
  app.on_net(NetEvent::Chat(tab.into(), line("alice", 0x00ff00, "morning chat", "1")));
  app.on_net(NetEvent::Chat(tab.into(), line("carol", 0xff00ff, "hey", "2")));
  app.on_net(NetEvent::Chat(
    tab.into(),
    line("bob", 0xffff00, "alice you are late, ask @carol or @dave what what", "3"),
  ));
  app.redraw();
  // an unknown @dave is looked up once, in the frame after the line landed
  let mut cards = Vec::new();
  while let Ok(c) = asked.try_recv() {
    if let Cmd::Social(crate::net::social::Cmd::Card { login, .. }) = c {
      cards.push(login);
    }
  }
  assert_eq!(cards, vec!["dave".into()], "only the stranger is asked, once");
  let ink = |c: &Canvas, rgb: u32| c.inked(rgb);
  assert!(
    ink(&app.canvas, 0x00ff00) > 30 && ink(&app.canvas, 0xff00ff) > 30,
    "alice and carol are drawn in their colors"
  );
  // find alice inside bob's line and rest the pointer on her
  // a real room, so a card can say where it was opened from
  let active = app.active;
  app.tabs[active].sources.push((Platform::Twitch, "lobby".into()));
  let area = app.chat_area();
  let spot = (area.x..area.right()).step_by(2).find_map(|x| {
    (area.y..area.bottom()).find_map(|y| {
      let res = &mut app.res;
      let h = app.tabs[app.active].view.who_at(area, x, y, res)?;
      (h.mention.as_ref().is_some_and(|n| &*n.login == "alice")).then_some((x, y))
    })
  });
  app.pointer = spot.expect("alice's name in bob's line answers the pointer");
  app.tip_hover();
  assert!(app.peek.is_some() && app.peek_due.is_some(), "a cold name shows at once and asks after resting");
  app.peek_fire();
  assert!(matches!(asked.try_recv(), Ok(Cmd::Social(crate::net::social::Cmd::Card { .. }))));
  // heatsync answers
  let info = crate::model::card::Info {
    registered: true,
    rows: vec![
      ("age", "9y 2mo".into()),
      ("heat", "1.2k".into()),
      ("followers", "15k".into()),
      ("ch follow", "2y".into()),
    ],
    color: None,
    twitch_name: None,
    avatar: Some("https://static-cdn.jtvnw.net/jtv_user_pictures/a-70x70.png".into()),
    twitch_id: None,
    ..Default::default()
  };
  app.on_net(NetEvent::Card { platform: Platform::Twitch, login: "alice".into(), res: Ok(info) });
  app.redraw();
  assert!(app.peek.is_some());
  // her picture is asked for and drawn beside the name
  let (id, target) = std::iter::from_fn(|| asked.try_recv().ok())
    .find_map(|c| if let Cmd::Image(id, url, t) = c { url.contains("a-70x70").then_some((id, t)) } else { None })
    .expect("the popup asks for her picture");
  let d = crate::images::decode::Decoded {
    w: 40,
    h: 40,
    frames: vec![crate::images::decode::Frame { rgba: [0, 255, 0, 255].repeat(40 * 40).into(), delay_ms: 0 }],
  };
  app.res.images.loaded(id, target, Ok(d));
  app.redraw();
  assert!(app.canvas.px.iter().filter(|&&p| p == 0x00ff00).count() >= 30 * 30, "the picture shows");
  // the popup is up: more than the bare line shows, a warm name needs no second ask
  app.hide_tip();
  app.tip_hover();
  assert!(app.peek.is_some() && app.peek_due.is_none(), "an answered name opens at once");
  app.redraw();
  crate::bench::dump(&app.canvas);
  // a click on her name in bob's line opens her card from the cached answer: nothing more is asked
  app.hide_tip();
  assert!(app.name_press(), "the mention answers a click");
  assert!(app.card().is_some() && asked.try_recv().is_err());
}

// feed engagement offline: upvote and bookmark show at once and roll back on a refusal, your own post
// edits in the box and deletes only after a yes, /op posts, logged out shows no post rows:
//   cargo test -j2 --release headless_feed_engage
#[test]
fn headless_feed_engage() {
  use crate::app::pop::Act;
  use crate::model::post::parse;
  use crate::net::social::{self, Page};
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.set = Settings::default();
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  let now = crate::util::time::now_ms();
  let post = |id: &str, user: &str, at: i64| {
    let mut p =
      parse(&serde_json::json!({"base36_id": id, "username": user, "user_id": "9", "content": "hello feed"})).unwrap();
    p.ts = at;
    p
  };
  let (theirs, mine, old) = (
    post("00004a", "bob", now - 5000),
    post("00004b", "mellen", now - 60_000),
    post("00004c", "mellen", now - 3_600_000),
  );
  let cmds = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
    std::iter::from_fn(|| asked.try_recv().ok()).collect::<Vec<_>>()
  };
  let feed = app.tabs.iter().position(|t| t.kind == Kind::Feed).expect("a feed tab");
  app.select(feed);
  let load = |app: &mut App| {
    let posts = vec![theirs.clone(), mine.clone(), old.clone()];
    app.on_net(NetEvent::Feed(Page { page: 1, posts, hot: false, cold: false, more: false }));
    app.redraw();
  };
  let pick = |app: &mut App, id: &str| {
    let h = app.chat_area().h;
    let res = &mut app.res;
    assert!(app.tabs[feed].view.select_id(id, res, h), "{id} is on screen");
  };
  let rows = |app: &mut App, id: &str| {
    pick(app, id);
    app.line_menu_key();
    app.pop().map(|p| p.menu.items.iter().map(|r| r.1.clone()).collect::<Vec<_>>()).unwrap_or_default()
  };
  // logged out: the menu opens (copies) but has no post rows, and nothing is asked about bookmarks
  load(&mut app);
  assert!(
    !cmds(&mut asked).iter().any(|c| matches!(c, Cmd::Social(social::Cmd::BookmarkCheck { .. }))),
    "no check logged out"
  );
  let r = rows(&mut app, "00004a");
  assert!(r.contains(&"copy message".to_owned()) && !r.iter().any(|l| l.contains("upvote") || l.contains("bookmark")));
  app.close_pop();
  // log in as mellen
  app.me = Some(crate::net::auth::Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: None,
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  });
  app.feed = Feed::default();
  load(&mut app);
  let checks: Vec<_> = cmds(&mut asked)
    .into_iter()
    .filter_map(|c| if let Cmd::Social(social::Cmd::BookmarkCheck { ids }) = c { Some(ids) } else { None })
    .collect();
  assert_eq!(checks.len(), 1);
  assert_eq!(checks[0].len(), 3, "one check for the page");
  app.on_net(NetEvent::Marks { ids: checks[0].clone(), res: Ok(vec![("00004a".into(), true)]) });
  // someone else's post: upvote + bookmark (already saved: remove)
  let r = rows(&mut app, "00004a");
  assert!(r.contains(&"upvote".to_owned()) && r.contains(&"remove bookmark".to_owned()), "{r:?}");
  assert!(!r.iter().any(|l| l.starts_with("edit") || l == "delete"));
  app.close_pop();
  // upvote: shows at once, a refusal puts it back
  pick(&mut app, "00004a");
  app.act(Act::Upvote, None, None);
  assert!(app.feed.find("00004a").unwrap().voted, "the ▲ lights at once");
  assert!(cmds(&mut asked).iter().any(|c| matches!(c, Cmd::Social(social::Cmd::Vote { id }) if &**id == "00004a")));
  app.on_net(NetEvent::Voted { id: "00004a".into(), res: Err(500) });
  assert!(!app.feed.find("00004a").unwrap().voted, "rolled back");
  app.act(Act::Upvote, None, None);
  app.on_net(NetEvent::Voted { id: "00004a".into(), res: Ok((true, Some(7))) });
  let p = app.feed.find("00004a").unwrap();
  assert!(p.voted && p.heat == 7);
  assert!(rows(&mut app, "00004a").contains(&"remove upvote".to_owned()));
  app.close_pop();
  // bookmark off: the request says off, a refusal keeps it
  pick(&mut app, "00004a");
  app.act(Act::Bookmark, None, None);
  assert!(!app.feed.marks.get("00004a"));
  assert!(
    cmds(&mut asked)
      .iter()
      .any(|c| matches!(c, Cmd::Social(social::Cmd::Bookmark { id, on: false }) if &**id == "00004a"))
  );
  app.on_net(NetEvent::Bookmarked { id: "00004a".into(), on: false, res: Err(500) });
  assert!(app.feed.marks.get("00004a"), "rolled back");
  // your own post: no upvote, edit with a clock, delete
  let r = rows(&mut app, "00004b");
  assert!(!r.iter().any(|l| l.contains("upvote")));
  assert!(r.iter().any(|l| l.starts_with("edit (")) && r.last().map(String::as_str) == Some("delete"), "{r:?}");
  app.close_pop();
  let r = rows(&mut app, "00004c");
  assert!(r.contains(&"edit (expired)".to_owned()));
  app.close_pop();
  // edit: the words come into the box, the draft waits, enter saves, a refusal keeps the words
  pick(&mut app, "00004c");
  app.act(Act::Edit, None, None);
  assert!(app.edit.is_none() && app.line.text.is_empty(), "past the window: no edit mode");
  app.line.insert("my draft");
  pick(&mut app, "00004b");
  app.act(Act::Edit, None, None);
  assert!(app.edit.is_some());
  assert_eq!(app.line.text, "hello feed");
  app.line.insert(" fixed");
  app.send();
  let sent = cmds(&mut asked);
  assert!(sent.iter().any(
    |c| matches!(c, Cmd::Social(social::Cmd::Edit { id, text }) if &**id == "00004b" && text == "hello feed fixed")
  ));
  app.on_net(NetEvent::Edited { id: "00004b".into(), res: Err(403) });
  assert!(app.edit.is_some() && app.line.text == "hello feed fixed", "a refused edit keeps the words");
  app.send();
  app.on_net(NetEvent::Edited { id: "00004b".into(), res: Ok("hello feed fixed".into()) });
  assert!(app.edit.is_none());
  assert_eq!(app.line.text, "my draft", "the draft is back");
  let p = app.feed.find("00004b").unwrap();
  assert!(p.edited && &*p.text == "hello feed fixed");
  // esc drops an edit and gives the draft back
  pick(&mut app, "00004b");
  app.act(Act::Edit, None, None);
  app.key_insert(Key::Named(NamedKey::Escape), None);
  assert!(app.edit.is_none() && app.line.text == "my draft");
  while asked.try_recv().is_ok() {}
  // delete asks first: cancel sends nothing, yes sends it, the post leaves on the answer
  pick(&mut app, "00004b");
  app.act(Act::Delete, None, None);
  let p = app.pop().expect("delete asks");
  assert_eq!(p.menu.items.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["yes, delete", "cancel"]);
  assert!(cmds(&mut asked).is_empty(), "nothing sent before the yes");
  let Some(super::overlay::Overlay::Pop(p)) = &mut app.overlay else { panic!("the pop is open") };
  p.menu.at = 1;
  app.pop_key(Key::Named(NamedKey::Enter));
  assert!(cmds(&mut asked).is_empty() && app.feed.find("00004b").is_some(), "cancel keeps it");
  app.act(Act::Delete, None, None);
  app.pop_key(Key::Named(NamedKey::Enter));
  assert!(cmds(&mut asked).iter().any(|c| matches!(c, Cmd::Social(social::Cmd::Delete { id }) if &**id == "00004b")));
  app.on_net(NetEvent::Deleted { id: "00004b".into(), res: Err(500) });
  assert!(app.feed.find("00004b").is_some(), "a refused delete keeps it");
  app.on_net(NetEvent::Deleted { id: "00004b".into(), res: Ok(()) });
  assert!(app.feed.find("00004b").is_none());
  // /op posts a thread from any tab; /re answers the last [OP] heard in chat
  app.line.take();
  app.line.insert("/op hi there");
  app.send();
  assert!(
    cmds(&mut asked)
      .iter()
      .any(|c| matches!(c, Cmd::Social(social::Cmd::Post { text, reply_to: None }) if text == "hi there"))
  );
  app.line.insert("/re yes");
  app.send();
  assert!(cmds(&mut asked).is_empty(), "no [OP] in chat yet: nothing sent");
  assert_eq!(app.line.text, "", "the say line took it");
  app.on_feed_live(theirs.clone());
  app.set.inline_op = true;
  app.on_feed_live(post("00004d", "bob", now));
  app.line.insert("/re yes");
  app.send();
  assert!(cmds(&mut asked).iter().any(
    |c| matches!(c, Cmd::Social(social::Cmd::Post { text, reply_to: Some(r) }) if text == "yes" && &**r == "00004d")
  ));
  // a failed /op comes back into the box with its command
  app.on_net(NetEvent::Posted { res: Err(500) });
  assert_eq!(app.line.text, "/re yes");
  // op to feed on a chat line: the box gets /op @name: words
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  app.line.take();
  let mut m = Message::system(now_ms(), Platform::Twitch, "x");
  m.kind = crate::model::message::Kind::Chat;
  m.id = "abc".into();
  m.nick = "Carol".into();
  m.login = "carol".into();
  m.body = "this is   news".into();
  app.on_chat("forsen", Chat::Msg(m));
  app.redraw();
  app.tabs[app.active].view.select_start(&mut app.res, 300);
  app.open_line_menu((10, 10));
  let labels: Vec<_> = app.pop().unwrap().menu.items.iter().map(|r| r.1.clone()).collect();
  assert!(labels.contains(&"op to feed".to_owned()), "{labels:?}");
  app.act(Act::OpToFeed, None, None);
  assert_eq!(app.line.text, "/op @Carol: this is news");
}

// the people whose paint you hid follow the account: its list replaces yours (saved, names drawn again) and
// goes back out, as an array, only when you change it
#[test]
fn headless_hidden_paints_follow_the_account() {
  use crate::cosmetics::id::PaintId;
  use serde_json::json;
  let me = || crate::net::auth::Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: None,
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  };
  let sent = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
    let mut out = Vec::new();
    while let Ok(c) = asked.try_recv() {
      if let Cmd::Hs(crate::net::hs_ws::Cmd::UiSync(p)) = c {
        out.push(p);
      }
    }
    out
  };
  let tick = |app: &mut App| {
    app.ui_state_tick();
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.ui_state_tick();
  };
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), false);
  app.canvas.resize(640, 240);
  app.me = Some(me());
  app.hs_authed = true;
  let (inb, bad) = crate::model::sync::pluck(&json!({"hiddenPaintUsers": ["7", "kick_9", "not an id!"]}));
  assert_eq!(bad, 0);
  app.on_net(NetEvent::UiState(inb));
  let h = &app.res.cosmetics.hidden;
  assert!(h.has(&PaintId::Twitch(7)) && h.has(&PaintId::Kick(9)), "applied");
  assert!(crate::cosmetics::hidden::Hidden::load().has(&PaintId::Twitch(7)), "saved");
  tick(&mut app);
  assert!(sent(&mut asked).is_empty(), "taking the account's list sends nothing back");
  // you hide one more: the whole list goes up once, as an array
  app.res.cosmetics.hidden.toggle(&PaintId::Twitch(12));
  app.paints_edited();
  tick(&mut app);
  let patches = sent(&mut asked);
  assert_eq!(patches.len(), 1);
  assert_eq!(patches[0]["hiddenPaintUsers"], json!(["12", "7", "kick_9"]));
  // a hostile list (not an array) is dropped and counted
  let (none, bad) = crate::model::sync::pluck(&json!({"hiddenPaintUsers": "7"}));
  assert!(none.is_empty() && bad == 1);
  // the first word from the account that says nothing, while we hold some, is not a delete: ours goes up
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), false);
  app.me = Some(me());
  app.hs_authed = true;
  app.res.cosmetics.hidden = crate::cosmetics::hidden::Hidden::from_json(r#"["5"]"#).unwrap();
  let (empty, _) = crate::model::sync::pluck(&json!({"hiddenPaintUsers": []}));
  app.on_net(NetEvent::UiState(empty));
  assert!(app.res.cosmetics.hidden.has(&PaintId::Twitch(5)), "kept");
  tick(&mut app);
  assert_eq!(sent(&mut asked)[0]["hiddenPaintUsers"], json!(["5"]));
}
