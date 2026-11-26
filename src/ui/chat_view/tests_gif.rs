//! a twitch gif-keyboard line: the picture heatsync named for its caption draws where the caption was, once the
//! picture has a size. the real row: what it asks for, when the caption folds, and when it must not

use super::layout::{layout_row, picture};
use super::*;
use crate::images::decode::{Decoded, Frame};
use crate::model::chatgif::Picture;
use crate::ui::res::EMBED_PX;
use parley::PositionedLayoutItem;

const NO_OPTS: Opts = Opts { ts: false, tags: false, find: None };
const CAPTION: &str = "[Mood Lol GIF by Giphy]";
const URL: &str = "https://media.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif";
// every picture from another site is asked for through heatsync's proxy, so that site never sees the viewer
const ASKED: &str =
  "https://heatsync.org/api/img?url=https%3A%2F%2Fmedia.giphy.com%2Fmedia%2FjoSNxeswxuc74Juo8X%2Fgiphy.gif";

fn gif_line(body: &str) -> Message {
  let mut m = Message::system(1, Platform::Twitch, "x");
  m.kind = Kind::Chat;
  m.id = "m1".into();
  m.login = "bob".into();
  m.nick = "bob".into();
  m.body = body.into();
  m.extra().gif = Some(Picture { text: CAPTION.into(), url: URL.into() }.into());
  m
}

fn pic(res: &mut Res, m: &Message) -> (Option<crate::images::ImgId>, Option<std::ops::Range<usize>>) {
  let (id, fold, card) = picture(res, m, 400, &mut Vec::new());
  assert!(card.is_none(), "a gif line has no link card");
  (id, fold)
}

// the picture asked for, and decoded: 120x80 (or the failure of it)
fn load(res: &mut Res, m: &Message, got: Result<(), ()>) {
  let (id, _) = pic(res, m);
  res.images.want(id.unwrap());
  let (id, url, target) = res.images.take_wanted().remove(0);
  assert_eq!(&*url, ASKED);
  assert_eq!((target & 0xffff, target >> 16), (EMBED_PX as u32, 400 - MARK_W as u32 - 8), "fit to the row, capped");
  let d =
    got.map(|()| Decoded { w: 120, h: 80, frames: vec![Frame { rgba: vec![0; 120 * 80 * 4].into(), delay_ms: 0 }] });
  res.images.loaded(id, target, d.map_err(|()| "x".into()));
}

// how many bytes of text the row draws (the caption folded away is bytes fewer)
fn drawn(res: &mut Res, m: &Message) -> usize {
  let l = layout_row(res, "c", m, 0, 400, NO_OPTS);
  l.layout
    .lines()
    .flat_map(|l| l.items().collect::<Vec<_>>())
    .filter_map(|it| match it {
      PositionedLayoutItem::GlyphRun(g) => Some(g.run().text_range().len()),
      _ => None,
    })
    .sum()
}

#[test]
fn the_picture_draws_in_place_of_the_caption_once_it_has_a_size() {
  let mut res = Res::new(1.0);
  let m = gif_line(CAPTION);
  // born with it, not loaded yet: the caption stays and the picture is asked for
  let (id, fold) = pic(&mut res, &m);
  assert!(id.is_some() && fold.is_none());
  let before = drawn(&mut res, &m);
  load(&mut res, &m, Ok(()));
  let (_, fold) = pic(&mut res, &m);
  assert_eq!(fold, Some(0..CAPTION.len()));
  let l = layout_row(&mut res, "c", &m, 0, 400, NO_OPTS);
  assert_eq!(l.boxes.len(), 1, "the picture is on the row");
  // the caption left the text; the picture's own line break came in its place
  assert_eq!(drawn(&mut res, &m) + CAPTION.len(), before + 1);
}

#[test]
fn a_dead_picture_keeps_the_caption() {
  let mut res = Res::new(1.0);
  let m = gif_line(CAPTION);
  let before = drawn(&mut res, &m);
  load(&mut res, &m, Err(()));
  assert_eq!(pic(&mut res, &m), (None, None));
  assert_eq!(drawn(&mut res, &m), before);
  assert_eq!(layout_row(&mut res, "c", &m, 0, 400, NO_OPTS).boxes.len(), 0);
}

#[test]
fn a_line_with_words_besides_the_caption_keeps_them_and_the_caption() {
  let mut res = Res::new(1.0);
  let m = gif_line(&format!("lol {CAPTION} no way"));
  let before = drawn(&mut res, &m);
  load(&mut res, &m, Ok(()));
  let (id, fold) = pic(&mut res, &m);
  assert!(id.is_some(), "the picture still draws");
  assert_eq!(fold, None, "but the words stay");
  assert_eq!(drawn(&mut res, &m), before + 1, "nothing left the text; the picture's line break came");
  assert_eq!(layout_row(&mut res, "c", &m, 0, 400, NO_OPTS).boxes.len(), 1);
}

#[test]
fn a_replys_name_stays_and_only_the_caption_folds() {
  let mut res = Res::new(1.0);
  let mut m = gif_line(&format!("@amy {CAPTION}"));
  m.reply_to = Some("amy".into());
  load(&mut res, &m, Ok(()));
  assert_eq!(pic(&mut res, &m).1, Some(5..5 + CAPTION.len()));
  let mut plain = m.clone();
  plain.reply_to = None;
  assert_eq!(pic(&mut res, &plain).1, None, "the same words on a line that answers nobody");
}

#[test]
fn inline_media_off_keeps_the_caption_and_asks_for_nothing() {
  let mut res = Res::new(1.0);
  res.media = false;
  let m = gif_line(CAPTION);
  assert_eq!(pic(&mut res, &m), (None, None));
  assert!(res.images.take_wanted().is_empty());
  assert_eq!(layout_row(&mut res, "c", &m, 0, 400, NO_OPTS).embed, None);
}

#[test]
fn a_line_without_a_named_picture_is_the_caption_and_nothing_else() {
  let mut res = Res::new(1.0);
  let mut m = gif_line(CAPTION);
  m.x = None;
  assert_eq!(pic(&mut res, &m), (None, None));
  assert!(res.images.take_wanted().is_empty());
}

#[test]
fn the_gif_is_the_lines_picture_and_a_link_beside_it_is_not() {
  let mut res = Res::new(1.0);
  let m = gif_line(&format!("{CAPTION} https://i.imgur.com/a.png"));
  load(&mut res, &m, Ok(()));
  assert_eq!(pic(&mut res, &m).1, None, "words (a link) beside the caption keep it");
  assert!(layout_row(&mut res, "c", &m, 0, 400, NO_OPTS).embed.is_some());
}

// a picture that moves is the usual picture path: its frames come with it, and the cache cap holds
#[test]
fn an_animated_gif_keeps_its_frames_under_the_usual_cap() {
  let mut res = Res::new(1.0);
  let m = gif_line(CAPTION);
  let (id, _) = pic(&mut res, &m);
  res.images.want(id.unwrap());
  let (id, _, target) = res.images.take_wanted().remove(0);
  let frames = (0..3).map(|i| Frame { rgba: vec![i; 40 * 40 * 4].into(), delay_ms: 80 }).collect();
  res.images.loaded(id, target, Ok(Decoded { w: 40, h: 40, frames }));
  assert!(res.images.bytes() > 0 && res.images.bytes() <= crate::images::DEFAULT_CAP);
  assert!(res.images.frame_at(id, true, 0).is_some() && res.images.frame_at(id, true, 90).is_some());
}
