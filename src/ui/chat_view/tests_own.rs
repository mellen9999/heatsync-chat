//! your own line on kick or youtube wears the badges and name colour your twitch self has in that room: a send
//! to several feeds keeps whichever echo lands first, and kick's copy has kick's colour and no twitch badges

use super::layout::layout_row;
use super::*;
use crate::badges::{Badge, key};
use crate::emotes::{Emote, Provider};
use crate::ui::res::Res;
use parley::PositionedLayoutItem;

const NO_OPTS: Opts = Opts { ts: false, tags: false, find: None };
const ORANGE: u32 = 0xff8700;
const KICK_GREEN: u32 = 0x75fd46;

fn line(p: Platform, login: &str, color: u32, badges: &[(&str, &str)]) -> Message {
  let mut m = Message::system(1, p, "x");
  m.kind = Kind::Chat;
  m.login = login.into();
  m.nick = login.into();
  m.body = "hi".into();
  m.color = color;
  m.badges = badges.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect();
  m
}

// a viewer linked as mellen on twitch (id 7) and kick, with the room's twitch badge art loaded
fn res() -> Res {
  let mut res = Res::new(1.0);
  res.senders.set_me(Some(7));
  res.senders.set_names(Some("mellen"), Some("mellen"), None);
  let art = |u: &str| Badge { url: u.into(), bg: None };
  res.badges.set_twitch(
    Some("c"),
    [(key("moderator", "1"), art("https://x/mod")), (key("subscriber", "108"), art("https://x/sub"))].into(),
  );
  res
}

// the brush the name's glyphs were laid out in
fn name_ink(l: &Laid) -> u32 {
  let mut seen = None;
  for item in l.layout.lines().flat_map(|ln| ln.items().collect::<Vec<_>>()) {
    if let PositionedLayoutItem::GlyphRun(gr) = item {
      let at = gr.run().text_range();
      if at.start < l.nick.end && l.nick.start < at.end {
        seen = Some(gr.style().brush.0 & 0xff_ffff);
      }
    }
  }
  seen.expect("the name has glyphs")
}

fn twitch_self() -> Message {
  line(Platform::Twitch, "mellen", ORANGE, &[("moderator", "1"), ("subscriber", "108")])
}

#[test]
fn your_kick_line_wears_your_twitch_badges_and_colour() {
  let mut res = res();
  let kick = line(Platform::Kick, "mellen", KICK_GREEN, &[]);
  let plain = layout_row(&mut res, "c", &kick, 0, 600, NO_OPTS);
  assert_eq!((plain.boxes.len(), name_ink(&plain)), (0, KICK_GREEN), "before your twitch self is heard");
  assert!(res.own.note("c", &twitch_self()), "the first line of yours says something new");
  let l = layout_row(&mut res, "c", &kick, 0, 600, NO_OPTS);
  assert_eq!(l.boxes.len(), 2, "mod + sub art");
  assert_eq!(name_ink(&l), ORANGE);
}

#[test]
fn only_your_own_kick_line_borrows_it() {
  let mut res = res();
  res.own.note("c", &twitch_self());
  let other = layout_row(&mut res, "c", &line(Platform::Kick, "bob", KICK_GREEN, &[]), 0, 600, NO_OPTS);
  assert_eq!((other.boxes.len(), name_ink(&other)), (0, KICK_GREEN));
  let theirs = layout_row(&mut res, "c", &line(Platform::Twitch, "bob", KICK_GREEN, &[]), 0, 600, NO_OPTS);
  assert_eq!((theirs.boxes.len(), name_ink(&theirs)), (0, KICK_GREEN), "a twitch line wears its own tags");
  let elsewhere = layout_row(&mut res, "other", &line(Platform::Kick, "mellen", KICK_GREEN, &[]), 0, 600, NO_OPTS);
  assert_eq!(elsewhere.boxes.len(), 0, "another room's badges are another room's");
}

#[test]
fn a_twitch_line_keeps_its_own_tags() {
  let mut res = res();
  let mut other = twitch_self();
  other.color = 0x00ff00;
  other.badges = Box::new([]);
  res.own.note("c", &twitch_self());
  let l = layout_row(&mut res, "c", &other, 0, 600, NO_OPTS);
  assert_eq!((l.boxes.len(), name_ink(&l)), (0, 0x00ff00));
}

// the room has a 7tv emote and kick's room has none: a kick row draws it as text, and a kick row that stands for a
// folded send (the twitch copy folded into it) draws the picture, whoever sent it
#[test]
fn a_folded_sends_kick_row_draws_the_twitch_rooms_emote() {
  let mut res = res();
  let pic = Emote { url: "https://cdn.7tv.app/emote/MAN/1x.webp".into(), w: 28, h: 28, zero_width: false };
  res.emotes.set(Some("c"), Provider::SevenTv, [("MANOFSTEEL".into(), pic)].into());
  for who in ["mellen", "bob"] {
    let mut kick = line(Platform::Kick, who, KICK_GREEN, &[]);
    kick.body = "MANOFSTEEL".into();
    let alone = layout_row(&mut res, "c", &kick, 0, 600, NO_OPTS);
    assert_eq!(alone.boxes.len(), 0, "{who}: kick's room has no such emote, so it is a word");
    kick.extra().twitch_leg = true;
    let folded = layout_row(&mut res, "c", &kick, 0, 600, NO_OPTS);
    assert_eq!(folded.boxes.len(), 1, "{who}: the twitch room's emote is drawn");
  }
}
