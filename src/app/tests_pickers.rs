// the whole app headless, part 2: cards, chat logs, the emoji and gif pickers

use super::*;
use crate::model::message::Message;
use crate::model::picker::Focus;
use crate::net::Cmd;
use winit::keyboard::Key;
use winit::keyboard::NamedKey;

fn dump_frame(app: &App) {
  if let Some(path) = std::env::var_os("HSC_DUMP") {
    let c = &app.canvas;
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(path, out);
  }
}

// `:smi` lists emotes first, then emoji (glyph and :shortcode:); picking puts the glyph in the box;
// a typed :shortcode: becomes the glyph on send. HSC_DUMP=$d/x.ppm (the list) ... cargo test -j2
// --release headless_picker_emoji -- --ignored  (scratch HOME as above)
// a stream line made before the streamer is known gets their color and twitch id when a lookup or the
// followed colors land; the frame's own color is never overruled; followed colors beat chat's
#[test]
fn stream_lines_resolve_late() {
  use crate::model::card::Info;
  use crate::model::color;
  use crate::model::stream_events::{Line, Tone};
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  let line = |ch: &str, color| Line {
    platform: Platform::Twitch,
    channel: ch.into(),
    text: "x".into(),
    tone: Tone::Switch,
    name: ch.into(),
    color,
    verb: "◆ switched to ".into(),
    game: "Chess".into(),
  };
  let ft = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  let last = |app: &App| app.tabs[ft].view.last_msg().map(|m| (m.color, m.user_id.to_string())).unwrap();
  app.on_stream(line("knut", None));
  app.on_stream(line("framed", Some(0x00ff00)));
  let framed = last(&app);
  assert_eq!(framed.0, color::readable(0x00ff00));
  app.tabs[ft].view.restyle_late(|_, _| (1, None));
  assert_eq!(last(&app), framed, "a frame's own color is not late");
  // before: palette color, no id
  app.on_stream(line("knut", None));
  let before = last(&app);
  assert_eq!(before, (color::readable(color::default_for("knut")), String::new()));
  // a lookup lands: chat's color + the id for badges and paints
  let info = Info { color: Some(0xff0000), twitch_id: Some(4242), ..Info::default() };
  app.learn_card(Platform::Twitch, "knut", Ok(info));
  assert_eq!(last(&app), (color::readable(0xff0000), "4242".to_string()));
  // followed colors outrank it
  app.on_follow_colors(vec![("knut".into(), 0x00ffff)]);
  assert_eq!(last(&app), (color::readable(0x00ffff), "4242".to_string()));
  // and a brand-new line follows the same chain
  app.on_stream(line("knut", None));
  assert_eq!(last(&app), (color::readable(0x00ffff), "4242".to_string()));
}

#[test]
fn a_height_only_resize_paints_the_chat_again() {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(420, 300);
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  for n in 0..40 {
    app.tabs[chat].chat().push(Message::system(now_ms(), Platform::Twitch, format!("line {n} with some words")));
  }
  app.redraw();
  let lit = |app: &App| {
    let a = app.chat_area();
    (a.y..a.bottom())
      .flat_map(|y| (a.x..a.right()).map(move |x| (x, y)))
      .filter(|&(x, y)| app.canvas.px[(y * app.canvas.w + x) as usize] != 0)
      .count()
  };
  let before = lit(&app);
  assert!(before > 500, "the chat drew ({before} px)");
  // a new window tiles beside this one: same width, shorter
  app.resized(420, 240);
  app.redraw();
  assert!(lit(&app) > 300, "the chat paints again after a height-only resize, not only its animations");
}

#[test]
fn the_card_asks_pronoundb_once_and_shows_the_answer_beside_the_name() {
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.set.pronouns = true;
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  let person = |login: &str, uid: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, "hi");
    m.kind = crate::model::message::Kind::Chat;
    (m.login, m.nick, m.user_id) = (login.into(), login.into(), uid.into());
    m
  };
  let alice = person("alice", "42");
  let who = app.subject_of(&alice).expect("a chatter");
  let drain = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
    std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| if let Cmd::Pronouns(u) = c { Some(u) } else { None })
      .collect::<Vec<_>>()
  };
  app.open_card(who.clone());
  assert_eq!(drain(&mut asked), vec![42], "her line's twitch id is asked about, at once");
  assert_eq!(app.pronouns_of(app.card_twitch_id()), None, "nothing shows until it lands");
  app.on_net(NetEvent::Pronouns(42, Ok(Some("she/they".into()))));
  assert_eq!(app.pronouns_of(app.card_twitch_id()), Some("she/they"));
  app.redraw();
  // closed and opened again: the answer is kept, nothing is asked
  app.card = None;
  app.open_card(who);
  assert!(drain(&mut asked).is_empty());
  // switched off: nothing shows, nobody new is asked about
  app.set.pronouns = false;
  assert_eq!(app.pronouns_of(app.card_twitch_id()), None);
  app.open_card(app.subject_of(&person("bob", "43")).unwrap());
  assert!(drain(&mut asked).is_empty(), "pronoundb never hears of bob");
}

#[test]
fn chat_logs_left_open_on_one_tab_keep_their_keys_to_that_tab() {
  let (net, _asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
  (app.set.vi, app.mode) = (true, Mode::Normal);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  let tab = |app: &App, n: &str| app.tabs.iter().position(|t| &*t.name == n).unwrap();
  let (a, b) = (tab(&app, "forsen"), tab(&app, "xqc"));
  let line = |id: &str| {
    let mut m = Message::system(now_ms(), Platform::Twitch, "hello");
    m.kind = crate::model::message::Kind::Chat;
    (m.id, m.login, m.nick) = (id.into(), "bob".into(), "bob".into());
    m
  };
  app.select(a);
  app.tabs[a].chat().push(line("1"));
  let who = app.subject_of(&line("1")).unwrap();
  app.logs_open(&who);
  assert!(app.logs_shown().is_some());
  // off to another tab without closing them: its lines answer r and m as always
  app.select(b);
  app.tabs[b].chat().push(line("2"));
  app.redraw();
  assert!(app.logs_shown().is_none());
  app.on_key(Key::Character("v"), Some("v"));
  app.on_key(Key::Character("r"), Some("r"));
  assert_eq!(app.reply.as_ref().map(|r| &*r.id), Some("2"), "r replies here, it doesn't refresh the hidden logs");
  app.mode = Mode::Normal;
  app.on_key(Key::Character("m"), Some("m"));
  assert!(app.pop.is_some(), "the line menu opens");
  app.pop = None;
  assert!(!app.hint().contains("y copy link"), "the box speaks for this tab");
  // back on their tab they are as left, and their keys work
  app.select(a);
  assert!(app.logs_shown().is_some());
  app.on_key(Key::Character("q"), Some("q"));
  assert!(app.logs.is_none() && app.tabs[a].under.is_none(), "q closes them");
}

#[test]
#[ignore]
fn headless_picker_emoji() {
  use crate::emotes::{Emote, Provider};
  use crate::model::suggest::Kind as S;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  let e = |url: &str| Emote { url: url.into(), w: 28, h: 28, zero_width: false };
  app.res.emotes.set(None, Provider::SevenTv, [("smirkCat".into(), e("https://cdn.7tv.app/emote/a/1x.webp"))].into());
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  app.enter_insert();
  let key = |app: &mut App, k: NamedKey| app.key_insert(Key::Named(k), None);
  app.line.insert("so :smi");
  app.input_dirty = true;
  app.redraw();
  let s = app.suggest.as_ref().expect("the list opens");
  assert_eq!(s.kind, S::Emote);
  assert_eq!(&*s.items[0].name, "smirkCat", "emotes first, like the ext");
  let first_emoji = s.items.iter().position(|p| p.code.is_some()).expect("emoji follow");
  assert!(first_emoji >= 1);
  assert_eq!(s.items[first_emoji].code, Some("smile"), "shortest prefix first");
  assert!(app.over_input && !app.suggest_rows.is_empty());
  dump_frame(&app);
  // the glyph is what goes in the box
  app.suggest.as_mut().unwrap().sel = first_emoji;
  key(&mut app, NamedKey::Enter);
  assert_eq!(app.line.text, "so 😄 ");
  assert!(app.line.chips.is_empty(), "an emoji is text, not a picture chip");
  assert!(app.complete.as_ref().and_then(|c| c.tip()).unwrap().contains(":smile:"), "the tip names the code");
  // tab with a bare :word cycles emoji too
  app.line.take();
  app.complete = None;
  app.line.insert(":joy");
  app.tab_complete(false);
  assert!(app.line.text.starts_with('😂'), "{}", app.line.text);
  // send turns shortcodes into glyphs
  app.complete = None;
  app.line.take();
  app.line.insert("gg :fire: :100: :nope:");
  while asked.try_recv().is_ok() {}
  app.send();
  let sent = std::iter::from_fn(|| asked.try_recv().ok()).find_map(|c| match c {
    Cmd::Send { text, .. } => Some(text),
    _ => None,
  });
  assert_eq!(sent.as_deref(), Some("gg 🔥 💯 :nope:"));
}

// the picker's gifs tab: ctrl-tab / [ ] switch, typing asks heatsync after a rest (old answers drop),
// an answer fills a grid of stills, the rested pick plays, enter inserts the link and keeps recents.
// HSC_DUMP=$d/x.ppm (the grid) ... cargo test -j2 --release headless_picker_gifs -- --ignored
#[test]
#[ignore]
fn headless_picker_gifs() {
  use crate::images::decode::{Decoded, Frame};
  use crate::model::gifs;
  use crate::model::picker::Tab;
  use crate::net::social;
  use serde_json::json;
  crate::store::remove("recent-gifs");
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 400);
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  app.select(chat);
  app.enter_insert();
  let gif_asks = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
    std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| match c {
        Cmd::Social(social::Cmd::Gifs { q, epoch }) => Some((q, epoch)),
        _ => None,
      })
      .collect::<Vec<_>>()
  };
  let page = |tag: &str, n: u32| {
    let rows: Vec<_> = (0..n)
      .map(|i| {
        json!({"id": i, "provider": "giphy", "label": format!("{tag} {i}"),
        "url": format!("https://media.giphy.com/media/{tag}{i}/giphy.gif"),
        "preview": format!("https://media.giphy.com/media/{tag}{i}/200_s.gif"),
        "animated": format!("https://media.giphy.com/media/{tag}{i}/200.gif")})
      })
      .collect();
    gifs::parse(&json!({"gifs": rows, "library": {"state": "ready", "indexed": 9}}))
  };
  // every picture asked for lands as a flat tile, one color per url
  let land = |app: &mut App, asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| -> Vec<String> {
    let mut urls = Vec::new();
    for _ in 0..3 {
      app.redraw();
      let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
        .filter_map(|c| if let Cmd::Image(id, url, target) = c { Some((id, url, target)) } else { None })
        .collect();
      if wanted.is_empty() {
        break;
      }
      for (id, url, target) in wanted {
        let tone = url.bytes().fold(60u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        let px = [(tone & 0xff) as u8 | 0x40, (tone >> 8 & 0xff) as u8 | 0x40, (tone >> 16 & 0xff) as u8 | 0x40, 255];
        let d = Decoded { w: 80, h: 45, frames: vec![Frame { rgba: px.repeat(80 * 45).into(), delay_ms: 0 }] };
        urls.push(url.to_string());
        app.res.images.loaded(id, target, Ok(d));
      }
    }
    urls
  };
  app.toggle_picker();
  assert_eq!(app.picker.as_ref().unwrap().tab, Tab::Emotes);
  // grid focus: ] goes to gifs; a cold box with no recents asks for the top list at once
  app.picker.as_mut().unwrap().focus = Focus::Grid;
  app.picker_key(Key::Character("]"), Some("]"));
  assert_eq!(app.picker.as_ref().unwrap().tab, Tab::Gifs);
  let top = gif_asks(&mut asked);
  assert_eq!(top.len(), 1, "{top:?}");
  assert_eq!(&*top[0].0, "");
  app.redraw();
  assert_eq!(app.picker.as_ref().unwrap().note, "searching…");
  // a stale answer (older epoch) is dropped; the right one fills the grid
  app.on_net(NetEvent::Gifs { epoch: top[0].1 + 7, q: "".into(), res: Ok(page("old", 3)) });
  assert!(app.picker.as_ref().unwrap().selected_gif().is_none());
  app.on_net(NetEvent::Gifs { epoch: top[0].1, q: "".into(), res: Ok(page("top", 24)) });
  assert_eq!(app.picker.as_ref().unwrap().selected_gif().map(|g| &*g.label), Some("top 0"));
  assert!(app.picker.as_ref().unwrap().note.is_empty());
  // typing asks once after a rest, not per key; a bad answer says why
  app.picker.as_mut().unwrap().focus = Focus::Search;
  for c in ["c", "a", "t"] {
    app.picker_key(Key::Character(c), Some(c));
  }
  assert!(gif_asks(&mut asked).is_empty(), "nothing before the rest is over");
  app.gifs_tick(Instant::now() + Duration::from_secs(1));
  let ask = gif_asks(&mut asked);
  assert_eq!(ask.len(), 1);
  assert_eq!(&*ask[0].0, "cat");
  app.on_net(NetEvent::Gifs {
    epoch: ask[0].1,
    q: "cat".into(),
    res: Err(gifs::Fail::of(429, &json!({"resetAt": now_ms() + 7_500}), now_ms())),
  });
  assert!(app.picker.as_ref().unwrap().note.contains("wait 8s"), "{}", app.picker.as_ref().unwrap().note);
  // the same text tries again on the next change
  app.picker_key(Key::Named(NamedKey::Backspace), None);
  app.picker_key(Key::Character("t"), Some("t"));
  app.gifs_tick(Instant::now() + Duration::from_secs(1));
  let ask = gif_asks(&mut asked);
  assert_eq!(ask.len(), 1);
  app.on_net(NetEvent::Gifs { epoch: ask[0].1, q: "cat".into(), res: Ok(page("cat", 24)) });
  // the grid paints stills for what is in view; the selected one has not started to play
  let urls = land(&mut app, &mut asked);
  assert!(!urls.is_empty() && urls.len() <= 24, "{}", urls.len());
  assert!(
    urls.iter().all(|u| u.starts_with("https://heatsync.org/api/img?url=") && u.contains("200_s.gif")),
    "{urls:?}"
  );
  dump_frame(&app);
  // a rested pick asks for its moving picture, once
  app.picker_key(Key::Named(NamedKey::ArrowRight), None);
  app.gifs_tick(Instant::now() + Duration::from_secs(1));
  assert!(app.picker.as_ref().unwrap().hot);
  let moving = land(&mut app, &mut asked);
  assert_eq!(moving.len(), 1, "{moving:?}");
  assert!(moving[0].contains("%2F200.gif"), "{moving:?}");
  // the same search again comes from the cache, no request
  app.picker.as_mut().unwrap().clear_query();
  app.picker_changed();
  app.gifs_tick(Instant::now() + Duration::from_secs(1));
  assert_eq!(app.picker.as_ref().unwrap().selected_gif().map(|g| &*g.label), Some("top 0"), "the top list, cached");
  app.picker.as_mut().unwrap().type_str("cat");
  app.picker_changed();
  app.gifs_tick(Instant::now() + Duration::from_secs(1));
  assert!(gif_asks(&mut asked).is_empty());
  assert_eq!(app.picker.as_ref().unwrap().selected_gif().map(|g| &*g.label), Some("cat 0"));
  // enter puts the link in the box, closes, and remembers it
  app.picker_key(Key::Named(NamedKey::ArrowRight), None);
  app.picker_key(Key::Named(NamedKey::Enter), None);
  assert!(app.picker.is_none());
  assert_eq!(app.line.text, "https://media.giphy.com/media/cat1/giphy.gif ");
  assert_eq!(app.gifs.recent.as_ref().map(|r| r.len()), Some(1));
  // reopening: the box is on the gifs you sent, no request; ctrl-tab goes back to emotes
  app.line.take();
  app.toggle_picker();
  app.picker_tab(Some(Tab::Gifs));
  assert!(gif_asks(&mut asked).is_empty());
  let g = app.picker.as_ref().unwrap().selected_gif().cloned().unwrap();
  assert_eq!(&*g.url, "https://media.giphy.com/media/cat1/giphy.gif");
  app.redraw();
  app.picker_tab(None);
  assert_eq!(app.picker.as_ref().unwrap().tab, Tab::Emotes);
  app.close_picker();
  assert!(app.gifs.due().is_none());
}
