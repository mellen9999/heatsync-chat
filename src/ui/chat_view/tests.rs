use super::layout::post_refs;
use super::*;
use crate::fake::Fake;
use crate::ui::canvas::Canvas;
use crate::ui::res::{EMBED_PX, Look};
use crate::ui::theme;

#[test]
fn post_refs_are_whole_words() {
  let ids = |s: &str| post_refs(s, 0).into_iter().map(|r| r.1.into_string()).collect::<Vec<_>>();
  assert_eq!(ids(">>3t and >>4a, not a>>5 or >>xyz1234 or >>Ab"), ["00003t", "00004a"]);
  assert_eq!(post_refs("x >>9", 2)[0].0, 2..5);
  assert!(ids(">> >>").is_empty());
}

#[test]
fn picture_waits_for_its_size_then_folds_the_link() {
  use crate::images::decode::{Decoded, Frame};
  let mut res = Res::new(1.0);
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.nick = "bob".into();
  m.body = "look https://i.imgur.com/a.png".into();
  let boxes = |res: &mut Res| layout_row(res, "c", &m, false, 400, NO_OPTS);
  // not loaded: no box, link stays as text, and nothing was requested by measuring
  let l = boxes(&mut res);
  assert!((l.boxes.len(), l.embed.is_some()) == (0, true));
  assert!(res.images.take_wanted().is_empty());
  // drawing asks for it, fit to the row width
  res.images.want(l.embed.unwrap());
  let (id, url, target) = res.images.take_wanted().remove(0);
  assert!(url.starts_with("https://heatsync.org/api/img?url="));
  assert_eq!((target & 0xffff, target >> 16), (EMBED_PX as u32, 400 - MARK_W as u32 - 8));
  let d = Decoded { w: 120, h: 80, frames: vec![Frame { rgba: vec![0; 120 * 80 * 4].into(), delay_ms: 0 }] };
  assert!(res.images.loaded(id, target, Ok(d)));
  let l = boxes(&mut res);
  assert_eq!(l.boxes.len(), 1);
  // the same size again needs no relayout; a failure removes the slot
  assert!(!res.images.loaded(id, target, Err("x".into())));
  assert_eq!(boxes(&mut res).boxes.len(), 0);
  res.media = false;
  assert!(boxes(&mut res).embed.is_none());
}

#[test]
fn seventv_badge_follows_the_cosmetics_answer() {
  use crate::badges::Badge;
  use crate::cosmetics::Cosmetic;
  let mut res = Res::new(1.0);
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.user_id = "42".into();
  m.nick = "bob".into();
  m.body = "hi".into();
  let n = |res: &mut Res| layout_row(res, "c", &m, false, 400, NO_OPTS).boxes.len();
  // unknown chatter: no badge yet, but the ask is queued
  assert_eq!(n(&mut res), 0);
  assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(42)]);
  let badge = Badge { url: "https://cdn.7tv.app/badge/B/2x.webp".into(), title: "t".into(), bg: None };
  res.cosmetics.put(
    &crate::cosmetics::id::PaintId::Twitch(42),
    Some(Cosmetic { paint: None, badge: Some(badge), ..Default::default() }),
  );
  assert_eq!(n(&mut res), 1);
  res.cosmetics.on = false;
  assert_eq!(n(&mut res), 0);
}

#[test]
fn bttv_badge_shows_for_its_holder_behind_its_switch() {
  use crate::badges::{Badge, Source, UserBadges};
  let mut res = Res::new(1.0);
  let badge = Badge { url: "https://cdn.betterttv.net/badges/developer.svg".into(), title: "dev".into(), bg: None };
  res.badges.set_users(Source::Bttv, UserBadges { list: vec![badge], users: [(42, vec![0])].into() });
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.user_id = "42".into();
  m.nick = "bob".into();
  m.body = "hi".into();
  let n = |res: &mut Res, m: &Message| layout_row(res, "c", m, false, 400, NO_OPTS).boxes.len();
  assert_eq!(n(&mut res, &m), 1);
  let mut other = m.clone();
  other.user_id = "43".into();
  assert_eq!(n(&mut res, &other), 0, "only its holder");
  res.look.bttv_badges = false;
  assert_eq!(n(&mut res, &m), 0, "bttv badges off");
}

#[test]
fn text_badges_swap_twitch_pictures_for_words_and_off_is_unchanged() {
  use crate::badges::{Badge, key};
  let mut res = Res::new(1.0);
  let b = |u: &str| Badge { url: u.into(), title: "".into(), bg: None };
  res.badges.set_twitch(
    None,
    [
      (key("moderator", "1"), b("https://static-cdn.jtvnw.net/m")),
      (key("subscriber", "0"), b("https://static-cdn.jtvnw.net/s")),
    ]
    .into(),
  );
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.nick = "bob".into();
  m.body = "hi".into();
  m.badges =
    vec![("moderator".into(), "1".into()), ("subscriber".into(), "0".into()), ("first-msg".into(), "1".into())].into();
  let at = |res: &mut Res| {
    let l = layout_row(res, "c", &m, false, 400, NO_OPTS);
    (l.boxes.len(), l.nick.start)
  };
  // off: two pictures, the name starts the text
  assert_eq!(at(&mut res), (2, 0));
  // on: "mod" and "sub" are boxed words before the name, no pictures, the unstyled role draws nothing
  res.look.text_badges = true;
  assert_eq!(at(&mut res), (0, " mod  sub ".len()));
  res.look.text_badges = false;
  assert_eq!(at(&mut res), (2, 0));
}

#[test]
fn painted_name_draws_shaded_and_only_the_name() {
  use crate::cosmetics::{Cosmetic, paint::Paint as P};
  let mut res = Res::new(1.0);
  let paint = P::parse(&serde_json::json!({"id": "G", "function": "linear-gradient", "angle": 90,
    "stops": [{"at": 0, "color": 0xff0000ffu32 as i32}, {"at": 1, "color": 0x0000ffffu32 as i32}]}))
  .unwrap();
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.user_id = "42".into();
  m.nick = "wwwwwwwwwwww".into();
  m.color = theme::GREEN;
  m.body = "wwwwwwwwwwww".into();
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(400);
  v.push(m);
  let area = Rect::new(0, 0, 400, 40);
  let mut c = Canvas::new(400, 40);
  // unknown chatter: plain green name, ask queued
  v.draw(&mut c, &mut res, area, area);
  assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(42)]);
  let tint = |c: &Canvas| {
    let (mut red, mut blue, mut green) = (0, 0, 0);
    for p in &c.px {
      let (r, g, b) = (p >> 16 & 255, p >> 8 & 255, p & 255);
      red += (r > 200 && g < 60 && b < 100) as u32;
      blue += (b > 200 && r < 100 && g < 60) as u32;
      green += (g > 200 && r < 60 && b < 60) as u32;
    }
    (red, blue, green)
  };
  let (r0, b0, g0) = tint(&c);
  assert!(g0 > 20 && r0 == 0 && b0 == 0);
  res.cosmetics.put(
    &crate::cosmetics::id::PaintId::Twitch(42),
    Some(Cosmetic { paint: Some(Arc::new(paint)), badge: None, ..Default::default() }),
  );
  v.touch_sender("42");
  v.draw(&mut c, &mut res, area, area);
  let (r1, b1, g1) = tint(&c);
  // the name now runs red to blue; the body line is untouched (still plain white, no green left)
  assert!(r1 > 10 && b1 > 10 && g1 == 0, "{r1} {b1} {g1}");
  assert!(res.arts.len() == 1);
  // switching paints off puts the plain name back
  res.cosmetics.on = false;
  v.relayout();
  v.draw(&mut c, &mut res, area, area);
  assert_eq!(tint(&c).2, g0);
}

#[test]
fn heatsync_native_solid_paint_colors_the_name() {
  use crate::cosmetics::{Cosmetic, paint::Paint as P};
  for zoom in [1.0, 1.3, 2.0] {
    let mut res = Res::new(zoom);
    let spec = serde_json::json!({"v": 2, "base": {"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]},
    "glow": null, "effects": [{"id": "twirl", "speed": 1}]});
    let paint = P::from_hs(&spec).unwrap();
    // a real nl_kripp history line, through the real irc parser
    let raw = "@flags;subscriber=1;first-msg=0;rm-received-ts=1793376500527;display-name=ennortix;emotes;color=#FF0000;turbo=0;tmi-sent-ts=1793376500347;user-type;mod=0;badge-info=subscriber/55;user-id=37005319;badges=subscriber/2048,ewc-2026-diamond/1;id=2f3d789f;room-id=29795919;historical=1 :ennortix!ennortix@ennortix.tmi.twitch.tv PRIVMSG #nl_kripp :watching the mentalist while gaming Ratting";
    let (_, crate::net::irc::Chat::Msg(m)) = crate::net::irc::to_chat(&crate::net::irc::parse(raw).unwrap()).unwrap()
    else {
      panic!()
    };
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.resize(400);
    v.push(m);
    let area = Rect::new(0, 0, 400, 40);
    let mut c = Canvas::new(400, 40);
    v.draw(&mut c, &mut res, area, area);
    assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(37005319)]);
    res.cosmetics.put(
      &crate::cosmetics::id::PaintId::Twitch(37005319),
      Some(Cosmetic { paint: Some(Arc::new(paint)), badge: None, ..Default::default() }),
    );
    v.touch_sender("37005319");
    v.draw(&mut c, &mut res, area, area);
    let amber = c.px.iter().filter(|p| (*p >> 16 & 255) > 200 && (*p >> 8 & 255) > 120 && (*p & 255) < 60).count();
    let red = c.px.iter().filter(|p| (*p >> 16 & 255) > 200 && (*p >> 8 & 255) < 60 && (*p & 255) < 60).count();
    assert!(amber > 20 && red == 0, "zoom {zoom} amber {amber} red {red}");
  }
}

// ennortix's real paint: an animated name ticks only while it is on screen, and never with motion off
#[test]
fn animated_names_tick_only_while_visible() {
  use crate::cosmetics::{Cosmetic, paint::Paint as P};
  let spec = serde_json::json!({"v": 2, "base": {"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]},
    "glow": null, "scene": {"weather": {"id": "petals", "tint": "#d7408c", "speed": 1, "density": 1},
    "backdrop": {"id": "synth", "tint": "#0ca43d", "speed": 1}}, "effects": [{"id": "twirl", "speed": 1}, {"id": "coin", "speed": 1}]});
  let paint = Arc::new(P::from_hs(&spec).unwrap());
  let mut m = said("ennortix", "gg");
  m.user_id = "37005319".into();
  let setup = |motion: bool| {
    let mut res = Res::new(1.0);
    res.arts.set_motion(motion);
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.resize(400);
    v.push(m.clone());
    let area = Rect::new(0, 0, 400, 40);
    let mut c = Canvas::new(400, 40);
    v.draw(&mut c, &mut res, area, area);
    res.cosmetics.put(
      &crate::cosmetics::id::PaintId::Twitch(37005319),
      Some(Cosmetic { paint: Some(paint.clone()), badge: None, ..Default::default() }),
    );
    v.touch_sender("37005319");
    v.draw(&mut c, &mut res, area, area);
    (res, v, c, area)
  };
  let (mut res, mut v, mut c, area) = setup(true);
  assert_eq!(v.next_anim, Some(TICK_SMOOTH_MS), "a visible animated name asks for ticks");
  // a viewport that does not contain the name: no repaint, no further ticks
  let out = v.animate(&mut c, &mut res, Rect::new(300, 0, 100, 40));
  assert!(out.is_none() && v.next_anim.is_none(), "off-screen: idle");
  // on screen: only the name's own rect is damaged
  let (mut res, mut v, mut c, area2) = setup(true);
  let r = v.animate(&mut c, &mut res, area2).expect("damage");
  assert!(r.w < area.w && r.h <= 40);
  assert_eq!(v.next_anim, Some(TICK_SMOOTH_MS));
  // motion off: a still frame, nothing ever ticks
  let (mut res, mut v, mut c, area) = setup(false);
  assert_eq!(v.next_anim, None);
  assert!(v.animate(&mut c, &mut res, area).is_none());
}

const NO_OPTS: Opts = Opts { ts: false, tags: false, find: None };

// a feed post: its author's twitch id asks for their paint, the paint animates like in chat, and
// only the ▲ (not the heat or the replies beside it) answers a vote click
// archived history comes without ids: once one is known those lines take it and ask for the paint,
// but never someone else's lines or another platform's
#[test]
fn lines_without_an_id_adopt_it_and_ask_for_the_paint() {
  let line = |p: Platform, login: &str, uid: &str| {
    let mut m = Message::system(0, p, "hi");
    m.kind = Kind::Chat;
    m.login = login.into();
    m.nick = login.into();
    m.user_id = uid.into();
    m
  };
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(10, "x".into());
  v.resize(500);
  v.push(line(Platform::Twitch, "ennortix", ""));
  v.push(line(Platform::Twitch, "alice", ""));
  v.push(line(Platform::Kick, "ennortix", ""));
  let area = Rect::new(0, 0, 500, 200);
  let mut c = Canvas::new(500, 200);
  v.draw(&mut c, &mut res, area, area);
  assert!(res.cosmetics.take_wanted().is_empty(), "no ids, nothing to ask");
  v.adopt_id("ennortix", "37005319");
  v.draw(&mut c, &mut res, area, area);
  assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(37005319)]);
  let ids: Vec<&str> = v.rows.iter().map(|r| &*r.msg.user_id).collect();
  assert_eq!(ids, vec!["37005319", "", ""]);
}

#[test]
fn feed_rows_wear_animated_paints_and_the_arrow_alone_votes() {
  use crate::cosmetics::{Cosmetic, paint::Paint as P};
  let spec = serde_json::json!({"v": 2, "base": {"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]},
    "glow": null, "scene": null, "effects": [{"id": "twirl", "speed": 1}]});
  let paint = Arc::new(P::from_hs(&spec).unwrap());
  let post = crate::model::post::parse(&serde_json::json!({"base36_id": "00abc1", "username": "ennortix",
    "user_id": "37005319", "platform": "twitch", "content": "gg", "heat": 20, "reply_count": 2}))
  .unwrap();
  let mut res = Res::new(1.0);
  res.look.timestamps = true;
  let mut v = ChatView::new(10, "feed".into());
  v.top = true;
  v.zebra = false;
  v.resize(500);
  v.push(crate::model::post::message(&post, Some("me"), true));
  let area = Rect::new(0, 0, 500, 40);
  let mut c = Canvas::new(500, 40);
  v.draw(&mut c, &mut res, area, area);
  assert_eq!(res.cosmetics.take_wanted(), vec![crate::cosmetics::id::PaintId::Twitch(37005319)]);
  res.cosmetics.put(
    &crate::cosmetics::id::PaintId::Twitch(37005319),
    Some(Cosmetic { paint: Some(paint), badge: None, ..Default::default() }),
  );
  v.touch_sender("37005319");
  v.draw(&mut c, &mut res, area, area);
  v.draw(&mut c, &mut res, area, area);
  assert_eq!(v.next_anim, Some(TICK_SMOOTH_MS), "a painted feed name ticks while on screen");
  let hits: Vec<i32> = (0..500).filter(|&x| v.vote_at(area, x, 8, &mut res).is_some()).collect();
  assert!(!hits.is_empty(), "the arrow is hittable");
  assert!(hits.windows(2).all(|w| w[1] == w[0] + 1) && hits.len() < 30, "only the arrow: {hits:?}");
  assert_eq!(v.vote_at(area, hits[0], 8, &mut res).as_deref(), Some("00abc1"));
  assert!(v.vote_at(area, 2, 8, &mut res).is_none());
  // the >>id alone opens the thread: a short run at the line's start, not the name or the arrow
  let ids: Vec<i32> = (0..500).filter(|&x| v.thread_at(area, x, 8, &mut res).is_some()).collect();
  assert!(!ids.is_empty() && ids.windows(2).all(|w| w[1] == w[0] + 1) && ids.len() < 60, "only >>abc1: {ids:?}");
  assert_eq!(v.thread_at(area, ids[0], 8, &mut res).as_deref(), Some("00abc1"));
  assert!(!ids.contains(&hits[0]));
  // logged out: no arrow, nothing to hit
  let mut v = ChatView::new(10, "feed".into());
  v.top = true;
  v.resize(500);
  v.push(crate::model::post::message(&post, Some("me"), false));
  assert!((0..500).all(|x| v.vote_at(area, x, 8, &mut res).is_none()));
}

// the same number from twitch, kick and youtube asks for three different people's paints,
// and a paint arriving for one dresses only that one's lines
#[test]
fn chat_lines_ask_for_paints_in_their_own_namespace() {
  use crate::cosmetics::{Cosmetic, id::PaintId, paint::Paint as P};
  let spec = serde_json::json!({"v": 2, "base": {"type": "solid", "angle": 0, "stops": [{"pos": 0, "color": "#ffb000"}]}, "glow": null, "scene": null, "effects": []});
  let paint = Arc::new(P::from_hs(&spec).unwrap());
  let yt = "UC0123456789abcdefghij_-";
  let line = |p: Platform, id: &str| {
    let mut m = said("bob", "hi");
    m.platform = p;
    m.user_id = id.into();
    m
  };
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(500);
  for m in
    [line(Platform::Twitch, "5"), line(Platform::Kick, "5"), line(Platform::Youtube, yt), line(Platform::Kick, "")]
  {
    v.push(m);
  }
  let area = Rect::new(0, 0, 500, 120);
  let mut c = Canvas::new(500, 120);
  v.draw(&mut c, &mut res, area, area);
  let mut asked = res.cosmetics.take_wanted();
  asked.sort_by_key(PaintId::wire);
  assert_eq!(asked, vec![PaintId::Twitch(5), PaintId::Kick(5), PaintId::Youtube(yt.into())]);
  // kick 5 wears a paint: twitch 5's line stays plain
  res.cosmetics.put(&PaintId::Kick(5), Some(Cosmetic { paint: Some(paint), badge: None, ..Default::default() }));
  assert!(res.cosmetics.get(&PaintId::Twitch(5)).is_none());
  assert!(res.cosmetics.get(&PaintId::Kick(5)).is_some());
}

fn said(login: &str, body: &str) -> Message {
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.login = login.into();
  m.nick = login.into();
  m.body = body.into();
  m
}

#[test]
fn avatars_off_ask_for_nothing_and_hold_no_room() {
  let mut res = Res::new(1.0);
  let m = said("bob", "hi");
  let boxes =
    |res: &mut Res| layout_row(res, "c", &m, false, 400, Opts { ts: false, tags: false, find: None }).boxes.len();
  assert!(!res.look.avatars, "off like the ext");
  assert_eq!(boxes(&mut res), 0);
  assert!(res.avatars.take_wanted().is_empty() && res.avatars.len() == 0);
  res.look.avatars = true;
  // on: the square is held at once, one ask queued, however often the line is laid out
  assert_eq!(boxes(&mut res), 1);
  assert_eq!(boxes(&mut res), 1);
  assert_eq!(res.avatars.take_wanted(), vec![(Platform::Twitch, "bob".into())]);
  // the answer turns the stand-in into the picture, with the box unchanged
  let url = "https://static-cdn.jtvnw.net/jtv_user_pictures/a-70x70.png";
  assert!(res.avatars.put(Platform::Twitch, "bob", Some(url.into())));
  let laid = layout_row(&mut res, "c", &m, false, 400, Opts { ts: false, tags: false, find: None });
  assert_eq!(laid.boxes.len(), 1);
  assert_ne!(laid.boxes[0].img, ImgId::MAX);
  assert!(laid.boxes[0].stand_in.is_some());
  // a notice never gets one
  let sys = Message::system(1, Platform::Twitch, "x");
  let n = layout_row(&mut res, "c", &sys, false, 400, Opts { ts: false, tags: false, find: None }).boxes.len();
  assert_eq!(n, 0);
}

#[test]
fn a_missing_picture_draws_its_letter_in_a_box() {
  let mut res = Res::new(1.0);
  res.look.avatars = true;
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(300);
  v.push(said("bob", "hi"));
  let area = Rect::new(0, 0, 300, 40);
  let mut c = Canvas::new(300, 40);
  v.draw(&mut c, &mut res, area, area);
  let (_, ink) = crate::avatars::tile("bob", "bob");
  let n = (0..300 * 40).filter(|&i| c.px[i] & 0xffffff == ink).count();
  assert!(n >= 4 * 17, "the box outline and letter are in the avatar color, got {n}");
}

#[test]
fn filtered_lines_take_no_room_and_come_back() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(50, "c".into());
  v.filtered = true;
  v.resize(400);
  filter::edit(|f| f.bots = true);
  v.push(said("bob", "hi"));
  let one = v.content_height(&mut res);
  v.push(said("nightbot", "!commands"));
  v.push(said("nightbot", "!discord"));
  assert_eq!(v.content_height(&mut res), one);
  // can't select what is hidden
  v.select_move(-1, &mut res, 100);
  assert_eq!(&*v.selected().unwrap().login, "bob");
  v.select_move(-1, &mut res, 100);
  assert_eq!(&*v.selected().unwrap().login, "bob");
  // the switch goes off: they are all back
  filter::edit(|f| f.bots = false);
  v.refilter();
  assert_eq!(v.content_height(&mut res), one * 3);
  // a list tab (not filtered) ignores the rules
  filter::edit(|f| f.bots = true);
  let mut list = ChatView::new(50, "feed".into());
  list.resize(400);
  list.push(said("nightbot", "!x"));
  assert_eq!(list.content_height(&mut res), one);
}

#[test]
fn search_keeps_matching_lines_walks_them_and_clears() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(50, "c".into());
  v.resize(400);
  for (who, what) in [("bob", "hello world"), ("alice", "bye"), ("bobby", "HELLO again")] {
    v.push(said(who, what));
  }
  let h = v.height_at(0, &mut res);
  assert_eq!(v.content_height(&mut res), 3 * h);
  assert_eq!(v.set_search("hello", &mut res, 200), 2);
  assert_eq!(v.content_height(&mut res), 2 * h);
  // lines that arrive later are judged too
  v.push(said("carl", "hello!"));
  v.push(said("dan", "nope"));
  assert_eq!(v.found(), 3);
  assert_eq!(v.set_search("@bob", &mut res, 200), 2);
  assert_eq!(v.set_search("hello", &mut res, 200), 3);
  // n / N wrap round the three
  assert_eq!(v.search_step(1, &mut res, 200), Some((1, 3)));
  assert_eq!(&*v.selected().unwrap().login, "bob");
  assert_eq!(v.search_step(1, &mut res, 200), Some((2, 3)));
  assert_eq!(v.search_step(1, &mut res, 200), Some((3, 3)));
  assert_eq!(v.search_step(1, &mut res, 200), Some((1, 3)));
  assert_eq!(v.search_step(-1, &mut res, 200), Some((3, 3)));
  assert_eq!(v.set_search("zzz", &mut res, 200), 0);
  assert!(v.selected().is_none() && v.search_step(1, &mut res, 200).is_none());
  // clearing brings every line back
  assert_eq!(v.set_search("", &mut res, 200), 5);
  assert!(!v.searching());
  assert_eq!(v.content_height(&mut res), 5 * h);
}

#[test]
fn a_hit_is_drawn_in_reverse_video() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(50, "c".into());
  v.resize(400);
  v.push(said("bob", "hello world"));
  let (mut c, area) = (Canvas::new(400, 60), Rect::new(0, 0, 400, 60));
  let white = |c: &Canvas| c.px.iter().filter(|&&p| p == theme::WHITE).count();
  v.draw(&mut c, &mut res, area, area);
  let plain = white(&c);
  v.set_search("hello", &mut res, 60);
  v.draw(&mut c, &mut res, area, area);
  // a solid bar behind five letters is far more white than the glyphs ever were
  assert!(white(&c) > plain + 300, "{} vs {plain}", white(&c));
}

#[test]
fn a_muted_line_keeps_its_name_but_not_its_words() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(50, "c".into());
  v.filtered = true;
  v.resize(120);
  filter::edit(|f| _ = f.mute("quiet", None));
  // far longer than one row
  v.push(said("quiet", &"word ".repeat(80)));
  v.push(said("loud", &"word ".repeat(80)));
  let h = |v: &mut ChatView, res: &mut Res, i| v.height_at(i, res);
  assert!(h(&mut v, &mut res, 0) < h(&mut v, &mut res, 1));
}

#[test]
fn first_line_per_person_glows_once() {
  let mut v = ChatView::new(100, "c".into());
  let mk = |login: &str, first: bool| {
    let mut m = Message::system(1, Platform::Twitch, "x");
    m.kind = Kind::Chat;
    m.login = login.into();
    if first {
      m.flags |= flag::FIRST;
    }
    m
  };
  for m in [mk("a", false), mk("A", false), mk("b", true), mk("c", false)] {
    v.push(m);
  }
  v.push(Message::system(2, Platform::Twitch, "sys"));
  let new: Vec<bool> = v.rows.iter().map(|r| r.msg.has(flag::NEW)).collect();
  // second "a" is not new; a channel-first line keeps its own (magenta) mark
  assert_eq!(new, [true, false, false, true, false]);
}

#[test]
fn selection_walks_lines_and_skips_system() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(100, "c".into());
  v.resize(400);
  let mut f = Fake::new(1);
  for i in 0..5 {
    let mut m = f.message(i);
    m.kind = Kind::Chat;
    m.id = format!("m{i}").into();
    v.push(m);
  }
  v.push(Message::system(9, Platform::Twitch, "joined"));
  assert!(v.selected().is_none());
  v.select_move(-1, &mut res, 1000);
  assert_eq!(&*v.selected().unwrap().id, "m4");
  v.select_move(-2, &mut res, 1000);
  assert_eq!(&*v.selected().unwrap().id, "m2");
  // past the newest line it stays put rather than landing on the system line
  v.select_move(5, &mut res, 1000);
  assert_eq!(&*v.selected().unwrap().id, "m4");
  v.deselect();
  assert!(v.selected().is_none());
}

#[test]
fn emote_hit_testing_and_blocked_slot() {
  use crate::emotes::{Emote, Provider};
  let mut res = Res::new(1.0);
  let url = "https://cdn.7tv.app/emote/01ABC/1x.webp";
  let emote = Emote { url: url.into(), w: 28, h: 28, zero_width: false };
  res.emotes.set(None, Provider::SevenTv, [("Kappa".into(), emote)].into());
  let mut v = ChatView::new(10, "c".into());
  v.resize(400);
  let mut m = Fake::new(1).message(1);
  m.kind = Kind::Chat;
  m.emotes = Box::new([]);
  m.body = "hi Kappa yo".into();
  v.push(m);
  let area = Rect::new(0, 0, 400, 200);
  let scan = |v: &mut ChatView, res: &mut Res| {
    (0..400).step_by(2).find_map(|x| (150..200).find_map(|y| v.emote_at(area, x, y, res)))
  };
  let hit = scan(&mut v, &mut res).expect("emote under the pointer");
  assert_eq!((&*hit.name, &*hit.url), ("Kappa", url));
  // plain words are not emotes
  assert!(v.emote_at(area, 10, 199, &mut res).is_none());
  // blocked: still hit-testable (so a click can unblock) and still holds its slot
  res.blocked.block("Kappa", url, Scope::Channel);
  v.relayout();
  assert_eq!(&*scan(&mut v, &mut res).unwrap().name, "Kappa");
}

#[test]
fn a_hit_on_a_stacked_emote_yields_every_layer() {
  use crate::emotes::{Emote, Provider};
  let mut res = Res::new(1.0);
  let (base, ice, spark) = (
    "https://cdn.7tv.app/emote/a/1x.webp",
    "https://cdn.7tv.app/emote/b/1x.webp",
    "https://cdn.7tv.app/emote/c/1x.webp",
  );
  let e = |url: &str, zero_width| Emote { url: url.into(), w: 28, h: 28, zero_width };
  res.emotes.set(
    None,
    Provider::SevenTv,
    [("Kappa".into(), e(base, false)), ("frozen0".into(), e(ice, true)), ("spark".into(), e(spark, true))].into(),
  );
  let mut v = ChatView::new(10, "c".into());
  v.resize(400);
  let mut m = Fake::new(1).message(1);
  m.kind = Kind::Chat;
  m.emotes = Box::new([]);
  m.body = "hi Kappa frozen0 spark yo".into();
  v.push(m);
  let area = Rect::new(0, 0, 400, 200);
  let hit = (0..400)
    .step_by(2)
    .find_map(|x| (150..200).find_map(|y| v.emote_at(area, x, y, &mut res)))
    .expect("the stack under the pointer");
  assert_eq!((&*hit.name, &*hit.url), ("Kappa", base));
  let over: Vec<(&str, &str)> = hit.over.iter().map(|l| (&*l.name, &*l.url)).collect();
  assert_eq!(over, [("frozen0", ice), ("spark", spark)]);
  // a blocked overlay is not drawn, so it is not listed
  res.blocked.block("frozen0", ice, Scope::Channel);
  v.relayout();
  let hit = (0..400).step_by(2).find_map(|x| (150..200).find_map(|y| v.emote_at(area, x, y, &mut res))).unwrap();
  assert_eq!(hit.over.len(), 1);
}

#[test]
fn names_in_words_wear_their_color_and_answer_the_pointer() {
  let mut res = Res::new(1.0);
  let mut alice = said("alice", "x");
  alice.color = theme::GREEN;
  res.chatters.heard(&alice);
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(700);
  let mut m = said("bob", "hi alice, welcome @carol you what https://alice.example/x");
  m.color = theme::MAGENTA;
  v.push(m);
  let area = Rect::new(0, 0, 700, 200);
  let mut c = Canvas::new(700, 200);
  v.draw(&mut c, &mut res, area, area);
  let who = |v: &mut ChatView, res: &mut Res| {
    let mut seen: Vec<(Option<Box<str>>, u32)> = Vec::new();
    for x in (0..700).step_by(2) {
      if let Some(h) = (150..200).find_map(|y| v.who_at(area, x, y, res)) {
        let k = h.mention.map(|n| (Some(n.login), n.color)).unwrap_or((None, theme::MAGENTA));
        if !seen.contains(&k) {
          seen.push(k);
        }
      }
    }
    seen
  };
  // the sender, a spoken name (bare, with its comma left out), an unknown @name (white until looked up);
  // plain words, the everyday "you"/"what" and the link's words are none of them
  let got = who(&mut v, &mut res);
  assert_eq!(
    got,
    vec![(None, theme::MAGENTA), (Some("alice".into()), theme::GREEN), (Some("carol".into()), theme::WHITE)]
  );
  assert!(c.inked(theme::GREEN) > 20, "alice is drawn green");
  assert_eq!(res.chatters.take_wanted(), vec![(Platform::Twitch, "carol".into())]);
  // the answer arrives: carol's lines are laid out again in her color
  res.chatters.learned(Platform::Twitch, "carol", theme::CYAN, None);
  v.touch_person(Platform::Twitch, "carol");
  v.draw(&mut c, &mut res, area, area);
  assert!(who(&mut v, &mut res).contains(&(Some("carol".into()), theme::CYAN)));
  assert!(c.inked(theme::CYAN) > 20);
  // a deleted line stays dim: no colors, nothing to hover
  v.clear_msg("");
  let mut gone = said("bob", "hi alice");
  gone.id = "g".into();
  gone.flags |= flag::CLEARED;
  let mut w = ChatView::new(10, "c".into());
  w.resize(700);
  w.push(gone);
  assert!(
    (0..700).step_by(2).all(|x| (150..200).all(|y| w.who_at(area, x, y, &mut res).is_none_or(|h| h.mention.is_none())))
  );
}

#[test]
fn a_reply_shows_its_parents_words_on_one_line() {
  let mut res = Res::new(1.0);
  let mut m = said("bob", "lol");
  m.reply_to = Some("wollip".into());
  m.reply_text = Some("short one".into());
  let text = |l: &Laid| l.layout.lines().map(|ln| ln.text_range()).collect::<Vec<_>>();
  let l = layout_row(&mut res, "c", &m, false, 400, NO_OPTS);
  assert_eq!(text(&l).len(), 2);
  // long words (and a newline from a tag) are cut with …, never a third line
  m.reply_text = Some(format!("{}\nline two", "words ".repeat(60)).into());
  for w in [140, 400, 900] {
    let l = layout_row(&mut res, "c", &m, false, w, NO_OPTS);
    assert_eq!(text(&l).len(), 2, "at {w}px");
  }
  let l = layout_row(&mut res, "c", &m, false, 400, NO_OPTS);
  assert!(l.layout.lines().next().is_some_and(|ln| ln.text_range().len() > 20), "the line is filled, not emptied");
}

#[test]
fn a_reply_headers_name_is_a_mention() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(700);
  let mut m = said("bob", "lol");
  m.reply_to = Some("NL_Kripp".into());
  v.push(m);
  let area = Rect::new(0, 0, 700, 200);
  let mut c = Canvas::new(700, 200);
  v.draw(&mut c, &mut res, area, area);
  let ids = |v: &mut ChatView, res: &mut Res| {
    let mut seen: Vec<(Box<str>, u32)> = Vec::new();
    for x in (0..700).step_by(2) {
      for y in 0..200 {
        if let Some(n) = v.who_at(area, x, y, res).and_then(|h| h.mention) {
          let k = (n.login, n.color);
          if !seen.contains(&k) {
            seen.push(k);
          }
        }
      }
    }
    seen
  };
  // unknown: white, and a lookup is asked for
  assert_eq!(ids(&mut v, &mut res), vec![("nl_kripp".into(), theme::WHITE)]);
  assert_eq!(res.chatters.take_wanted(), vec![(Platform::Twitch, "nl_kripp".into())]);
  res.chatters.learned(Platform::Twitch, "nl_kripp", theme::CYAN, None);
  v.touch_person(Platform::Twitch, "nl_kripp");
  v.draw(&mut c, &mut res, area, area);
  assert_eq!(ids(&mut v, &mut res), vec![("nl_kripp".into(), theme::CYAN)]);
}

#[test]
fn a_reply_headers_line_answers_a_click_with_the_replys_id_and_the_words_below_it_do_not() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(700);
  let mut m = said("bob", "lol");
  m.id = "r1".into();
  m.reply_to = Some("wollip".into());
  m.reply_text = Some("short one".into());
  v.push(m);
  v.push(said("amy", "plain"));
  let area = Rect::new(0, 0, 700, 200);
  let mut c = Canvas::new(700, 200);
  v.draw(&mut c, &mut res, area, area);
  let hits: Vec<(i32, i32)> = (0..200)
    .step_by(2)
    .flat_map(|y| (0..700).step_by(4).map(move |x| (x, y)))
    .filter(|&(x, y)| v.reply_at(area, x, y, &mut res).is_some())
    .collect();
  assert!(!hits.is_empty(), "the header line is a target");
  assert!(v.reply_at(area, hits[0].0, hits[0].1, &mut res).as_deref() == Some("r1"));
  let (top, bottom) = (hits.iter().map(|h| h.1).min().unwrap(), hits.iter().map(|h| h.1).max().unwrap());
  assert!(bottom - top < 40, "one line tall, not the whole row: {top}..{bottom}");
}

#[test]
fn a_channel_owner_answers_the_pointer_by_bare_name_without_speaking() {
  let mut res = Res::new(1.0);
  res.chatters.host(Platform::Twitch, "nl_kripp");
  assert_eq!(res.chatters.take_wanted(), vec![(Platform::Twitch, "nl_kripp".into())]);
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(700);
  v.push(said("bob", "gg nl_kripp"));
  let area = Rect::new(0, 0, 700, 200);
  let mut c = Canvas::new(700, 200);
  v.draw(&mut c, &mut res, area, area);
  let hit = |v: &mut ChatView, res: &mut Res| {
    (0..700)
      .step_by(2)
      .find_map(|x| (0..200).find_map(|y| v.who_at(area, x, y, res)?.mention))
      .map(|n| (n.login, n.color))
  };
  assert_eq!(hit(&mut v, &mut res), Some(("nl_kripp".into(), theme::WHITE)));
  // their color lands from the lookup
  res.chatters.learned(Platform::Twitch, "nl_kripp", theme::CYAN, None);
  v.touch_person(Platform::Twitch, "nl_kripp");
  v.draw(&mut c, &mut res, area, area);
  assert_eq!(hit(&mut v, &mut res), Some(("nl_kripp".into(), theme::CYAN)));
}

#[test]
fn a_painted_mention_is_drawn_from_its_bitmap() {
  use crate::cosmetics::{Cosmetic, paint::Paint as P};
  let mut res = Res::new(1.0);
  let paint = P::parse(&serde_json::json!({"id": "G", "function": "linear-gradient", "angle": 90,
    "stops": [{"at": 0, "color": 0xff0000ffu32 as i32}, {"at": 1, "color": 0x0000ffffu32 as i32}]}))
  .unwrap();
  let mut alice = said("alice", "x");
  alice.user_id = "42".into();
  alice.color = theme::GREEN;
  res.chatters.heard(&alice);
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(400);
  v.push(said("bob", "wwwwwwwwwwww alice"));
  let area = Rect::new(0, 0, 400, 40);
  let mut c = Canvas::new(400, 40);
  v.draw(&mut c, &mut res, area, area);
  // their cosmetics are asked for, and alice is plain green meanwhile
  assert_eq!(res.cosmetics.take_wanted(), vec![PaintId::Twitch(42)]);
  let green = |c: &Canvas| c.inked(theme::GREEN);
  assert!(green(&c) > 20);
  res
    .cosmetics
    .put(&PaintId::Twitch(42), Some(Cosmetic { paint: Some(Arc::new(paint)), badge: None, ..Default::default() }));
  v.touch_paint(&PaintId::Twitch(42));
  v.draw(&mut c, &mut res, area, area);
  assert_eq!(green(&c), 0, "the name now runs red to blue");
  let (red, blue) = c.px.iter().fold((0, 0), |(r, b), p| {
    let (pr, pg, pb) = (p >> 16 & 255, p >> 8 & 255, p & 255);
    (r + (pr > 200 && pg < 60 && pb < 100) as u32, b + (pb > 200 && pr < 100 && pg < 60) as u32)
  });
  assert!(red > 5 && blue > 5, "{red} {blue}");
}

#[test]
fn links_hit_test_to_their_canonical_target() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(10, "c".into());
  v.resize(600);
  let mut m = Fake::new(1).message(1);
  m.kind = Kind::Chat;
  m.emotes = Box::new([]);
  m.body = "aaaa watch?v=Gz0fAz9n_Os bbbb".into();
  v.push(m);
  let area = Rect::new(0, 0, 600, 200);
  let scan = |v: &mut ChatView, res: &mut Res| {
    (0..600).step_by(2).filter_map(|x| (150..200).find_map(|y| v.link_at(area, x, y, res))).collect::<Vec<_>>()
  };
  let urls = scan(&mut v, &mut res);
  assert!(!urls.is_empty() && urls.iter().all(|u| u == "https://www.youtube.com/watch?v=Gz0fAz9n_Os"), "{urls:?}");
  // plain words are not links, and the switch turns the partial rule off
  assert!(v.link_at(area, 2, 199, &mut res).is_none());
  res.partial_links = false;
  v.relayout();
  assert!(scan(&mut v, &mut res).is_empty());
}

fn plain(id: &str, body: &str) -> Message {
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.id = id.into();
  m.nick = "bob".into();
  m.body = body.into();
  m
}

// renderInventoryEmotes: the sender's own emote and a server-resolved ref both draw, or neither does
#[test]
fn inventory_emotes_draw_only_while_their_switch_is_on() {
  let mut res = Res::new(1.0);
  let ts = 1_793_399_211_099;
  let mut m = plain("a", "kek hi");
  m.user_id = "42".into();
  m.ts = ts;
  let n = |res: &mut Res, m: &Message| layout_row(res, "c", m, false, 400, NO_OPTS).boxes.len();
  // the sender's own set, the privacy-safe source
  res.senders.lookup(42, "kek", ts);
  let row = serde_json::json!({"custom_name": "kek", "url": "https://cdn.7tv.app/emote/x/1x.webp", "width": 32,
    "height": 32, "added_at": "2026-10-30T22:26:51.099Z"});
  res.senders.put(42, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
  assert_eq!(n(&mut res, &m), 1);
  res.look.inventory_emotes = false;
  assert_eq!(n(&mut res, &m), 0, "off: their own emote is the plain word");
  // a ref the server resolved for the line is an inventory emote too
  let mut r = plain("b", "pog hi");
  r.hs = Box::new([("pog".into(), "https://cdn.7tv.app/emote/y/1x.webp".into(), false)]);
  assert_eq!(n(&mut res, &r), 0);
  res.look.inventory_emotes = true;
  assert_eq!(n(&mut res, &r), 1);
}

// privacy invariant, archived lines: the archive's own render may name a heatsync emote, but it draws
// only while the SENDER's set has it (in its window, with the switch on) — the one gate. a provider
// emote in the same render draws as before
#[test]
fn archived_heatsync_emotes_draw_only_from_the_senders_own_set() {
  use crate::model::logs::{Who, parse_row};
  let mut res = Res::new(1.0);
  let who = Who { platform: Platform::Twitch, login: "bob".into(), nick: "bob".into(), user_id: "42".into() };
  let row = serde_json::json!({"message": "kek LUL", "timestamp": "2026-10-30T22:30:00.000Z",
    "message_html": "<img src=\"/e/hs/abc\" alt=\"kek\"> <img src=\"/e/7tv/9\" alt=\"LUL\">"});
  let m = parse_row(&row, &who).unwrap().msg;
  let n = |res: &mut Res, m: &Message| layout_row(res, "c", m, false, 400, NO_OPTS).boxes.len();
  assert_eq!(n(&mut res, &m), 1, "the render's word is not enough: kek stays text, LUL is the channel's");
  res.senders.lookup(42, "kek", m.ts);
  let owns = |from: &str| {
    let row = serde_json::json!({"custom_name": "kek", "url": "https://cdn.7tv.app/emote/x/1x.webp", "width": 32,
      "height": 32, "added_at": from});
    crate::heatsync::senders::parse_set(&serde_json::json!([row]))
  };
  // they added it after this line: not theirs yet
  res.senders.put(42, owns("2026-11-01T00:00:00.000Z"));
  assert_eq!(n(&mut res, &m), 1);
  res.senders.put(42, owns("2026-10-30T22:26:51.099Z"));
  assert_eq!(n(&mut res, &m), 2, "theirs when they said it");
  res.look.inventory_emotes = false;
  assert_eq!(n(&mut res, &m), 1, "the switch off: only the channel's");
  res.look.inventory_emotes = true;
  // someone else's set never reaches this sender's line
  let mut other = m.clone();
  other.user_id = "43".into();
  assert_eq!(n(&mut res, &other), 1);
  // and a heatsync image placed on a line by position (any producer) goes the same road
  let mut placed = plain("p", "kek hi");
  placed.user_id = "44".into();
  placed.ts = m.ts;
  placed.emotes = Box::new([(0..3, "https://heatsync.org/e/hs/abc".into())]);
  assert_eq!(n(&mut res, &placed), 0, "not 44's");
  placed.emotes = Box::new([(0..3, "https://heatsync.org/e/7tv/abc".into())]);
  assert_eq!(n(&mut res, &placed), 1, "a provider mirror placed by position is native");
}

// an overlay emote right after a lone emoji stacks on it (ext): the emoji becomes the slot's base,
// drawn as itself; anything between them, or no overlay, leaves the emoji as text
#[test]
fn an_overlay_after_an_emoji_stacks_on_it() {
  use crate::emotes::{Emote, Provider};
  let mut res = Res::new(1.0);
  let fog = Emote { url: "https://cdn.7tv.app/emote/fog/1x.webp".into(), w: 28, h: 28, zero_width: true };
  let base = Emote { url: "https://cdn.7tv.app/emote/b/1x.webp".into(), w: 28, h: 28, zero_width: false };
  res.emotes.set(Some("c"), Provider::SevenTv, [("fog0".into(), fog), ("Base".into(), base)].into());
  let laid = |res: &mut Res, body: &str| {
    let l = layout_row(res, "c", &plain("a", body), false, 400, NO_OPTS);
    l.boxes.iter().map(|b| (b.glyph.as_deref().map(str::to_owned), b.zw.len())).collect::<Vec<_>>()
  };
  let on = |g: &str| vec![(Some(g.to_owned()), 0)];
  assert_eq!(laid(&mut res, "🏙  fog0"), on("🏙"), "two spaces between, like the report");
  assert_eq!(laid(&mut res, "hi 🏙\u{fe0f} fog0 there"), on("🏙\u{fe0f}"));
  assert_eq!(laid(&mut res, "👍🏽 fog0"), on("👍🏽"));
  assert_eq!(laid(&mut res, "🏙 fog0 fog0"), vec![(Some("🏙".to_owned()), 1)], "a second overlay stacks too");
  assert!(laid(&mut res, "🏙 hi fog0").iter().all(|b| b.0.is_none()), "a word between: not stacked");
  assert!(laid(&mut res, "hi🏙 fog0").iter().all(|b| b.0.is_none()), "not a lone emoji");
  assert!(laid(&mut res, "🏙 hi").is_empty(), "no overlay: the emoji stays text");
  assert_eq!(laid(&mut res, "Base fog0"), vec![(None, 1)], "an emote base as before");
}

// linksEnabled: off, a url is plain text, not a link and not a partial one either
#[test]
fn links_off_leaves_urls_plain() {
  let mut res = Res::new(1.0);
  let m = plain("a", "see https://example.com/x and watch?v=Gz0fAz9n_Os");
  assert_eq!(layout_row(&mut res, "c", &m, false, 600, NO_OPTS).links.len(), 2);
  res.links = false;
  assert!(layout_row(&mut res, "c", &m, false, 600, NO_OPTS).links.is_empty());
}

// the chat look: the clock before a line, its format, platform tags, line spacing, emote size
#[test]
fn the_look_changes_what_a_row_is_made_of() {
  let mut res = Res::new(1.0);
  let mut m = plain("a", "hi");
  m.ts = 1_700_000_000_000;
  let at = |res: &mut Res, o: Opts| layout_row(res, "c", &m, false, 600, o).nick.start;
  let on = Opts { ts: true, tags: true, find: None };
  // off (the ext's default): no time column at all
  assert_eq!(at(&mut res, on), 3, "just the tag: the clock is off");
  res.look.tags = false;
  assert_eq!(at(&mut res, on), 0, "no tags, no clock: the name leads");
  res.look.timestamps = true;
  assert_eq!(at(&mut res, on), "00:00 ".len(), "24h");
  assert_eq!(at(&mut res, Opts { ts: false, ..on }), 0, "a view that never shows time still doesn't");
  res.look.twelve = true;
  let n = at(&mut res, on);
  assert!(n == "4:05pm ".len() || n == "12:05pm ".len(), "12h with am/pm, no leading zero: {n}");
  // line spacing: 22 and 26 are +10% and +30% of the 20px line
  let h = |res: &mut Res| layout_row(res, "c", &m, false, 600, NO_OPTS).layout.height().round() as i32;
  let tight = h(&mut res);
  res.look.spacing = 1.3;
  assert_eq!((tight, h(&mut res)), (20, 26));
  // emote size multiplies the base box; badges and the rest keep theirs
  assert_eq!([1u16, 2, 4].map(|k| Look { emote_mul: k, ..res.look }.emote_px()), [28, 56, 112]);
}

// readable names off keeps the platform's raw color; dim timeouts off leaves a timed-out line as it was
#[test]
fn the_look_picks_name_colors_and_dimming() {
  let look = Look::default();
  assert_ne!(look.ink(0x0000ff), 0x0000ff, "readable lifts dark blue");
  assert_eq!(Look { readable: false, ..look }.ink(0x0000ff), 0x0000ff, "off: as sent");
  let mut res = Res::new(1.0);
  res.chatters.learned(Platform::Twitch, "carol", theme::CYAN, None);
  let mut m = said("bob", "hi @carol");
  m.flags |= flag::CLEARED;
  let seen = |res: &mut Res| layout_row(res, "c", &m, false, 600, NO_OPTS).mentions.len();
  assert_eq!(seen(&mut res), 0, "a timed-out line is dim: no colored names");
  res.look.dim_timeouts = false;
  assert_eq!(seen(&mut res), 1, "off: the line reads as normal");
}

// animate emotes never: an animated picture holds its first frame and asks for no ticks
#[test]
fn animate_never_holds_the_first_frame() {
  use crate::images::decode::{Decoded, Frame};
  let mut res = Res::new(1.0);
  let id = res.images.id("https://x/a.gif", 28);
  assert!(res.images.frame(id).is_none());
  let (_, _, want) = res.images.take_wanted().remove(0);
  let frame = |v: u8| Frame { rgba: vec![v; 4].into(), delay_ms: 50 };
  res.images.loaded(id, want, Ok(Decoded { w: 1, h: 1, frames: vec![frame(1), frame(2)] }));
  assert!(res.images.frame_with(id, true).is_some_and(|f| f.3.is_some()), "plays: a flip is due");
  let still = res.images.frame_with(id, false).map(|f| (f.0.rgba[0], f.3));
  assert_eq!(still, Some((1, None)), "never: frame one, no ticks");
}

#[test]
fn alert_lines_get_the_mention_fill_and_copies_keep_their_room() {
  use crate::model::message::{Extra, Seg};
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.glow = false;
  v.resize(400);
  let mut m = plain("a", "hello");
  v.push(m.clone());
  m.flags |= flag::ALERT;
  m.id = "b".into();
  v.push(m);
  let area = Rect::new(0, 0, 400, 80);
  let mut c = Canvas::new(400, 80);
  v.draw(&mut c, &mut res, area, area);
  let edge = |y: usize| c.px[y * 400];
  // newest line sits at the bottom, on the dark red fill; the one above it stays black
  assert_eq!(edge(70), theme::MENTION_BG);
  assert_eq!(edge(50), theme::BLACK);
  // on a zebra row a mention takes the lighter red, never the gray stripe
  v.zebra = true;
  v.draw(&mut c, &mut res, area, area);
  let (ping, plain_row) = (c.px[70 * 400 + 200], c.px[50 * 400 + 200]);
  assert!([theme::MENTION_BG, theme::MENTION_STRIPE].contains(&ping), "{ping:06x}");
  assert!([theme::BLACK, theme::ZEBRA].contains(&plain_row), "{plain_row:06x}");
  assert_ne!(ping == theme::MENTION_STRIPE, plain_row == theme::ZEBRA, "stripes alternate");
  // a copied line draws a room tag and head/tail segs around the name
  let mut x = plain("c", "hi");
  x.x = Some(Box::new(Extra {
    chan: "forsen".into(),
    head: Box::new([Seg { text: ">>1 ".into(), ink: theme::YELLOW, bold: true }]),
    tail: Box::new([Seg { text: " 5".into(), ink: theme::WHITE, bold: false }]),
    edge: 0,
    paint: None,
    pic: None,
    late: false,
    held: None,
    ..Extra::default()
  }));
  let l = layout_row(&mut res, "tab", &x, false, 400, NO_OPTS);
  let n = l.layout.lines().count();
  assert_eq!(n, 1);
  assert!(l.layout.width() > layout_row(&mut res, "tab", &plain("c", "hi"), false, 400, NO_OPTS).layout.width());
}

#[test]
fn a_top_list_reads_from_the_first_row_and_scrolls_down() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(50, "feed".into());
  v.top = true;
  v.zebra = false;
  v.resize(400);
  for i in 0..30 {
    v.push(plain(&format!("p{i}"), &format!("post {i}")));
  }
  let area = Rect::new(0, 0, 400, 100);
  let mut c = Canvas::new(400, 100);
  let ink = |c: &Canvas, y0: usize, y1: usize| (y0 * 400..y1 * 400).any(|i| c.px[i] != theme::BLACK);
  assert!(matches!(v.take_paint(&mut res, 100), Paint::Full));
  v.draw(&mut c, &mut res, area, area);
  // first post at the very top, and a short list hangs from the top too
  assert!(ink(&c, 0, 20));
  // scrolling "down the list" (wheel down = negative px) moves content up
  v.scroll_by(-40, &mut res);
  assert!(matches!(v.take_paint(&mut res, 100), Paint::Full));
  v.draw(&mut c, &mut res, area, area);
  assert!(ink(&c, 0, 20));
  // end of list clamps
  v.scroll_to_bottom();
  v.draw(&mut c, &mut res, area, area);
  assert!(ink(&c, 80, 100));
  v.scroll_to_top(&mut res);
  // selection starts at the top post and reveals what it moves to
  v.select_start(&mut res, 100);
  assert_eq!(&*v.selected().unwrap().id, "p0");
  v.select_move(25, &mut res, 100);
  assert_eq!(&*v.selected().unwrap().id, "p25");
  v.draw(&mut c, &mut res, area, area);
  assert!(v.select_id("p3", &mut res, 100));
  assert_eq!(&*v.selected().unwrap().id, "p3");
  assert!(!v.select_id("nope", &mut res, 100));
  // a click lands on the row drawn there
  v.scroll_to_top(&mut res);
  v.draw(&mut c, &mut res, area, area);
  assert!(v.select_at(area, 5, &mut res));
  assert_eq!(&*v.selected().unwrap().id, "p0");
}
#[test]
fn a_view_made_after_startup_lays_a_system_line_on_one_row() {
  let mut res = Res::new(1.0);
  let sys = || Message::system(1, Platform::Kick, "not sent: log in to send (ctrl-l)");
  let mut v = ChatView::new(10, "dellor".into());
  v.push(sys());
  v.resize(600);
  let one = v.height_at(0, &mut res);
  let line = layout_row(&mut res, "dellor", &plain("c", "x"), false, 600, NO_OPTS).layout.height().ceil() as i32;
  assert_eq!(one, line, "one row at a normal width");
  // a transient 0 clamps: a few rows at worst, never a word per line (9 words)
  let mut w = ChatView::new(10, "dellor".into());
  w.push(sys());
  w.resize(0);
  assert!(w.height_at(0, &mut res) <= 4 * line);
  w.resize(600);
  assert_eq!(w.height_at(0, &mut res), one);
}
#[test]
fn replace_and_clear() {
  let mut v = ChatView::new(10, "c".into());
  v.push(plain("a", "one"));
  v.push(plain("b", "two"));
  assert!(v.replace("b", plain("b", "TWO")));
  assert!(!v.replace("zz", plain("zz", "x")));
  v.clear();
  assert_eq!(v.len(), 0);
  v.push(plain("c", "three"));
  assert_eq!(v.len(), 1);
  assert!(v.selected().is_none());
}

#[test]
fn reload_keeps_scroll_and_selection() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(10, "c".into());
  v.top = true;
  v.resize(400);
  for id in ["a", "b", "c"] {
    v.push(plain(id, id));
  }
  assert!(v.select_id("b", &mut res, 200));
  v.reload(vec![plain("z", "new"), plain("a", "a"), plain("b", "b")]);
  assert_eq!(v.len(), 3);
  assert_eq!(&*v.selected().unwrap().id, "b");
  v.reload(vec![plain("q", "only")]);
  assert!(v.selected().is_none());
}

fn at(id: &str, ts: i64) -> Message {
  let mut m = plain(id, id);
  m.ts = ts;
  m
}

fn ids(v: &ChatView) -> Vec<String> {
  v.rows.iter().map(|r| r.msg.id.to_string()).collect()
}

#[test]
fn a_gap_is_filled_in_time_order_without_doubling_a_line() {
  let mut v = ChatView::new(50, "c".into());
  for (id, ts) in [("a", 10), ("b", 20), ("e", 50), ("f", 60)] {
    v.push(at(id, ts));
  }
  // the socket was down for b..e: c and d were said meanwhile; b and e came through
  let n = v.fill_gap(vec![at("b", 20), at("c", 30), at("d", 40), at("e", 50), at("old", 1), at("", 35)]);
  assert_eq!(n, 2, "only c and d are new: b/e are had, `old` is older than the view, no id can't be placed");
  assert_eq!(ids(&v), ["a", "b", "c", "d", "e", "f"]);
  assert!(v.rows[2].msg.has(flag::HISTORY));
  // asking again changes nothing
  assert_eq!(v.fill_gap(vec![at("c", 30), at("d", 40)]), 0);
  assert_eq!(v.len(), 6);
  // later pushes carry on from the renumbered run
  v.push(at("g", 70));
  assert_eq!(ids(&v).last().map(String::as_str), Some("g"));
  let seqs: Vec<i64> = v.rows.iter().map(|r| r.seq).collect();
  assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{seqs:?}");
}

#[test]
fn a_gap_fill_keeps_the_picked_line_and_the_cap() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(5, "c".into());
  v.resize(400);
  for (id, ts) in [("a", 10), ("d", 40), ("e", 50)] {
    v.push(at(id, ts));
  }
  assert!(v.select_id("e", &mut res, 200));
  v.fill_gap(vec![at("b", 20), at("c", 30), at("x", 45)]);
  assert_eq!(ids(&v), ["b", "c", "d", "x", "e"], "six lines in a view of five: the oldest goes");
  assert_eq!(&*v.selected().unwrap().id, "e", "the pick moves with its line");
}

#[test]
fn history_then_live_lines_can_be_picked() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(50, "c".into());
  v.resize(400);
  v.prepend(vec![at("h1", 1), at("h2", 2)]);
  v.push(at("l1", 10));
  v.push(at("l2", 11));
  assert!(v.select_id("l2", &mut res, 200));
  assert_eq!(&*v.selected().unwrap().id, "l2");
}

// event rows draw in their kind's color with that color down the left; a cheer's amount sits in a twitch
// box. HSC_DUMP=x.ppm writes the frame
#[test]
fn event_rows_wear_their_color() {
  use crate::model::message::Notice;
  let mut res = Res::new(1.0);
  let mut sub = Message::system(1, Platform::Twitch, "bob subscribed at tier 1");
  sub.extra().notice = Notice::Sub;
  let mut resub = said("amy", "hype");
  resub.system = Some("amy subscribed for 5 months".into());
  resub.extra().notice = Notice::Milestone;
  let mut cheer = said("cat", "Cheer100 gg");
  cheer.extra().bits = 100;
  let mut announce = said("eve", "listen up");
  announce.extra().notice = Notice::Announce;
  let mut redeem = said("dan", "play the song");
  redeem.system = Some("◆ channel point redeem".into());
  redeem.extra().notice = Notice::Redeem;
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(400);
  for m in [sub, resub, cheer, redeem, announce] {
    v.push(m);
  }
  let area = Rect::new(0, 0, 400, 200);
  let mut c = Canvas::new(400, 200);
  v.draw(&mut c, &mut res, area, area);
  if let Some(path) = std::env::var_os("HSC_DUMP") {
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(path, out);
  }
  let at = |x: i32, y: i32| c.px[(y * c.w + x) as usize];
  let rows: Vec<i32> = v.rows.iter().map(|r| r.height).collect();
  let y0 = area.bottom() - rows.iter().sum::<i32>();
  // each row's left edge: magenta sub, cyan milestone, none for the cheer
  assert_eq!(at(0, y0 + 2), theme::MAGENTA);
  assert_eq!(at(0, y0 + rows[0] + 2), theme::CYAN);
  assert_ne!(at(0, y0 + rows[0] + rows[1] + 2), theme::MAGENTA);
  // the redeem: twitch-colored edge
  assert_eq!(at(0, y0 + rows[0] + rows[1] + rows[2] + 2), theme::TWITCH);
  let count = |ink| c.px.iter().filter(|&&p| p == ink).count();
  assert!(count(theme::TWITCH) > 60, "the bits box and its words");
  // the sub's words are magenta, not the dim of a plain notice
  assert!(count(theme::MAGENTA) > rows[0] as usize * MARK_W as usize + 20);
  // the announcement: its chip's box is a yellow line across the chip, past the edge
  let y4 = y0 + rows[..4].iter().sum::<i32>();
  let boxed = (y4..y4 + rows[4]).any(|y| (MARK_W + 1..c.w).filter(|&x| at(x, y) == theme::YELLOW).count() >= 40);
  assert!(boxed, "the announce chip is boxed");
}

// a line sent through heatsync.org wears [H] in heatsync orange even with platform tags off
#[test]
fn a_heatsync_send_wears_its_tag() {
  let mut res = Res::new(1.0);
  res.look.tags = false;
  let mut m = said("mellen", "gg");
  let plain = layout_row(&mut res, "c", &m, false, 400, NO_OPTS);
  assert_eq!(plain.nick.start, 0, "no tag");
  m.extra().origin = true;
  let l = layout_row(&mut res, "c", &m, false, 400, NO_OPTS);
  assert_eq!(l.nick.start, "[H]".len(), "the tag before the name");
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(400);
  v.push(m);
  let area = Rect::new(0, 0, 400, 40);
  let mut c = Canvas::new(400, 40);
  v.draw(&mut c, &mut res, area, area);
  if let Some(path) = std::env::var_os("HSC_DUMP") {
    let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
    c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
    let _ = std::fs::write(path, out);
  }
  assert!(c.px.iter().filter(|&&p| p == theme::HEATSYNC).count() > 20);
}

fn relayed(p: Platform, badges: &[(&str, &str)]) -> Message {
  let mut m = said("bob", "hi");
  m.platform = p;
  m.color = theme::RED;
  m.badges = badges.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect();
  m
}

#[test]
fn kick_lines_wear_sub_art_by_nearest_month_and_words_for_roles() {
  let mut res = Res::new(1.0);
  res.badges.set_kick("c", vec![(1, "https://files.kick.com/a/1".into()), (6, "https://files.kick.com/a/6".into())]);
  let m = relayed(Platform::Kick, &[("moderator", "1"), ("subscriber", "7"), ("og", "1"), ("trainwreckstv", "1")]);
  let at = |res: &mut Res, m: &Message, chan: &str| {
    let l = layout_row(res, chan, m, false, 400, NO_OPTS);
    (l.boxes.len(), l.nick.start)
  };
  // the sub is a picture, the roles words, a channel's own badge name draws nothing
  assert_eq!(at(&mut res, &m, "c"), (1, " mod  og ".len()));
  // the sub art is the 6 month tier (7 months, nearest lower): nothing asked until drawn, so look at the box
  let l = layout_row(&mut res, "c", &m, false, 400, NO_OPTS);
  assert_eq!(l.boxes[0].img, res.images.id("https://files.kick.com/a/6", crate::ui::res::BADGE_PX));
  // another tab has no art: the sub falls back to its word, like the ext
  assert_eq!(at(&mut res, &m, "other"), (0, " mod  sub  og ".len()));
  // text badges: words only, no picture
  res.look.text_badges = true;
  assert_eq!(at(&mut res, &m, "c"), (0, " mod  sub  og ".len()));
  // a sub below the lowest tier has no art either
  res.look.text_badges = false;
  res.badges.set_kick("c", vec![(3, "https://files.kick.com/a/3".into())]);
  assert_eq!(at(&mut res, &relayed(Platform::Kick, &[("subscriber", "1")]), "c"), (0, " sub ".len()));
  // a twitch-id badge set never dresses a kick chatter
  res.badges.set_users(
    crate::badges::Source::Ffz,
    crate::badges::UserBadges {
      list: vec![crate::badges::Badge { url: "https://cdn.frankerfacez.com/x".into(), title: "".into(), bg: None }],
      users: [(42, vec![0])].into(),
    },
  );
  let mut k = relayed(Platform::Kick, &[]);
  k.user_id = "42".into();
  assert_eq!(at(&mut res, &k, "c"), (0, 0));
}

fn drawn(res: &mut Res, m: Message, colors: [u32; 2]) -> (usize, usize) {
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(400);
  v.push(m);
  let area = Rect::new(0, 0, 400, 40);
  let mut c = Canvas::new(400, 40);
  v.draw(&mut c, res, area, area);
  (c.inked(colors[0]), c.inked(colors[1]))
}

// pixels of the periwinkle (theme::BLUE) ink at any shade: blue well over red, red = green. the
// exact-channel `inked` can't say it (its channels are neither 0 nor 255)
fn periwinkle(m: Message, res: &mut Res) -> usize {
  let mut v = ChatView::new(10, "c".into());
  v.zebra = false;
  v.resize(400);
  v.push(m);
  let area = Rect::new(0, 0, 400, 40);
  let mut c = Canvas::new(400, 40);
  v.draw(&mut c, res, area, area);
  let near = |p: &u32| {
    let (r, g, b) = (p >> 16 & 255, p >> 8 & 255, p & 255);
    r.abs_diff(g) < 6 && b > r + r / 2 + 24
  };
  c.px.iter().filter(|p| near(p)).count()
}

#[test]
fn kick_role_chips_are_drawn_in_their_colors() {
  let mut res = Res::new(1.0);
  let plain = drawn(&mut res, relayed(Platform::Kick, &[]), [theme::MAGENTA, theme::GREEN]);
  let dressed = drawn(&mut res, relayed(Platform::Kick, &[("vip", "1"), ("og", "1")]), [theme::MAGENTA, theme::GREEN]);
  assert!(dressed.0 > plain.0 + 10, "vip is magenta");
  assert!(dressed.1 > plain.1 + 10, "og is green");
}

#[test]
fn youtube_lines_wear_a_star_and_swords_and_nothing_for_members() {
  let mut res = Res::new(1.0);
  let m = relayed(Platform::Youtube, &[("broadcaster", "1"), ("moderator", "1"), ("member", "1"), ("verified", "1")]);
  let l = layout_row(&mut res, "c", &m, false, 400, NO_OPTS);
  assert_eq!((l.boxes.len(), l.nick.start), (0, "\u{2606}\u{2694}".len()));
  let yellow = |res: &mut Res, m| drawn(res, m, [theme::YELLOW, theme::YELLOW]).0;
  let plain = relayed(Platform::Youtube, &[]);
  assert!(yellow(&mut res, m.clone()) > yellow(&mut res, plain.clone()), "the owner's star is yellow");
  assert!(periwinkle(m, &mut res) > periwinkle(plain, &mut res), "the moderator's swords are blue");
}

#[test]
fn lines_below_count_while_scrolled_up_and_clear_at_the_bottom() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(200, "c".into());
  v.resize(400);
  for i in 0..40 {
    v.push(plain(&format!("p{i}"), "hello"));
  }
  assert!(matches!(v.take_paint(&mut res, 100), Paint::Full));
  assert_eq!(v.jump(), None, "pinned: no bar");
  v.scroll_by(60, &mut res);
  assert_eq!(v.jump(), Some(0));
  for i in 0..3 {
    v.push(plain(&format!("q{i}"), "more"));
  }
  v.take_paint(&mut res, 100);
  assert_eq!(v.jump(), Some(3));
  v.scroll_to_bottom();
  assert_eq!(v.jump(), None);
  v.take_paint(&mut res, 100);
  v.scroll_by(60, &mut res);
  assert_eq!(v.jump(), Some(0), "back at the bottom forgot the count");
  v.clear();
  assert_eq!(v.jump(), None);
}

#[test]
fn a_list_read_from_the_top_has_no_jump_bar() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(50, "feed".into());
  v.top = true;
  v.resize(400);
  for i in 0..30 {
    v.push(plain(&format!("p{i}"), "post"));
  }
  v.take_paint(&mut res, 100);
  v.scroll_by(-40, &mut res);
  v.push(plain("z", "late"));
  v.take_paint(&mut res, 100);
  assert_eq!(v.jump(), None);
}

// ---- heatsync's marks before a twitch name: sub flair, flame, heat bar

fn marked(uid: &str) -> Message {
  let mut m = said("bob", "hi");
  m.user_id = uid.into();
  m
}

// how much text sits before the name, and how many pictures (badges) the line holds
fn before_name(res: &mut Res, m: &Message) -> (usize, usize) {
  let l = layout_row(res, "c", m, false, 400, NO_OPTS);
  (l.nick.start, l.boxes.len())
}

// a heat score landed for bob, and his set says he uses heatsync
fn heated(res: &mut Res, heat: u32, known: bool) {
  res.heats.get("bob");
  res.heats.put("bob", heat);
  if known {
    let row = serde_json::json!({"custom_name": "kek", "url": "https://cdn.7tv.app/emote/x/1x.webp", "width": 32,
      "height": 32, "added_at": "2026-10-30T22:26:51.099Z"});
    res.senders.lookup(42, "kek", 1);
    res.senders.put(42, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
  }
}

const BAR_W: usize = "\u{258c}".len();
const FLAME_W: usize = "\u{1f525}".len();

#[test]
fn heat_bars_follow_the_ladder_and_draw_in_its_colors() {
  let mut res = Res::new(1.0);
  let m = marked("42");
  let bar = |res: &mut Res, heat| {
    res.heats.put("bob", heat);
    before_name(res, &m).0
  };
  heated(&mut res, 0, false);
  assert_eq!(before_name(&mut res, &m), (0, 0), "no score: nothing");
  assert_eq!(bar(&mut res, 4), 0, "under 5 is hidden");
  for heat in [5, 20, 50, 100, 200, 500, 1000, 5000] {
    assert_eq!(bar(&mut res, heat), 3, "one 3-byte bar glyph at {heat}");
  }
  // pixels: white under 50, yellow from 50, red from 1000 (the name is red/green/etc., so count the other two)
  let ink = |res: &mut Res, heat, c| {
    res.heats.put("bob", heat);
    let mut m = m.clone();
    m.color = theme::BLUE;
    drawn(res, m, [c, c]).0
  };
  let base = |res: &mut Res, c| ink(res, 0, c);
  let (w0, y0, r0) = (base(&mut res, theme::WHITE), base(&mut res, theme::YELLOW), base(&mut res, theme::RED));
  assert!(ink(&mut res, 10, theme::WHITE) > w0, "cold is white");
  assert_eq!(ink(&mut res, 10, theme::YELLOW), y0, "and not yellow");
  assert!(ink(&mut res, 100, theme::YELLOW) > y0, "warm is yellow");
  assert!(ink(&mut res, 5000, theme::RED) > r0, "blowing up is red");
  assert_eq!(ink(&mut res, 5000, theme::YELLOW), y0, "and not yellow");
}

#[test]
fn the_flame_is_for_known_users_without_a_bar_and_never_beside_one() {
  let mut res = Res::new(1.0);
  let m = marked("42");
  heated(&mut res, 0, true);
  assert_eq!(before_name(&mut res, &m).0, FLAME_W, "known and cold: the flame");
  res.heats.put("bob", 3);
  assert_eq!(before_name(&mut res, &m).0, FLAME_W, "under 5 has no bar, the flame stays");
  res.heats.put("bob", 120);
  assert_eq!(before_name(&mut res, &m).0, BAR_W, "a bar replaces the flame, they are never together");
  res.look.heat_bars = false;
  assert_eq!(before_name(&mut res, &m).0, FLAME_W, "bars off: the flame is back");
  // it really draws: more pixels lit than with the flame off
  let lit = |res: &mut Res| {
    let mut v = ChatView::new(10, "c".into());
    v.zebra = false;
    v.resize(400);
    v.push(m.clone());
    let area = Rect::new(0, 0, 400, 40);
    let mut c = Canvas::new(400, 40);
    v.draw(&mut c, res, area, area);
    c.px.iter().filter(|&&p| p != 0).count()
  };
  let with = lit(&mut res);
  res.look.flame = false;
  assert!(with > lit(&mut res) + 20, "the flame has pixels");
  res.look.flame = true;
  res.look.flame = false;
  assert_eq!(before_name(&mut res, &m).0, 0, "both off");
  res.look.flame = true;
  res.senders.set_me(Some(42));
  assert_eq!(before_name(&mut res, &m).0, 0, "never on your own line");
  res.senders.set_me(None);
  // a stranger (no set) wears none
  let mut other = marked("43");
  other.login = "amy".into();
  other.nick = "amy".into();
  assert_eq!(before_name(&mut res, &other).0, 0);
}

#[test]
fn the_marks_come_flair_then_flame_then_bar_and_only_on_twitch() {
  use crate::badges::Badge;
  let mut res = Res::new(1.0);
  let m = marked("42");
  heated(&mut res, 0, true);
  res.flairs.get(42);
  let sub = Badge { url: "https://static-cdn.jtvnw.net/badges/v1/a/1".into(), title: "x sub".into(), bg: None };
  res.flairs.put(42, Some(sub));
  let l = layout_row(&mut res, "c", &m, false, 400, NO_OPTS);
  // the flair is a picture at the start of the text; the flame follows it
  assert_eq!((l.boxes.len(), l.nick.start), (1, FLAME_W));
  let at = |l: &Laid| l.boxes[0].img;
  assert_eq!(at(&l), res.images.id("https://static-cdn.jtvnw.net/badges/v1/a/1", crate::ui::res::BADGE_PX));
  res.heats.put("bob", 300);
  assert_eq!(before_name(&mut res, &m), (BAR_W, 1), "flair, then the bar in place of the flame");
  res.look.sub_flair = false;
  assert_eq!(before_name(&mut res, &m), (BAR_W, 0), "flair off");
  // kick and youtube lines never wear them, and never ask
  res.heats.take_wanted();
  res.flairs.take_wanted();
  res.look.sub_flair = true;
  for p in [Platform::Kick, Platform::Youtube] {
    let mut k = marked("42");
    k.platform = p;
    k.login = "kickbob".into();
    assert_eq!(before_name(&mut res, &k), (0, 0));
  }
  assert!(res.heats.take_wanted().is_empty() && res.flairs.take_wanted().is_empty());
  // a system line has no marks either
  let mut s = Message::system(1, Platform::Twitch, "raid");
  s.user_id = "42".into();
  s.login = "bob".into();
  assert_eq!(before_name(&mut res, &s), (0, 0));
}

#[test]
fn seeing_a_twitch_chatter_asks_once_for_their_heat_and_flair() {
  let mut res = Res::new(1.0);
  let m = marked("42");
  before_name(&mut res, &m);
  assert_eq!(res.heats.take_wanted(), ["bob".into()]);
  assert_eq!(res.flairs.take_wanted(), [42]);
  before_name(&mut res, &m);
  assert!(res.heats.take_wanted().is_empty() && res.flairs.take_wanted().is_empty(), "not asked twice");
  // switches off: nothing is asked
  let mut res = Res::new(1.0);
  res.look.heat_bars = false;
  res.look.sub_flair = false;
  before_name(&mut res, &m);
  assert!(res.heats.take_wanted().is_empty() && res.flairs.take_wanted().is_empty());
  // a line with no twitch id (heatsync's archive) still gets a bar by login, but no flair or flame
  let mut res = Res::new(1.0);
  let bare = marked("");
  heated(&mut res, 120, false);
  assert_eq!(before_name(&mut res, &bare).0, BAR_W);
  assert!(res.flairs.take_wanted().is_empty());
}

#[test]
fn a_heat_answer_lays_out_only_that_chatters_lines_again() {
  let mut res = Res::new(1.0);
  let mut v = ChatView::new(10, "x".into());
  v.resize(500);
  for who in ["bob", "amy", "bob"] {
    let mut m = said(who, "hi");
    m.user_id = "42".into();
    v.push(m);
  }
  let area = Rect::new(0, 0, 500, 200);
  let mut c = Canvas::new(500, 200);
  v.draw(&mut c, &mut res, area, area);
  let bars = |v: &ChatView| v.rows.iter().map(|r| r.laid.as_ref().map_or(99, |l| l.nick.start)).collect::<Vec<_>>();
  assert_eq!(bars(&v), [0, 0, 0]);
  assert!(res.heats.put("bob", 300));
  v.touch_chatter(Platform::Twitch, "bob");
  assert_eq!(bars(&v), [99, 0, 99], "bob's two lines are stale, amy's is not");
  v.draw(&mut c, &mut res, area, area);
  assert_eq!(bars(&v), [BAR_W, 0, BAR_W]);
}
