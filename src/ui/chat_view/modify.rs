//! the modifier words of a line (`w! Kappa`, `Kappa ffzX`): which emote each changes, and the room that
//! emote then takes. the rule itself is emotes/mods.rs; this is the line's side of it

use super::EmoteBox;
use crate::emotes::mods::{self, Fx, Word};
use crate::emotes::resolve::{Found, find_word};
use crate::model::message::Message;
use crate::ui::res::Res;
use parley::InlineBox;

// the emote a word of a line is, as it will be drawn, and whether the viewer blocked it. a picture that
// failed to load reads as its name, plain text (ext b1f4970b): the slot takes the word's width. the cache
// tries it again later and a relayout follows, so a late success swaps the picture back
pub(super) fn look(
  res: &mut Res,
  chan: &str,
  m: &Message,
  (a, b): (usize, usize),
  word: &str,
  px: u16,
) -> (Option<Found>, bool) {
  let found = find_word(&mut res.sources(), chan, m, a, b, word);
  let base = crate::emotes::overlay_base(word);
  let hidden = |url: &str| res.blocked.contains(word, url) || base.is_some_and(|b| res.blocked.contains(b, url));
  let blocked = found.as_ref().is_some_and(|f| hidden(&f.0));
  let found = found.filter(|f| {
    blocked || {
      let id = res.images.id(&f.0, px);
      !res.images.failed(id)
    }
  });
  (found, blocked)
}

// for each word of the line, the word of the emote it changes when it is a modifier bound to one
// (emotes::mods::bind is the rule). empty for a line with no modifier word: nearly every line
pub(super) fn bindings(
  res: &mut Res,
  chan: &str,
  m: &Message,
  fold: Option<&std::ops::Range<usize>>,
  px: u16,
) -> Vec<Option<usize>> {
  if !m.body.split(' ').any(mods::is) {
    return Vec::new();
  }
  let mut at = 0usize;
  let mut line = Vec::new();
  for word in m.body.split(' ') {
    let (a, b) = (at, at + word.len());
    at = b + 1;
    let kind = if word.is_empty() || fold.is_some_and(|f| f.start == a && f.end == b) {
      Word::Gap
    } else {
      // a platform's own emote sits where the platform put it, whatever its name
      let placed = m.emotes.iter().any(|(r, _)| r.start as usize == a && r.end as usize == b);
      let emote = look(res, chan, m, (a, b), word, px).0.is_some();
      match (mods::is(word) && !placed, emote) {
        (true, emote) => Word::Mod { emote },
        (_, true) => Word::Emote,
        _ => Word::Text,
      }
    };
    line.push((kind, word));
  }
  mods::bind(&line)
}

// the modifiers of the top picture of the last box settle its slot: the room the layers under it take, or
// the picture's own once turned and stretched. modifiers that cancel out (`l! r!`) leave it as it was
pub(super) fn settle(ib: &mut [InlineBox], boxes: &mut [EmoteBox], layer: (u32, u32), under: (f32, f32), fx: Fx) {
  let Some(k) = boxes.len().checked_sub(1) else { return };
  let n = boxes[k].zw.len() as u8;
  let none = fx == Fx::default();
  if none && boxes[k].fx.iter().all(|f| f.0 != n) {
    return;
  }
  let (w, h) = fx.size(layer);
  if let Some(slot) = ib.iter_mut().rev().find(|i| i.id == k as u64) {
    (slot.width, slot.height) = (under.0.max(w as f32), under.1.max(h as f32));
  }
  boxes[k].fx.retain(|f| f.0 != n);
  if !none {
    boxes[k].fx.push((n, fx));
  }
}
