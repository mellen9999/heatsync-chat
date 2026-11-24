//! a modifier (`w! Kappa`, `Kappa ffzX`) changes its emote and is not drawn as a word, as on the site and
//! in the extension. the grammar and the binding rule run from the shared vectors in emotes/mods.rs; here
//! the line and the pixels

use super::layout::layout_row;
use super::*;
use crate::emotes::{Emote, Provider};
use crate::images::decode::{Decoded, Frame};
use crate::ui::canvas::Canvas;
use parley::PositionedLayoutItem;

const NO_OPTS: Opts = Opts { ts: false, tags: false, find: None };
// the test picture: a blue band along the top, red on the left under it, green on the right
const RED: u32 = 0xc80a0a;
const GREEN: u32 = 0x0ac80a;
const BLUE: u32 = 0x0a0ac8;
const PIC: &str = "https://cdn.7tv.app/emote/pic/1x.webp";
const WIDE: &str = "https://cdn.7tv.app/emote/lift/1x.webp";

fn said(body: &str) -> Message {
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.login = "bob".into();
  m.nick = "bob".into();
  m.body = body.into();
  m
}

// a room with `pic` (28×28), an overlay `lift0` (84×28) and an emote named like a modifier, all loaded
fn room(frames: usize) -> Res {
  let mut res = Res::new(1.0);
  let e = |url: &str, w: u16, zero_width: bool| Emote { url: url.into(), w, h: 28, zero_width };
  res.emotes.set(
    Some("c"),
    Provider::SevenTv,
    [("pic".into(), e(PIC, 28, false)), ("lift0".into(), e(WIDE, 84, true)), ("ffzX".into(), e(WIDE, 84, false))]
      .into(),
  );
  for (url, w) in [(PIC, 28u32), (WIDE, 84)] {
    let id = res.images.id(url, 28);
    res.images.want(id);
    let (_, _, want) = res.images.take_wanted().remove(0);
    let px = |x: u32, y: u32| {
      if y < 7 {
        BLUE
      } else if x < w / 2 {
        RED
      } else {
        GREEN
      }
    };
    let rgba: Vec<u8> = (0..28 * w)
      .flat_map(|i| {
        let p = px(i % w, i / w);
        [(p >> 16) as u8, (p >> 8) as u8, p as u8, 255]
      })
      .collect();
    let frame = || Frame { rgba: rgba.clone().into(), delay_ms: if frames > 1 { 50 } else { 0 } };
    res.images.loaded(id, want, Ok(Decoded { w, h: 28, frames: (0..frames).map(|_| frame()).collect() }));
  }
  res
}

// the line drawn in a 400×140 window
fn drawn(res: &mut Res, body: &str) -> (Canvas, ChatView) {
  let mut v = ChatView::new(50, "c".into());
  v.resize(400);
  v.glow = false;
  v.push(said(body));
  let (mut c, area) = (Canvas::new(400, 140), Rect::new(0, 0, 400, 140));
  v.draw(&mut c, res, area, area);
  crate::bench::dump(&c);
  (c, v)
}

// where a colour is on the canvas: left, top, right, bottom (inclusive)
fn at(c: &Canvas, ink: u32) -> (i32, i32, i32, i32) {
  let hit = (0..c.w * c.h).filter(|&i| c.px[i as usize] == ink).map(|i| (i % c.w, i / c.w)).collect::<Vec<_>>();
  assert!(!hit.is_empty(), "{ink:06x} is nowhere");
  let (xs, ys) = (hit.iter().map(|p| p.0), hit.iter().map(|p| p.1));
  (xs.clone().min().unwrap(), ys.clone().min().unwrap(), xs.max().unwrap(), ys.max().unwrap())
}

// the whole picture's box: width, height
fn size(c: &Canvas) -> (i32, i32) {
  let b = [RED, GREEN, BLUE].map(|k| at(c, k));
  let (l, t) = (b.iter().map(|b| b.0).min().unwrap(), b.iter().map(|b| b.1).min().unwrap());
  (b.iter().map(|b| b.2).max().unwrap() - l + 1, b.iter().map(|b| b.3).max().unwrap() - t + 1)
}

// the slots of a line: width, height
fn slots(res: &mut Res, m: &Message) -> Vec<(f32, f32)> {
  let l = layout_row(res, "c", m, 0, 400, NO_OPTS);
  l.layout
    .lines()
    .flat_map(|l| l.items().collect::<Vec<_>>())
    .filter_map(|it| match it {
      PositionedLayoutItem::InlineBox(b) if b.id != u64::MAX => Some((b.width, b.height)),
      _ => None,
    })
    .collect()
}

#[test]
fn a_modifier_is_not_drawn_as_a_word() {
  let mut res = room(1);
  // the words' ink: text is white at some coverage, a grey. the test picture has no grey in it
  let white = |res: &mut Res, body: &str| {
    drawn(res, body).0.px.iter().filter(|&&p| p != 0 && p >> 16 == (p >> 8) & 255 && p >> 16 == p & 255).count()
  };
  let alone = white(&mut res, "pic");
  for w in ["w!", "h!", "ffzX", "c!#f00", "w!h!ffzX", "p!", "ffzSpin", "z!"] {
    assert_eq!(white(&mut res, &format!("pic {w}")), alone, "{w} left ink on the line");
    assert_eq!(slots(&mut res, &said(&format!("pic {w}"))).len(), 1, "{w}: one emote, nothing more");
  }
  // several in a row all belong to it
  assert_eq!(white(&mut res, "pic w! h! v!"), alone);
  // before its emote too
  for w in ["w!", "ffzX", "c!#f00", "w!h!"] {
    assert_eq!(white(&mut res, &format!("{w} pic")), alone, "{w} before the emote left ink on the line");
  }
  // with no emote beside it, it is what was typed: a word, or the emote of that name
  assert!(white(&mut res, "hello w!") > white(&mut res, "hello"));
  assert!(white(&mut res, "pic hi w!") > white(&mut res, "pic hi"));
  assert!(white(&mut res, "w! hi pic") > white(&mut res, "hi pic"));
  assert_eq!(slots(&mut res, &said("ffzX")), [(84.0, 28.0)], "alone it is the room's emote of that name");
  assert_eq!(slots(&mut res, &said("hi ffzX")).len(), 1);
  // and a word that is no modifier stays a word after an emote
  assert!(white(&mut res, "pic w!!") > alone);
}

#[test]
fn a_plain_emote_is_drawn_as_it_always_was() {
  let (c, _) = drawn(&mut room(1), "pic");
  let (r, g, b) = (at(&c, RED), at(&c, GREEN), at(&c, BLUE));
  assert_eq!(size(&c), (28, 28));
  assert!(r.2 < g.0 && b.3 < r.1, "red left of green, the band on top: {r:?} {g:?} {b:?}");
}

#[test]
fn mirrors_and_turns_move_the_pixels() {
  let mut res = room(1);
  let mut look = |body: &str| {
    let (c, _) = drawn(&mut res, body);
    (at(&c, RED), at(&c, GREEN), at(&c, BLUE), size(&c))
  };
  for w in ["h!", "x!", "ffzX"] {
    let (r, g, b, s) = look(&format!("pic {w}"));
    assert!(g.2 < r.0 && b.3 < r.1 && s == (28, 28), "{w}: green left of red, band still on top");
  }
  for w in ["v!", "y!", "ffzY"] {
    let (r, g, b, s) = look(&format!("pic {w}"));
    assert!(r.2 < g.0 && b.1 > r.3 && s == (28, 28), "{w}: the band at the bottom");
  }
  // a quarter turn right: the top goes to the right side, the left side to the top
  let (r, g, b, s) = look("pic r!");
  assert!(b.0 > r.2 && r.3 < g.1 && s == (28, 28), "r!: {r:?} {g:?} {b:?}");
  let (r, g, b, _) = look("pic l!");
  assert!(b.2 < r.0 && r.1 > g.3, "l!: {r:?} {g:?} {b:?}");
  // two the same way is upside down and back to front; there and back is nothing
  let (r, g, b, _) = look("pic l! l!");
  assert!(g.2 < r.0 && b.1 > r.3);
  let (r, g, b, _) = look("pic l! r!");
  assert!(r.2 < g.0 && b.3 < r.1);
  // a mirror after a turn mirrors what the turn drew
  let (r, _, b, _) = look("pic r! h!");
  assert!(b.2 < r.0, "the band, sent right by the turn, is on the left");
}

#[test]
fn a_stretch_takes_its_room_on_the_line() {
  let mut res = room(1);
  let one = |res: &mut Res, body: &str| slots(res, &said(body))[0];
  assert_eq!(one(&mut res, "pic"), (28.0, 28.0));
  assert_eq!(one(&mut res, "pic w!"), (56.0, 28.0));
  assert_eq!(one(&mut res, "pic ffzTall"), (28.0, 56.0));
  assert_eq!(one(&mut res, "pic w! w!"), (112.0, 28.0));
  assert_eq!(one(&mut res, "pic w! w! w!"), (112.0, 28.0), "four times is the most");
  assert_eq!(one(&mut res, "pic w!ffzTall"), (56.0, 56.0));
  assert_eq!(one(&mut res, "w! pic"), (56.0, 28.0));
  // modifiers that cancel out leave the emote as it was
  assert_eq!(one(&mut res, "pic l! r!"), (28.0, 28.0));
  assert_eq!(layout_row(&mut res, "c", &said("pic l! r!"), 0, 400, NO_OPTS).boxes[0].fx.len(), 0);
  // drawn that size, whole, with nothing cut off and the row tall enough for it
  let (c, _) = drawn(&mut res, "pic w!");
  assert_eq!(size(&c), (56, 28));
  assert!(at(&c, RED).2 < at(&c, GREEN).0);
  let (c, _) = drawn(&mut res, "pic ffzTall");
  assert_eq!(size(&c), (28, 56));
  let l = layout_row(&mut res, "c", &said("pic ffzTall"), 0, 400, NO_OPTS);
  assert!(l.layout.height() >= 56.0);
  // a turned picture that is not square takes the room it now needs (the room emote named ffzX is 84×28)
  assert_eq!(one(&mut res, "ffzX l!"), (28.0, 84.0));
  let (c, _) = drawn(&mut res, "ffzX l!");
  assert_eq!(size(&c), (28, 84));
}

#[test]
fn a_modifier_belongs_to_the_picture_right_before_it() {
  let mut res = room(1);
  let fx = |res: &mut Res, body: &str| {
    let l = layout_row(res, "c", &said(body), 0, 400, NO_OPTS);
    l.boxes.iter().map(|b| (b.zw.len(), b.fx.iter().map(|f| f.0).collect::<Vec<_>>())).collect::<Vec<_>>()
  };
  // an ffz word is the emote's before it: the base, the overlay stacked on it, each
  assert_eq!(fx(&mut res, "pic ffzX lift0"), [(1, vec![0])]);
  assert_eq!(fx(&mut res, "pic lift0 ffzX"), [(1, vec![1])]);
  assert_eq!(fx(&mut res, "pic ffzX lift0 ffzW ffzY"), [(1, vec![0, 1])]);
  // a bttv word is the emote's after it, an overlay included; with none after it, the one before
  assert_eq!(fx(&mut res, "pic w! lift0"), [(1, vec![1])]);
  assert_eq!(fx(&mut res, "w! pic lift0"), [(1, vec![0])]);
  assert_eq!(fx(&mut res, "pic lift0 w!"), [(1, vec![1])]);
  // of two emotes only the one it names changes
  assert_eq!(fx(&mut res, "pic pic w!"), [(0, vec![]), (0, vec![0])]);
  assert_eq!(fx(&mut res, "pic w! pic"), [(0, vec![]), (0, vec![0])]);
  assert_eq!(fx(&mut res, "pic ffzX pic"), [(0, vec![0]), (0, vec![])]);
  assert_eq!(fx(&mut res, "w! hi pic"), [(0, vec![])], "never over a word");
  // the slot is the widest layer: the overlay is 84, doubled 168; a doubled base under it changes nothing
  assert_eq!(slots(&mut res, &said("pic lift0")), [(84.0, 28.0)]);
  assert_eq!(slots(&mut res, &said("pic lift0 w!")), [(168.0, 28.0)]);
  assert_eq!(slots(&mut res, &said("pic w! lift0")), [(168.0, 28.0)]);
  assert_eq!(slots(&mut res, &said("pic ffzW lift0")), [(84.0, 28.0)]);
  // drawn: the overlay covers its base, red then green; mirrored it is green then red
  let (c, _) = drawn(&mut res, "pic lift0");
  assert!(at(&c, RED).2 < at(&c, GREEN).0);
  let (c, _) = drawn(&mut res, "pic lift0 h!");
  assert!(at(&c, GREEN).2 < at(&c, RED).0);
}

#[test]
fn colours_change_and_the_shape_does_not() {
  let mut res = room(1);
  let count = |c: &Canvas, k: u32| c.px.iter().filter(|&&p| p == k).count();
  let green = at(&drawn(&mut res, "pic").0, GREEN);
  let (c, _) = drawn(&mut res, "pic c!");
  assert_eq!(count(&c, RED) + count(&c, GREEN) + count(&c, BLUE), 0, "cursed keeps no colour");
  // grey, darker, hard contrast: the red and the blue go black, the green half is one dark grey
  let dark = (green.1..=green.3).flat_map(|y| (green.0..=green.2).map(move |x| (x, y)));
  assert!(dark.clone().count() == 14 * 21 && dark.clone().all(|(x, y)| c.px[(y * c.w + x) as usize] == 0x404040));
  // a tint turns every colour of the picture and leaves its size
  let (c, _) = drawn(&mut res, "pic c!#00f");
  assert_eq!(count(&c, RED) + count(&c, GREEN) + count(&c, BLUE), 0);
  assert!(c.px[(green.1 * c.w + green.0) as usize] != 0, "and still draws it");
  assert_eq!(slots(&mut res, &said("pic c!#00f")), [(28.0, 28.0)]);
}

#[test]
fn a_big_emote_keeps_its_modifiers() {
  let mut res = room(1);
  let mut m = said("pic w!");
  m.extra().giant = true;
  assert_eq!(slots(&mut res, &m), [(112.0, 56.0)]);
}

// a moving picture is redrawn alone on each tick: the redraw is the modified picture too
#[test]
fn a_moving_emote_stays_modified_on_every_frame() {
  let mut res = room(2);
  let (mut c, mut v) = drawn(&mut res, "pic h!");
  assert!(at(&c, GREEN).2 < at(&c, RED).0);
  let area = Rect::new(0, 0, 400, 140);
  assert!(v.animate(&mut c, &mut res, area).is_some(), "the picture moves: a repaint is due");
  assert!(at(&c, GREEN).2 < at(&c, RED).0, "still mirrored after its own repaint");
  assert_eq!(size(&c), (28, 28));
}
