// the whole app headless, part 3: streams, tabs, threads, line menus and chat logs

use super::*;
use crate::model::message::Message;
use crate::net::Cmd;
use winit::keyboard::Key;
use winit::keyboard::NamedKey;

// a channel's went-live lands in every chat tab; only its own tab turns yellow:
//   cargo test -j2 --release headless_stream_everywhere
#[test]
fn headless_stream_everywhere() {
  use crate::model::stream_events::{Line, Tone};
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
  let tab = |app: &App, n: &str| app.tabs.iter().position(|t| &*t.name == n).unwrap();
  let (forsen, xqc) = (tab(&app, "forsen"), tab(&app, "xqc"));
  app.select(forsen);
  let line = |ch: &str| Line {
    platform: Platform::Twitch,
    channel: ch.into(),
    text: format!("[{ch}] ◆ went live").into(),
    tone: Tone::Online,
    name: ch.into(),
    color: None,
    verb: "◆ went live".into(),
    game: "".into(),
  };
  let n = |app: &App, i: usize| app.tabs[i].view.len();
  let before = (n(&app, forsen), n(&app, xqc));
  // followed, no tab: both tabs get it, neither turns yellow
  app.on_stream(line("ennortix"));
  assert_eq!((n(&app, forsen), n(&app, xqc)), (before.0 + 1, before.1 + 1));
  assert!(!app.tabs[xqc].event && !app.tabs[xqc].unread, "someone else's news lights no tab");
  // a tabbed channel: everywhere too, and its own tab turns yellow
  app.on_stream(line("xqc"));
  assert_eq!(n(&app, forsen), before.0 + 2);
  assert!(app.tabs[xqc].event && !app.tabs[forsen].event);
  // drawn like the ext: name in its color, `◆ verb` in the tone, the game white, a stripe at the left
  app.on_stream(Line {
    color: Some(0x00ff00),
    tone: Tone::Switch,
    verb: "◆ switched to ".into(),
    game: "Chess".into(),
    ..line("forsen")
  });
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 200);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  app.redraw();
  let has = |rgb| app.canvas.px.contains(&rgb);
  assert!(has(0x00ff00) && has(crate::ui::theme::YELLOW), "the name in its color, the verb in yellow");
  crate::bench::dump(&app.canvas);
}

// an @mention while the window is away flashes the title with who it was, the latest name winning,
// until the window comes back; focused, nothing flashes:
//   cargo test -j2 --release headless_mention_flash
#[test]
fn headless_mention_flash() {
  use crate::model::message::Kind;
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  // silent: a test never plays the ping
  app.set.mention_volume = crate::model::settings::Volume::V0;
  app.set.title_flash = true;
  app.mentions.set(["mellen"]);
  let line = |app: &mut App, who: &str| {
    let mut m = Message::system(1, Platform::Twitch, "x");
    (m.kind, m.login, m.nick, m.body) = (Kind::Chat, who.into(), who.into(), "hi @mellen".into());
    app.scan_mention(Platform::Twitch, "forsen", &mut m);
  };
  line(&mut app, "bob");
  assert!(app.flash.is_none(), "focused: you saw it");
  app.focused = false;
  line(&mut app, "bob");
  line(&mut app, "eve\u{7}");
  assert_eq!(app.flash.as_ref().map(|f| &*f.0), Some("eve"), "the latest, control bytes gone");
  app.flash_tick();
  assert!(app.flash.as_ref().is_some_and(|f| f.1) && app.flash_due.is_some(), "showing the name");
  app.flash_tick();
  assert!(app.flash.as_ref().is_some_and(|f| !f.1), "and back");
  app.focused = true;
  app.flash_stop();
  assert!(app.flash.is_none() && app.flash_due.is_none());
  // switched off mid-flash: the next turn stops it
  app.focused = false;
  line(&mut app, "bob");
  app.set.title_flash = false;
  app.flash_tick();
  assert!(app.flash.is_none() && app.flash_due.is_none());
}

// a stream line turns an unwatched tab yellow, and right-click clears a red, cyan or yellow tab
// without opening it or its menu, offline:
//   cargo test -j2 --release headless_tab_ack
#[test]
fn headless_tab_ack() {
  use crate::model::stream_events::Tone;
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), true);
  app.canvas.resize(720, 360);
  let at = |app: &App, k: Kind| app.tabs.iter().position(|t| t.kind == k).unwrap();
  let (feed, flood) = (at(&app, Kind::Feed), app.tabs.iter().position(|t| &*t.name == FLOOD_TAB).unwrap());
  let (mentions, whispers) = (at(&app, Kind::Mentions), at(&app, Kind::Whispers));
  app.select(feed);
  app.say_line(flood, Message::system(0, Platform::Twitch, "switched to chess"), Tone::Switch);
  assert!(app.tabs[flood].event && app.tabs[flood].unread, "the news turns the tab yellow");
  // a mention lights the tab; looking on another client before it changes nothing, after it puts it out
  use crate::model::unseen::Surface;
  app.unseen_note(Surface::Mentions);
  assert!(app.tabs[mentions].unread);
  app.on_net(NetEvent::Seen { surface: Surface::Mentions, at: now_ms() - 60_000 });
  assert!(app.tabs[mentions].unread, "an older look elsewhere leaves it lit");
  app.on_net(NetEvent::Seen { surface: Surface::Mentions, at: now_ms() + 1 });
  assert!(!app.tabs[mentions].unread, "a look on another client puts it out");
  app.say_line(flood, Message::system(0, Platform::Twitch, "[🔥] chat is popping"), Tone::Moment);
  app.tabs[mentions].unread = true;
  app.tabs[whispers].unread = true;
  let right_click = |app: &mut App, i: usize| {
    app.bar_dirty = true;
    app.redraw();
    let p = app.bar_map.iter().position(|&t| t == i).expect("the tab is in the bar");
    let r = app.bar_hits[p];
    app.pointer = (r.x + r.w / 2, r.y + r.h / 2);
    app.right_click();
  };
  right_click(&mut app, flood);
  assert!(!app.tabs[flood].event && app.tabs[flood].unread, "yellow clears, the plain unread stays");
  for i in [mentions, whispers] {
    right_click(&mut app, i);
    assert!(!app.tabs[i].unread && app.tab_menu().is_none(), "cleared, no menu");
  }
  assert_eq!(app.active, feed, "nothing was opened");
  right_click(&mut app, mentions);
  assert!(app.tab_menu().is_some(), "a quiet tab still gets its menu");
  // opening a yellow tab clears it too
  app.close_tab_menu();
  app.say_line(flood, Message::system(0, Platform::Twitch, "went offline"), Tone::Offline);
  app.select(flood);
  assert!(!app.tabs[flood].event);
}

// a click on a reply's >>id opens its parent's thread with the reply picked out, and a click on
// the open thread's own >>id goes back to the list, offline:
//   cargo test -j2 --release headless_feed_thread
#[test]
fn headless_feed_thread_click() {
  use crate::model::post::parse;
  use crate::net::social;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, Vec::new(), true);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  let post = |id: &str, reply_to: Option<&str>| {
    let mut v = serde_json::json!({"base36_id": id, "username": "alice", "content": "hi"});
    if let Some(r) = reply_to {
      v["reply_to"] = r.into();
    }
    parse(&v).unwrap()
  };
  let (op, re) = (post("00004c", None), post("00004d", Some("00004c")));
  app.feed.posts = vec![re.clone(), op.clone()];
  app.feed.loaded = true;
  let feed = app.tabs.iter().position(|t| t.kind == Kind::Feed).expect("a feed tab");
  app.select(feed);
  app.feed_show();
  app.redraw();
  while asked.try_recv().is_ok() {}
  // the >>id of the line holding `id`
  let id_spot = |app: &mut App, id: &str| {
    let area = app.chat_area();
    (area.x..area.right()).find_map(|x| {
      (area.y..area.bottom()).find_map(|y| {
        let res = &mut app.res;
        (app.tabs[feed].view.thread_at(area, x, y, res)?.as_ref() == id).then_some((x, y))
      })
    })
  };
  app.pointer = id_spot(&mut app, "00004d").expect("the reply's >>4d is on screen");
  app.click();
  let t = app.feed.thread.as_ref().expect("the click opens a thread");
  assert_eq!(&*t.id, "00004c", "a reply opens its parent's thread");
  let fetched = std::iter::from_fn(|| asked.try_recv().ok())
    .any(|c| matches!(c, Cmd::Social(social::Cmd::Thread { ref id }) if &**id == "00004c"));
  assert!(fetched, "the thread is fetched");
  app.on_net(NetEvent::Thread { id: "00004c".into(), res: Ok((op, vec![re])) });
  app.redraw();
  let picked = app.tabs[feed].view.selected().map(|m| m.id.clone());
  assert_eq!(picked.as_deref(), Some("00004d"), "the clicked reply is picked out");
  app.pointer = id_spot(&mut app, "00004c").expect("the op's >>4c heads the thread");
  app.click();
  assert!(app.feed.thread.is_none(), "the open thread's own >>id goes back");
  assert!(app.tabs[feed].under.is_none(), "and the list is back in its tab");
}

// right-click on a chat line opens its menu; r on a picked line starts a reply
#[test]
fn headless_line_menu_and_r() {
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
  let mut m = Message::system(now_ms(), Platform::Twitch, "hello there");
  m.kind = crate::model::message::Kind::Chat;
  m.id = "abc".into();
  m.nick = "bob".into();
  m.login = "bob".into();
  app.on_chat("forsen", Chat::Msg(m));
  app.redraw();
  let area = app.chat_area();
  // the newest line sits at the bottom of the chat
  app.pointer = (area.x + 200, area.bottom() - 6);
  app.right_click();
  assert!(app.pop().is_some(), "right-click opens the line menu");
  app.close_pop();
  app.mode = Mode::Normal;
  app.key_normal(Key::Character("r"));
  assert!(app.reply.is_some(), "r on the picked line starts a reply");
}

// 🏙 fog0: the overlay stacks on the emoji, drawn over it (HSC_DUMP to look)
#[test]
fn headless_overlay_on_emoji() {
  use crate::emotes::{Emote, Provider};
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(420, 200);
  for t in &mut app.tabs {
    t.view.resize(420);
  }
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let url = "https://cdn.7tv.app/emote/fog/1x.webp";
  let fog = Emote { url: url.into(), w: 28, h: 28, zero_width: true };
  app.res.emotes.set(Some("forsen"), Provider::SevenTv, [("fog0".into(), fog)].into());
  let mut m = Message::system(now_ms(), Platform::Twitch, "🏙  fog0 and 🏙 alone");
  (m.kind, m.id, m.nick, m.login) = (crate::model::message::Kind::Chat, "e1".into(), "bob".into(), "bob".into());
  app.on_chat("forsen", Chat::Msg(m));
  app.redraw();
  // the fog: a see-through gray band over the lower half
  let lh = app.res.look.emote_px();
  let id = app.res.images.id(url, lh);
  let px = app.res.images.px(lh);
  let rgba: Vec<u8> =
    (0..px * px).flat_map(|i| if i / px > px / 2 { [200, 200, 200, 160] } else { [0, 0, 0, 0] }).collect();
  let frame = crate::images::decode::Frame { rgba: rgba.into(), delay_ms: 0 };
  app.res.images.loaded(id, px, Ok(crate::images::decode::Decoded { w: px, h: px, frames: vec![frame] }));
  app.tabs[chat].view.relayout();
  app.redraw();
  if let Some(path) = std::env::var_os("HSC_DUMP") {
    let c = &app.canvas;
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(path, out);
  }
  // the fog's gray lands on the canvas somewhere in the chat
  let area = app.chat_area();
  let gray = (area.y..area.bottom())
    .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
    .filter(|&(x, y)| {
      let p = app.canvas.px[(y * app.canvas.w + x) as usize];
      let (r, g, b) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
      r == g && g == b && (100..=150).contains(&r)
    })
    .count();
  assert!(gray > 20, "the overlay was drawn ({gray} px)");
}

// a went-live line in every tab is news, not chat: never picked, menued or replied to
#[test]
fn headless_stream_line_is_not_a_chat_line() {
  use crate::model::stream_events::{Line, Tone};
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
  let say = |app: &mut App, id: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, "hello there");
    m.kind = crate::model::message::Kind::Chat;
    m.id = id.into();
    m.nick = "bob".into();
    m.login = "bob".into();
    app.on_chat("forsen", Chat::Msg(m));
  };
  say(&mut app, "a1");
  app.on_stream(Line {
    platform: Platform::Twitch,
    channel: "xqc".into(),
    text: "[xqc] ◆ went live — Chess".into(),
    tone: Tone::Online,
    name: "xQc".into(),
    color: Some(0xff0000),
    verb: "◆ went live — ".into(),
    game: "Chess".into(),
  });
  say(&mut app, "a2");
  app.redraw();
  let area = app.chat_area();
  // every row: a right-click picks a real line or nothing, never the stream line
  let mut picked = Vec::new();
  for y in (area.y..area.bottom()).step_by(2) {
    app.tabs[chat].view.deselect();
    (app.overlay, app.pointer) = (None, (area.x + 200, y));
    app.right_click();
    let id = app.tabs[chat].view.selected().map(|m| m.id.clone());
    assert!(id.as_deref() != Some(""), "the stream line was picked at y {y}");
    assert_eq!(app.pop().is_some(), id.is_some(), "a menu only for a picked line (y {y})");
    picked.extend(id);
  }
  assert!(picked.iter().any(|i| &**i == "a1") && picked.iter().any(|i| &**i == "a2"), "{picked:?}");
  // k from the newest steps over it, and r replies to a real line
  app.tabs[chat].view.deselect();
  (app.overlay, app.mode) = (None, Mode::Normal);
  app.select_move(-1);
  app.select_move(-1);
  assert_eq!(app.tabs[chat].view.selected().map(|m| m.id.clone()).as_deref(), Some("a1"));
  app.key_normal(Key::Character("r"));
  assert!(app.reply.is_some(), "r replies");
}

// a feed post lands as a line in a chat tab; its >>id opens the thread right there, the tab reads
// `← back`, chat keeps landing underneath, and back puts the chat where it was:
//   ... cargo test -j2 --release headless_thread_takeover
#[test]
fn headless_thread_takeover() {
  use crate::model::post::parse;
  use crate::net::social;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let line = |t: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, t);
    m.kind = crate::model::message::Kind::Chat;
    m.nick = "bob".into();
    m.login = "bob".into();
    m
  };
  app.on_chat("forsen", Chat::Msg(line("before")));
  let op =
    parse(&serde_json::json!({"base36_id": "00003t", "username": "alice", "user_id": "5", "content": "a thread"}))
      .unwrap();
  app.on_feed_live(op.clone());
  app.on_feed_live(op.clone());
  let rows = app.tabs[chat].view.len();
  assert_eq!(rows, 2, "the post is one line in the chat tab, heard once");
  app.on_chat("forsen", Chat::Msg(line("see >>3t")));
  app.redraw();
  while asked.try_recv().is_ok() {}
  let area = app.chat_area();
  let spot = |app: &mut App, want: &str| {
    (area.y..area.bottom()).find_map(|y| {
      (area.x..area.right()).find_map(|x| {
        let res = &mut app.res;
        (app.tabs[chat].view.thread_at(area, x, y, res)?.as_ref() == want).then_some((x, y))
      })
    })
  };
  // both the post's own >>3t and the one written in chat name it; click the written one
  let hits: Vec<_> = (area.y..area.bottom())
    .filter_map(|y| {
      let res = &mut app.res;
      (area.x..area.right()).find(|&x| app.tabs[chat].view.thread_at(area, x, y, res).is_some()).map(|x| (x, y))
    })
    .collect();
  assert!(hits.len() >= 2, "a post's >>id and one typed in chat are both clickable");
  app.pointer = *hits.last().unwrap();
  app.click();
  assert!(app.tabs[chat].under.is_some(), "the thread took the chat tab over");
  assert!(app.feedish(), "keys and typing go to the thread");
  let fetched = std::iter::from_fn(|| asked.try_recv().ok())
    .any(|c| matches!(c, Cmd::Social(social::Cmd::Thread { ref id }) if &**id == "00003t"));
  assert!(fetched);
  app.on_net(NetEvent::Thread { id: "00003t".into(), res: Ok((op, vec![])) });
  app.on_chat("forsen", Chat::Msg(line("during")));
  assert_eq!(app.tabs[chat].view.len(), 2, "the thread view: the op and 'no replies yet'");
  app.redraw();
  assert!(spot(&mut app, "00003t").is_some());
  crate::bench::dump(&app.canvas);
  // the tab reads `← back`; a click on it goes back
  let p = app.bar_map.iter().position(|&i| i == chat).unwrap();
  let r = app.bar_hits[p];
  app.pointer = (r.x + 2, r.y + 2);
  app.bar_click();
  assert!(app.tabs[chat].under.is_none() && app.feed.thread.is_none());
  assert_eq!(app.tabs[chat].view.len(), rows + 2, "chat that landed meanwhile is there");
}

// right-click a line → chat logs: one person's archive over the tab, drawn as chat, older pages at
// the top, a to swap scope, / to search, esc back:
//   HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_chat_logs
#[test]
fn headless_chat_logs() {
  use crate::model::logs::{self, Who};
  use crate::net::social;
  use serde_json::json;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  app.res.look.timestamps = false;
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let line = |who: &str, t: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, t);
    m.kind = crate::model::message::Kind::Chat;
    m.nick = who.into();
    m.login = who.into();
    m.id = format!("live-{t}").into();
    m
  };
  app.on_chat("forsen", Chat::Msg(line("bob", "hello chat")));
  let before = app.tabs[chat].view.len();
  // the line menu has the row; picking it opens the view
  app.tabs[chat].view.select_start(&mut app.res, 300);
  app.open_line_menu((20, 20));
  let rows = app.pop().map(|p| p.menu.items.iter().map(|r| r.1.clone()).collect::<Vec<_>>()).unwrap();
  assert_eq!(rows.last().map(String::as_str), Some("chat logs"), "after filter to, like the ext");
  app.pop_key(Key::Named(NamedKey::Escape));
  let m = app.tabs[chat].view.selected().cloned().unwrap();
  let who = app.subject_of(&m).unwrap();
  app.act(pop::Act::Logs, Some(who), None);
  assert!(app.logs.is_some() && app.tabs[chat].under.is_some(), "the chat tab is taken over");
  assert!(!app.feedish(), "not a feed thread: no votes, no posting");
  let asks = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
    std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| match c {
        Cmd::Social(social::Cmd::Logs(r)) => Some(r),
        _ => None,
      })
      .collect::<Vec<_>>()
  };
  let first = asks(&mut asked);
  assert_eq!(first.len(), 1);
  assert_eq!(first[0].path(), "/api/archive/user/twitch/bob/messages?limit=100&channel=forsen");
  let w = Who { platform: Platform::Twitch, login: "bob".into(), nick: "bob".into(), user_id: "".into() };
  let row = |n: i64, ch: &str, html: &str| {
    json!({"message": format!("archived {n}"), "message_id": format!("m{n}"), "channel": ch,
      "timestamp": format!("2026-10-3{}T23:59:5{}.000Z", n % 2, n % 10), "color": "#00FF00", "display_name": "Bob",
      "message_html": html})
  };
  let page = |from: i64, n: i64, cursor: Option<&str>| {
    let rows: Vec<_> = (from..from + n).rev().map(|i| row(i, "forsen", "")).collect();
    logs::parse_page(&json!({"results": rows, "next_cursor": cursor}), &w)
  };
  // a stale answer (from before something else) is ignored
  app.on_net(NetEvent::Logs { epoch: first[0].epoch + 1000, res: Ok(page(0, 3, None)) });
  assert!(app.tabs[chat].view.len() == 0, "nothing from a stale answer");
  app.on_net(NetEvent::Logs { epoch: first[0].epoch, res: Ok(page(10, 40, Some("2026-10-30|1"))) });
  assert_eq!(app.tabs[chat].view.len(), 41, "forty lines and the dim line at the top");
  app.redraw();
  crate::bench::dump(&app.canvas);
  // keys: j picks the newest, y finds its public link, g goes to the top and asks for older
  assert!(app.logs_key(Key::Character("j")));
  let picked = app.tabs[chat].view.selected().cloned().unwrap();
  assert_eq!(&*picked.id, "m49");
  assert_eq!(
    app.logs.as_ref().unwrap().link_of(&picked).as_deref(),
    Some("https://heatsync.org/search/logs/twitch/forsen/2026-10-31?m=m49")
  );
  assert!(app.logs_key(Key::Character("g")));
  let older = asks(&mut asked);
  assert_eq!(older.len(), 1, "the top asks for the next page");
  assert_eq!(older[0].cursor.as_deref(), Some("2026-10-30|1"));
  app.on_net(NetEvent::Logs { epoch: older[0].epoch, res: Ok(page(0, 10, None)) });
  assert_eq!(app.tabs[chat].view.len(), 51, "ten older lines in front");
  assert!(app.logs.as_ref().unwrap().done, "no cursor left");
  // a: every channel. the old answer is dropped, a fresh ask has no channel filter
  assert!(app.logs_key(Key::Character("a")));
  let all = asks(&mut asked);
  assert_eq!(all.len(), 1);
  assert!(all[0].channel.is_none());
  let html = "hi <img class=\"hs-archive-emote\" src=\"/e/7tv/01ABC\" alt=\"LUL\">";
  let mut rows: Vec<_> = (0..6).rev().map(|i| row(i, if i % 2 == 0 { "forsen" } else { "xqc" }, "")).collect();
  rows.push(json!({"message": "hi LUL", "message_html": html, "channel": "xqc", "message_id": "e1",
    "timestamp": "2026-10-31T00:00:00Z"}));
  rows.rotate_right(1);
  let p = logs::parse_page(&json!({"results": rows}), &w);
  app.on_net(NetEvent::Logs { epoch: all[0].epoch, res: Ok(p) });
  assert_eq!(app.tabs[chat].view.len(), 7 + 1);
  // live: bob in another room joins the bottom of the every-channel view, alice never does
  app.on_chat("forsen", Chat::Msg(line("alice", "not bob")));
  app.on_chat("forsen", Chat::Msg(line("bob", "live line")));
  assert_eq!(app.tabs[chat].view.len(), 9);
  assert_eq!(app.tabs[chat].under.as_ref().unwrap().len(), before + 2, "chat keeps landing underneath");
  app.redraw();
  crate::bench::dump(&app.canvas);
  // / searches the archive on enter; esc clears it, then esc leaves
  assert!(app.logs_key(Key::Character("/")));
  assert!(app.find.on && app.prompt.is_some());
  for c in "lul".chars() {
    let t = c.to_string();
    app.find_key(Key::Character(&t), Some(&t));
  }
  app.find_key(Key::Named(NamedKey::Enter), None);
  let s = asks(&mut asked);
  assert_eq!(s.len(), 1);
  assert_eq!(s[0].query.as_deref(), Some("lul"));
  app.on_net(NetEvent::Logs { epoch: s[0].epoch, res: Ok(logs::parse_page(&json!({"results": []}), &w)) });
  assert_eq!(app.tabs[chat].view.len(), 0, "an empty search says so");
  assert!(app.logs_key(Key::Named(NamedKey::Escape)));
  assert!(app.logs.as_ref().unwrap().query.is_none(), "esc clears the search first");
  assert_eq!(asks(&mut asked).len(), 1, "and the timeline is asked for again");
  assert!(app.logs_key(Key::Named(NamedKey::Escape)));
  assert!(app.logs.is_none() && app.tabs[chat].under.is_none(), "esc leaves");
  assert_eq!(app.tabs[chat].view.len(), before + 2, "the chat is back with what landed meanwhile");
  // a failure never crashes and a late answer for a closed view is dropped
  app.on_net(NetEvent::Logs { epoch: s[0].epoch, res: Err(503) });
  // the ← back tab closes it too
  let who = app.subject_of(&m).unwrap();
  app.act(pop::Act::Logs, Some(who), None);
  assert!(app.logs.is_some());
  let first = asks(&mut asked).pop().unwrap();
  app.on_net(NetEvent::Logs { epoch: first.epoch, res: Err(503) });
  assert!(app.logs.as_ref().unwrap().failed && !app.logs.as_ref().unwrap().done);
  app.redraw();
  let p = app.bar_map.iter().position(|&i| i == chat).unwrap();
  let r = app.bar_hits[p];
  app.pointer = (r.x + 2, r.y + 2);
  app.bar_click();
  assert!(app.logs.is_none() && app.tabs[chat].under.is_none());
}

// heatsync's refs for a twitch line land on our own copy of it, by id, whichever comes first; additive,
// a name the line already has keeps its ref
#[test]
fn headless_late_refs_land_on_their_line() {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  let f = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  let line = |id: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, "kek pog");
    m.kind = crate::model::message::Kind::Chat;
    (m.id, m.nick, m.login) = (id.into(), "bob".into(), "bob".into());
    m
  };
  let url = |n: &str| format!("https://cdn.7tv.app/emote/{n}/1x.webp");
  let refs = |pairs: &[(&str, &str)]| -> Box<[crate::model::message::HsEmote]> {
    pairs.iter().map(|(w, u)| ((*w).into(), url(u).into(), false)).collect()
  };
  let hs = |app: &App| {
    app.tabs[f].view.last_msg().map(|m| m.hs.iter().map(|h| (h.0.to_string(), h.1.to_string())).collect::<Vec<_>>())
  };
  let before = app.tabs[f].view.len();
  // the line first, then its refs
  let mut first = line("a");
  first.hs = refs(&[("kek", "MINE")]);
  app.on_chat("forsen", Chat::Msg(first));
  app.on_net(NetEvent::HsRefs { chan: "forsen".into(), id: "a".into(), refs: refs(&[("kek", "OTHER"), ("pog", "P")]) });
  assert_eq!(hs(&app), Some(vec![("kek".into(), url("MINE")), ("pog".into(), url("P"))]));
  // the refs first: they wait for the line
  app.on_net(NetEvent::HsRefs { chan: "forsen".into(), id: "b".into(), refs: refs(&[("pog", "P")]) });
  app.on_chat("forsen", Chat::Msg(line("b")));
  assert_eq!(hs(&app), Some(vec![("pog".into(), url("P"))]));
  assert_eq!(app.tabs[f].view.len(), before + 2, "never a second copy of the line");
}

// a channel tab's right-click menu is edit + remove, and every other action lives in the edit form
//   cargo test -j2 --release headless_tab_menu
#[test]
fn headless_tab_menu_is_edit_remove() {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.canvas.resize(720, 360);
  let flood = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  let menu_of = |app: &mut App, i: usize| {
    app.close_tab_menu();
    app.bar_dirty = true;
    app.redraw();
    let p = app.bar_map.iter().position(|&t| t == i).expect("in the bar");
    let r = app.bar_hits[p];
    app.pointer = (r.x + r.w / 2, r.y + r.h / 2);
    app.right_click();
    app.tab_menu().map(|m| m.labels())
  };
  app.tabs[flood].event = false;
  app.tabs[flood].unread = false;
  assert_eq!(menu_of(&mut app, flood), Some(vec!["edit".to_owned(), "remove".to_owned()]));
  // j then enter picks remove's neighbour: edit is first, so enter opens the form
  app.tab_menu_key(Key::Named(NamedKey::Enter));
  let f = app.form.as_ref().expect("edit opens the form");
  let names: Vec<_> = f.acts.iter().map(|a| a.1.as_str()).collect();
  assert!(names.contains(&"summary") && names.contains(&"watch"), "{names:?}");
  // tab walks past the three fields onto the buttons; hjkl move there, enter runs
  for _ in 0..3 {
    app.form_key(Key::Named(NamedKey::Tab), None);
  }
  let at = |app: &App| app.form.as_ref().map(|f| f.focus);
  assert_eq!(at(&app), Some(3));
  app.form_key(Key::Character("j"), Some("j"));
  assert_eq!(at(&app), Some(4));
  app.form_key(Key::Character("k"), Some("k"));
  assert_eq!(at(&app), Some(3));
  let sum = app.form.as_ref().unwrap().acts.iter().position(|a| a.0 == crate::model::chan::Act::Summary).unwrap();
  app.form.as_mut().unwrap().focus = 3 + sum;
  app.redraw();
  crate::bench::dump(&app.canvas);
  app.form_key(Key::Named(NamedKey::Enter), None);
  assert!(app.form.is_none(), "running a button closes the form");
  assert_eq!(app.screen.as_ref().map(|s| s.page()), Some(crate::app::screen::Page::Summary));
}

// your account's tab list, saved by another client, changes the tabs here: an open tab keeps its lines, a new
// one joins, a gone one leaves, and none of it is sent back. a stale list changes nothing; a change made here
// goes up with what the other client stored; logged out, nothing comes in:
//   cargo test -j2 --release headless_tab_sync
#[test]
fn headless_tab_sync() {
  use crate::model::tabsync::frame;
  use crate::net::hs_ws::Cmd as HsCmd;
  use serde_json::json;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
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
  let xqc = app.tabs.iter().position(|t| &*t.name == "xqc").unwrap();
  app.tabs[xqc].view.push(Message::system(0, Platform::Twitch, "kept"));
  let kept = app.tabs[xqc].view.len();
  while asked.try_recv().is_ok() {}
  let names = |app: &App| app.channel_specs().iter().map(|s| s.name()).collect::<Vec<_>>();
  let push = |app: &mut App, v: serde_json::Value| app.on_net(NetEvent::TabSync(frame(&v).unwrap()));
  push(
    &mut app,
    json!({"type": "multichat:config", "rev": 5, "channels": [
      {"id": "xqc", "twitch": "xqc", "sendTargets": {"twitch": true}},
      {"id": "trainwreckstv", "kick": "trainwreckstv"},
      {"id": "yt-1", "youtube": "UCabcdefghijklmnopqrstuv"},
      {"id": "yt-2", "youtube": "lofigirl"},
    ]}),
  );
  assert_eq!(names(&app), ["xqc", "kick:trainwreckstv", "yt:@lofigirl"]);
  let xqc = app.tabs.iter().position(|t| &*t.name == "xqc").unwrap();
  assert_eq!(app.tabs[xqc].view.len(), kept, "an open tab keeps its lines");
  let cmds: Vec<Cmd> = std::iter::from_fn(|| asked.try_recv().ok()).collect();
  assert!(cmds.iter().any(|c| matches!(c, Cmd::Part(k) if &**k == "forsen")), "forsen left");
  assert!(cmds.iter().any(|c| matches!(c, Cmd::Hs(HsCmd::Join(Platform::Kick, k)) if &**k == "trainwreckstv")));
  assert!(!cmds.iter().any(|c| matches!(c, Cmd::Hs(HsCmd::TabSync(_)))), "the account's own list is not sent back");
  assert_eq!(
    crate::store::load_tabs(),
    ["xqc".into(), "kick:trainwreckstv".into(), "yt:@lofigirl".into()] as [Box<str>; 3]
  );
  // a list read before that one, landing after it, is dropped
  push(&mut app, json!({"type": "multichat:config", "rev": 4, "channels": [{"id": "forsen", "twitch": "forsen"}]}));
  assert_eq!(names(&app), ["xqc", "kick:trainwreckstv", "yt:@lofigirl"]);
  // a tab closed here goes up, with what the other client stored riding along
  let lofi = app.tabs.iter().position(|t| &*t.name == "yt:@lofigirl").unwrap();
  app.remove_tab(lofi);
  let sent: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
    .filter_map(|c| if let Cmd::Hs(HsCmd::TabSync(l)) = c { Some(l) } else { None })
    .collect();
  assert_eq!(
    sent,
    [vec![
      json!({"id": "xqc", "twitch": "xqc", "sendTargets": {"twitch": true}}),
      json!({"id": "trainwreckstv", "kick": "trainwreckstv"}),
      json!({"id": "yt-1", "youtube": "UCabcdefghijklmnopqrstuv"}),
    ]]
  );
  // logged out: the account's lists no longer reach the tabs
  app.on_net(NetEvent::Me(None));
  push(&mut app, json!({"type": "multichat:config", "rev": 9, "channels": []}));
  assert_eq!(names(&app), ["xqc", "kick:trainwreckstv"]);
}
