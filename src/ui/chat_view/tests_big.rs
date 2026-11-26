//! big emote-only lines: a line of nothing but pictures (up to five) draws them twice as big. the rule runs
//! from the shared vectors in emotes/only.rs; here the real row: what it asks for, the room it holds and what
//! it draws, with the setting on and off

use super::layout::layout_row;
use super::*;
use crate::emotes::{Emote, Provider};
use crate::images::decode::{Decoded, Frame};
use crate::model::message::Notice;
use crate::ui::canvas::Canvas;
use crate::ui::res::EMOJI_MUL;
use parley::PositionedLayoutItem;

const NO_OPTS: Opts = Opts { ts: false, tags: false, find: None };
const RED: u32 = 0xc80a0a;
const GREEN: u32 = 0x0ac80a;
const BLUE: u32 = 0x0a0ac8;
// chat's file for the emote, and the files a big line asks for in its place (7tv: 1x, 2x, 4x)
const PIC: &str = "https://cdn.7tv.app/emote/pic/1x.webp";
const PIC_2X: &str = "https://cdn.7tv.app/emote/pic/2x.webp";
const PIC_4X: &str = "https://cdn.7tv.app/emote/pic/4x.webp";
const LIFT: &str = "https://cdn.7tv.app/emote/lift/1x.webp";
const LIFT_2X: &str = "https://cdn.7tv.app/emote/lift/2x.webp";
const ONE_FILE: &str = "https://cdn.heatsync.org/uploads/one.png";

fn said(body: &str) -> Message {
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.login = "bob".into();
  m.nick = "bob".into();
  m.body = body.into();
  m
}

// a room with `pic` (28×28), `pic2` (the same file), an overlay `lift0` and a base `lift` (84×28), and `one`
// (a heatsync picture: one file, no bigger rung)
fn room(scale: f32) -> Res {
  let mut res = Res::new(scale);
  let e = |url: &str, w: u16, zero_width: bool| Emote { url: url.into(), w, h: 28, zero_width };
  res.emotes.set(
    Some("c"),
    Provider::SevenTv,
    [
      ("pic".into(), e(PIC, 28, false)),
      ("pic2".into(), e(PIC, 28, false)),
      ("lift0".into(), e(LIFT, 84, true)),
      ("lift".into(), e(LIFT, 84, false)),
      ("one".into(), e(ONE_FILE, 28, false)),
    ]
    .into(),
  );
  res
}

// a picture decoded: `w`×`h`, a blue band along the top, red on the left under it, green on the right. every
// frame's first byte is its number, so which frame was drawn can be read back
fn load(res: &mut Res, url: &str, lh: u16, (w, h): (u32, u32), delays: &[u32]) {
  let id = res.images.id(url, lh);
  res.images.want(id);
  let (_, _, want) = res.images.take_wanted().remove(0);
  let px = |x: u32, y: u32| {
    if y < h / 4 {
      BLUE
    } else if x < w / 2 {
      RED
    } else {
      GREEN
    }
  };
  let frames = delays
    .iter()
    .enumerate()
    .map(|(n, &delay_ms)| {
      let mut rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| {
          let p = px(i % w, i / w);
          [(p >> 16) as u8, (p >> 8) as u8, p as u8, 255]
        })
        .collect();
      rgba[0] = n as u8;
      Frame { rgba: rgba.into(), delay_ms }
    })
    .collect();
  res.images.loaded(id, want, Ok(Decoded { w, h, frames }));
}

fn lay(res: &mut Res, m: &Message, look: u8, width: i32) -> super::Laid {
  layout_row(res, "c", m, look, width, NO_OPTS)
}

// the slots of a line, in its order: width, height
fn slots_at(res: &mut Res, m: &Message, look: u8, width: i32) -> Vec<(f32, f32)> {
  lay(res, m, look, width)
    .layout
    .lines()
    .flat_map(|l| l.items().collect::<Vec<_>>())
    .filter_map(|it| match it {
      PositionedLayoutItem::InlineBox(b) if b.id != u64::MAX => Some((b.width, b.height)),
      _ => None,
    })
    .collect()
}

fn slots(res: &mut Res, body: &str) -> Vec<(f32, f32)> {
  slots_at(res, &said(body), 0, 400)
}

fn height(res: &mut Res, m: &Message) -> f32 {
  lay(res, m, 0, 400).layout.height()
}

// the biggest type size on the line: an emoji at its drawn size beats the 14px of the words
fn biggest_type(res: &mut Res, body: &str) -> f32 {
  lay(res, &said(body), 0, 400)
    .layout
    .lines()
    .flat_map(|l| l.items().collect::<Vec<_>>())
    .filter_map(|it| match it {
      PositionedLayoutItem::GlyphRun(g) => Some(g.run().font_size()),
      _ => None,
    })
    .fold(0.0, f32::max)
}

// what drawing the line asks the image path for: the file and the height
fn asked(res: &mut Res, m: &Message) -> Vec<(String, u32)> {
  for b in &lay(res, m, 0, 400).boxes {
    res.images.want(b.img);
  }
  res.images.take_wanted().into_iter().map(|w| (w.1.into(), w.2 & 0xffff)).collect()
}

#[test]
fn a_line_of_only_emotes_draws_each_twice_as_big() {
  let mut res = room(1.0);
  assert_eq!(slots(&mut res, "pic"), [(56.0, 56.0)]);
  assert_eq!(slots(&mut res, "pic pic2 pic"), vec![(56.0, 56.0); 3]);
  // the row makes room for it
  let (big, small) = (height(&mut res, &said("pic")), height(&mut res, &said("hi pic")));
  assert!(big >= 56.0 && small < 56.0, "{big} vs {small}");
}

#[test]
fn five_emotes_are_big_and_six_are_a_wall() {
  let mut res = room(1.0);
  assert_eq!(slots(&mut res, "pic pic pic pic pic"), vec![(56.0, 56.0); 5]);
  assert_eq!(slots(&mut res, "pic pic pic pic pic pic"), vec![(28.0, 28.0); 6]);
  assert_eq!(slots(&mut res, "pic pic pic pic pic pic pic pic pic pic pic pic"), vec![(28.0, 28.0); 12]);
}

#[test]
fn a_word_a_link_a_mention_or_an_unknown_name_keeps_the_line_normal() {
  let mut res = room(1.0);
  for line in [
    "lol pic",
    "pic lol",
    "pic lol pic",
    "pic https://heatsync.org",
    "@someone pic",
    "!hype pic",
    "pic!",
    "pic nope",
    "pic , pic",
  ] {
    assert!(slots(&mut res, line).iter().all(|s| *s == (28.0, 28.0)), "{line}");
  }
  // nothing to draw is nothing to enlarge
  assert!(slots(&mut res, "").is_empty() && slots(&mut res, "just words").is_empty());
}

// the line's own spacing doesn't count as a word
#[test]
fn a_double_space_between_emotes_is_still_an_emote_only_line() {
  let mut res = room(1.0);
  assert_eq!(slots(&mut res, "pic  pic"), vec![(56.0, 56.0); 2]);
}

#[test]
fn a_modifier_and_an_overlay_are_not_counted_and_grow_with_their_emote() {
  let mut res = room(1.0);
  // the stretch is the modifier's: 2x wide, and the whole slot is twice as big
  assert_eq!(slots(&mut res, "w! pic"), [(112.0, 56.0)]);
  assert_eq!(slots(&mut res, "pic ffzX"), [(56.0, 56.0)]);
  // an overlay wider than its base widens the slot: 84 → 168
  assert_eq!(slots(&mut res, "pic lift0"), [(168.0, 56.0)]);
  assert_eq!(slots(&mut res, "w! pic lift0 pic2"), [(168.0, 56.0), (56.0, 56.0)]);
  // five emotes each with a modifier are five; six are a wall, modifiers or not
  assert_eq!(slots(&mut res, "w! pic w! pic w! pic w! pic w! pic"), vec![(112.0, 56.0); 5]);
  assert!(slots(&mut res, "w! pic pic pic pic pic pic").iter().all(|s| s.1 == 28.0));
  // a modifier with nothing to change is a word
  assert!(slots(&mut res, "pic w! hello").iter().all(|s| s.1 == 28.0));
}

#[test]
fn an_emoji_line_draws_its_emoji_twice_as_big() {
  let mut res = room(1.0);
  let (font, usual) = (res.text.font_px, res.text.font_px * f32::from(EMOJI_MUL));
  assert_eq!(biggest_type(&mut res, "😂"), usual * 2.0);
  assert_eq!(biggest_type(&mut res, "😂😂😂"), usual * 2.0, "glued together, a picture each");
  assert_eq!(biggest_type(&mut res, "😂 😂 😂 😂 😂"), usual * 2.0);
  assert_eq!(biggest_type(&mut res, "😂😂😂😂😂😂"), usual, "six are a wall");
  assert_eq!(biggest_type(&mut res, "hi 😂"), usual, "a word keeps it normal");
  assert!(usual > font);
  // an emote and an emoji are two pictures on one line
  assert_eq!(slots(&mut res, "pic 😂"), [(56.0, 56.0)]);
  assert_eq!(biggest_type(&mut res, "pic 😂"), usual * 2.0);
  assert_eq!(biggest_type(&mut res, "pic hi 😂"), usual);
  // an overlay stacked on an emoji grows with it
  assert_eq!(slots(&mut res, "😂 lift0"), [(168.0, 56.0)]);
  assert!(height(&mut res, &said("😂")) > height(&mut res, &said("hi 😂")) + 20.0);
}

#[test]
fn off_every_emote_keeps_one_size_as_before() {
  let mut res = room(1.0);
  res.look.big_lines = false;
  assert_eq!(slots(&mut res, "pic"), [(28.0, 28.0)]);
  assert_eq!(slots(&mut res, "pic pic2 pic"), vec![(28.0, 28.0); 3]);
  assert_eq!(slots(&mut res, "w! pic lift0"), [(84.0, 28.0)]);
  assert_eq!(biggest_type(&mut res, "😂"), res.text.font_px * f32::from(EMOJI_MUL));
  // the same asks as ever: the chat file, at the usual height, and nothing of a bigger rung
  assert_eq!(asked(&mut res, &said("pic")), [(PIC.to_owned(), 28)]);
  // so a line's height is what any line with its emote is
  assert_eq!(height(&mut res, &said("pic")), height(&mut res, &said("hi pic")));
  // on again: big
  res.look.big_lines = true;
  assert_eq!(slots(&mut res, "pic"), [(56.0, 56.0)]);
}

// the account's switch reaches the row: Settings → Look → the line
#[test]
fn the_setting_is_what_turns_it_off() {
  let mut res = room(1.0);
  load(&mut res, PIC, 28, (28, 28), &[0]);
  load(&mut res, PIC_2X, 56, (56, 56), &[0]);
  let mut s = crate::model::settings::Settings::default();
  assert!(s.big_emote_lines, "on by default");
  assert_eq!(size(&drawn(&mut res, "pic").0), (56, 56));
  s.big_emote_lines = false;
  res.look = crate::ui::res::Look::of(&s);
  assert_eq!(size(&drawn(&mut res, "pic").0), (28, 28));
}

// the line drawn in a 400×140 window
fn drawn(res: &mut Res, body: &str) -> (Canvas, ChatView) {
  drawn_row(res, said(body))
}

fn drawn_row(res: &mut Res, m: Message) -> (Canvas, ChatView) {
  let mut v = ChatView::new(50, "c".into());
  v.resize(400);
  v.glow = false;
  v.push(m);
  let (mut c, area) = (Canvas::new(400, 140), Rect::new(0, 0, 400, 140));
  v.draw(&mut c, res, area, area);
  crate::bench::dump(&c);
  (c, v)
}

// the whole picture's box: width, height
fn size(c: &Canvas) -> (i32, i32) {
  let at = |ink: u32| {
    let hit = (0..c.w * c.h).filter(|&i| c.px[i as usize] == ink).map(|i| (i % c.w, i / c.w)).collect::<Vec<_>>();
    assert!(!hit.is_empty(), "{ink:06x} is nowhere");
    let (xs, ys) = (hit.iter().map(|p| p.0), hit.iter().map(|p| p.1));
    (xs.clone().min().unwrap(), ys.clone().min().unwrap(), xs.max().unwrap(), ys.max().unwrap())
  };
  let b = [RED, GREEN, BLUE].map(at);
  let (l, t) = (b.iter().map(|b| b.0).min().unwrap(), b.iter().map(|b| b.1).min().unwrap());
  (b.iter().map(|b| b.2).max().unwrap() - l + 1, b.iter().map(|b| b.3).max().unwrap() - t + 1)
}

#[test]
fn a_big_line_draws_the_bigger_picture() {
  let mut res = room(1.0);
  load(&mut res, PIC, 28, (28, 28), &[0]);
  // a big line asks for the provider's 2x file at the big height
  load(&mut res, PIC_2X, 56, (56, 56), &[0]);
  assert_eq!(size(&drawn(&mut res, "pic").0), (56, 56));
  assert_eq!(size(&drawn(&mut res, "lol pic").0), (28, 28));
  // a modifier stretches the big picture, as it does the usual one
  assert_eq!(size(&drawn(&mut res, "w! pic").0), (112, 56));
}

#[test]
fn a_big_emote_asks_for_the_providers_next_file_at_the_big_height() {
  let mut res = room(1.0);
  assert_eq!(asked(&mut res, &said("pic")), [(PIC_2X.to_owned(), 56)]);
  // an emote with one file only is asked for at the big height from that file
  assert_eq!(asked(&mut res, &said("one")), [(ONE_FILE.to_owned(), 56)]);
  // a usual line: the usual file at the usual height
  assert_eq!(asked(&mut res, &said("hi pic")), [(PIC.to_owned(), 28)]);
  // the display scale applies to the big height like to every picture, and picks the file that holds it
  let mut hidpi = room(2.0);
  assert_eq!(asked(&mut hidpi, &said("pic")), [(PIC_4X.to_owned(), 112)]);
  assert_eq!(asked(&mut hidpi, &said("hi pic")), [(PIC.to_owned(), 56)]);
}

#[test]
fn a_bigger_file_that_fails_falls_back_to_the_one_chat_has() {
  let mut res = room(1.0);
  let first = lay(&mut res, &said("pic"), 0, 400);
  let id = first.boxes[0].img;
  assert_eq!(res.images.url(id), Some(PIC_2X));
  res.images.want(id);
  let (id, _, target) = res.images.take_wanted().remove(0);
  assert!(res.images.loaded(id, target, Err("404".into())), "a failure lays the rows out again");
  // laid out again: still big, from the chat file
  let again = lay(&mut res, &said("pic"), 0, 400);
  assert_eq!(res.images.url(again.boxes[0].img), Some(PIC));
  assert_eq!(slots(&mut res, "pic"), [(56.0, 56.0)]);
  assert_eq!(asked(&mut res, &said("pic")), [(PIC.to_owned(), 56)]);
}

// the box is held from the first layout, from the emote's own shape, so the decode changes nothing
#[test]
fn the_big_slot_is_held_before_the_picture_lands() {
  let mut res = room(1.0);
  for (body, want) in [("pic", (56u32, 56u32)), ("lift", (168, 56))] {
    let l = lay(&mut res, &said(body), 0, 400);
    let img = l.boxes[0].img;
    res.images.want(img);
    let (id, _, target) = res.images.take_wanted().remove(0);
    let relayout = res.images.loaded(id, target, Ok(Decoded { w: want.0, h: want.1, frames: one_frame(want) }));
    assert!(!relayout, "{body}: the picture is the size the slot was held at");
  }
}

fn one_frame((w, h): (u32, u32)) -> Vec<Frame> {
  vec![Frame { rgba: vec![0; (w * h * 4) as usize].into(), delay_ms: 0 }]
}

#[test]
fn a_picture_wider_than_the_row_keeps_the_line_normal() {
  let mut res = room(1.0);
  // `lift` is 84 wide: 168 big. a 400 window holds it; a 150 one does not
  assert_eq!(slots_at(&mut res, &said("lift"), 0, 400), [(168.0, 56.0)]);
  assert_eq!(slots_at(&mut res, &said("lift"), 0, 150), [(84.0, 28.0)]);
  assert_eq!(slots_at(&mut res, &said("pic"), 0, 150), [(56.0, 56.0)]);
  // the room is a number of pixels, not a count: it is the row's width less its edge and padding
  let room_w = 150.0 - MARK_W as f32 - 2.0 * theme::PAD_X;
  assert!((84.0..168.0).contains(&room_w), "{room_w}");
}

#[test]
fn only_plain_chat_rows_draw_big() {
  use crate::model::filter;
  let mut res = room(1.0);
  let big = |res: &mut Res, m: &Message, look: u8| slots_at(res, m, look, 400) == [(56.0, 56.0)];
  let plain = said("pic");
  assert!(big(&mut res, &plain, 0));
  // a /me line is a plain line
  let mut me = said("pic");
  me.kind = Kind::Action;
  assert!(big(&mut res, &me, 0));
  // a reply: the body draws big, the line above it (the parent's words) does not change
  let mut reply = said("pic");
  reply.reply_to = Some("amy".into());
  reply.reply_text = Some("pic pic".into());
  assert!(big(&mut res, &reply, 0));
  // every filled row stays as it is
  let flagged = |f: u16| {
    let mut m = said("pic");
    m.flags |= f;
    m
  };
  let noticed = |n: Notice| {
    let mut m = said("pic");
    m.system = Some("amy subscribed".into());
    m.extra().notice = n;
    m
  };
  let mut intro = said("pic");
  intro.extra().intro = true;
  let mut effect = said("pic");
  effect.extra().fx = "simmer".into();
  let mut event = said("pic");
  event.kind = Kind::Event;
  event.color = theme::GREEN;
  let rows = [
    ("first message", flagged(flag::FIRST)),
    ("highlight", flagged(flag::HIGHLIGHTED)),
    ("mention", flagged(flag::ALERT)),
    ("sub", noticed(Notice::Sub)),
    ("raid", noticed(Notice::Raid)),
    ("announcement", noticed(Notice::Announce)),
    ("redeem", noticed(Notice::Redeem)),
    ("new here", intro),
    ("message effect", effect),
    ("stream event", event),
  ];
  for (name, m) in &rows {
    assert!(!big(&mut res, m, 0), "{name}");
    assert!(slots_at(&mut res, m, 0, 400).iter().all(|s| *s == (28.0, 28.0)), "{name} keeps the usual size");
  }
  // a rule's fill is a fill
  assert!(!big(&mut res, &plain, filter::MARKED), "rule hit");
  // a notice is not a picture row at all
  let notice = Message::system(1, Platform::Twitch, "pic");
  assert!(slots_at(&mut res, &notice, 0, 400).iter().all(|s| *s == (28.0, 28.0)));
}

// the platform's own emotes (twitch's tags place them by position) are pictures too, from their own bigger file
#[test]
fn a_platforms_own_emotes_are_pictures_of_the_line() {
  let mut res = room(1.0);
  let (kappa, keepo) = (crate::emotes::twitch_url("25"), crate::emotes::twitch_url("1902"));
  let mut m = said("Kappa Keepo");
  m.emotes = vec![(0..5, kappa.clone().into()), (6..11, keepo.into())].into();
  assert_eq!(slots_at(&mut res, &m, 0, 400), vec![(56.0, 56.0); 2]);
  let want = kappa.replace("/1.0", "/2.0");
  assert_eq!(asked(&mut res, &m)[0], (want, 56));
  // with a word between them it is a usual line
  let mut said_too = said("Kappa hi Keepo");
  said_too.emotes = vec![(0..5, kappa.into()), (9..14, crate::emotes::twitch_url("1902").into())].into();
  assert!(slots_at(&mut res, &said_too, 0, 400).iter().all(|s| *s == (28.0, 28.0)));
}

// a click, a hover or a block on a big emote is about the emote chat knows by that word, not the file the
// big line asked for
#[test]
fn a_big_emote_under_the_pointer_is_the_emote_not_its_bigger_file() {
  let mut res = room(1.0);
  let mut v = ChatView::new(50, "c".into());
  v.resize(400);
  v.push(said("pic"));
  let area = Rect::new(0, 0, 400, 140);
  let hit = (0..400).step_by(2).find_map(|x| (84..140).find_map(|y| v.emote_at(area, x, y, &mut res)));
  let hit = hit.expect("the big emote is under the pointer");
  assert_eq!((&*hit.name, &*hit.url), ("pic", PIC));
}

// a picture that failed to load reads as its name (see look): a word, so the line is a normal one
#[test]
fn an_emote_that_failed_to_load_is_a_word_so_the_line_stays_normal() {
  let mut res = room(1.0);
  assert_eq!(slots(&mut res, "pic lift"), [(56.0, 56.0), (168.0, 56.0)]);
  let id = res.images.id(LIFT, 28);
  res.images.want(id);
  let (id, _, target) = res.images.take_wanted().remove(0);
  assert!(res.images.loaded(id, target, Err("404".into())));
  assert_eq!(slots(&mut res, "pic lift"), [(28.0, 28.0)], "lift is its name now, and the line has a word");
  assert_eq!(slots(&mut res, "pic"), [(56.0, 56.0)]);
}

#[test]
fn a_blocked_emote_is_not_a_picture_on_the_line() {
  use crate::emotes::block::Scope;
  let mut res = room(1.0);
  res.blocked.block("pic", PIC, Scope::Channel);
  // the dashed box that holds its place keeps the line as it was
  assert!(slots(&mut res, "pic").iter().all(|s| *s == (28.0, 28.0)));
  assert!(slots(&mut res, "pic pic2").iter().all(|s| *s == (28.0, 28.0)));
}

// a power-up's gigantified emote is already twice as big: a big line does not make it four times
#[test]
fn a_gigantified_big_line_is_twice_as_big_not_four_times() {
  let mut res = room(1.0);
  let mut m = said("pic");
  m.extra().giant = true;
  assert_eq!(slots_at(&mut res, &m, 0, 400), [(56.0, 56.0)]);
  let mut two = said("pic pic2");
  two.extra().giant = true;
  assert_eq!(slots_at(&mut res, &two, 0, 400), vec![(56.0, 56.0); 2]);
  // and with the setting off it is still the power-up's own: the last emote only
  res.look.big_lines = false;
  assert_eq!(slots_at(&mut res, &two, 0, 400), [(28.0, 28.0), (56.0, 56.0)]);
}

// every animated emote on screen is read off the one clock: the frame is a function of the time and the
// picture's own delays, never of its size or of which file it came from
#[test]
fn a_big_emote_shows_the_same_frame_as_the_usual_one_at_the_same_instant() {
  let mut res = room(1.0);
  let delays = [30, 50, 70, 90, 20, 140];
  let big = lay(&mut res, &said("pic"), 0, 400).boxes[0].img;
  let usual = lay(&mut res, &said("hi pic"), 0, 400).boxes[0].img;
  assert_ne!(big, usual, "two pictures: the 2x file at 56 and the 1x file at 28");
  load(&mut res, PIC_2X, 56, (56, 56), &delays);
  load(&mut res, PIC, 28, (28, 28), &delays);
  let total: u64 = delays.iter().map(|&d| u64::from(d)).sum();
  let mut seen = std::collections::BTreeSet::new();
  for now in 0..total * 3 {
    let a = res.images.frame_at(big, true, now).map(|f| (f.0.rgba[0], f.1, f.3));
    let b = res.images.frame_at(usual, true, now).map(|f| (f.0.rgba[0], f.1, f.3));
    let (a, b) = (a.expect("the big picture is here"), b.expect("the usual picture is here"));
    assert_eq!((a.0, a.2), (b.0, b.2), "frame and time to the next one, at {now}ms");
    assert_eq!((a.1, b.1), (56, 28), "each at its own size");
    seen.insert(a.0);
  }
  assert_eq!(seen.len(), delays.len(), "every frame came round");
  // held on its first frame, both, while animation is off
  assert_eq!(res.images.frame_at(big, false, 77).map(|f| f.0.rgba[0]), Some(0));
}

// the repaint of what moves is registered for the big emote like for any other: it is redrawn on its tick
#[test]
fn a_moving_big_emote_is_repainted_on_the_same_tick_path() {
  let mut res = room(1.0);
  load(&mut res, PIC_2X, 56, (56, 56), &[40, 40, 40]);
  load(&mut res, PIC, 28, (28, 28), &[40, 40, 40]);
  let (mut c, mut v) = drawn(&mut res, "pic");
  assert_eq!(v.anims.len(), 1, "it moves");
  assert_eq!((v.anims[0].rect.w, v.anims[0].rect.h), (56, 56));
  assert!(v.next_anim.is_some(), "a repaint is due");
  let area = Rect::new(0, 0, 400, 140);
  assert!(v.animate(&mut c, &mut res, area).is_some());
  assert_eq!(size(&c), (56, 56), "still its own size after its repaint");
  // and the usual one beside it, in another line, is the same path
  let (_, v) = drawn(&mut res, "hi pic");
  assert_eq!((v.anims[0].rect.w, v.anims[0].rect.h), (28, 28));
}

// every kind of line in one window, for the eye: HSC_DUMP=big.ppm cargo test a_chat_of_every_kind_of_line
#[test]
fn a_chat_of_every_kind_of_line() {
  let mut res = room(1.0);
  for (url, lh, size) in [(PIC, 28, (28, 28)), (PIC_2X, 56, (56, 56)), (LIFT, 28, (84, 28)), (LIFT_2X, 56, (168, 56))] {
    load(&mut res, url, lh, size, &[0]);
  }
  let mut first = said("pic");
  first.flags |= flag::FIRST;
  let mut me = said("pic pic");
  me.kind = Kind::Action;
  let mut reply = said("pic pic2");
  reply.reply_to = Some("amy".into());
  reply.reply_text = Some("any emote wars tonight".into());
  let rows = [
    said("a line with words and an emote pic in it"),
    said("pic"),
    said("pic pic2 pic"),
    said("pic lift0"),
    said("w! pic"),
    said("😂 😂 😂"),
    said("pic pic pic pic pic pic pic"),
    first,
    me,
    reply,
  ];
  let mut v = ChatView::new(50, "c".into());
  v.resize(480);
  v.glow = false;
  for m in rows {
    v.push(m);
  }
  let (mut c, area) = (Canvas::new(480, 560), Rect::new(0, 0, 480, 560));
  v.draw(&mut c, &mut res, area, area);
  crate::bench::dump(&c);
  // the rows, top to bottom: the big ones are the tall ones
  let hs: Vec<i32> = v.rows.iter().map(|r| r.height).collect();
  let usual = hs[0];
  assert!(usual < 40, "{hs:?}");
  for (i, big) in [(1, true), (2, true), (3, true), (4, true), (5, true), (6, false), (7, false), (8, true), (9, true)]
  {
    assert_eq!(hs[i] > usual + 20, big, "row {i}: {hs:?}");
  }
}
