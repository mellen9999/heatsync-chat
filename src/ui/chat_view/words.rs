// a line's words become text and emote boxes: its emotes by position or by name, the overlays stacked on
// them, the modifiers that change them, links and names in the rest. and, read as it goes, whether the line
// is only pictures (emotes/only.rs), in which case it is laid out again at twice the height

use super::layout::inline;
use super::modify::{bindings, look, settle};
use super::{EmoteBox, Mention, Words};
use crate::emotes::mods::Fx;
use crate::emotes::only::{self, BIG_SCALE};
use crate::images::{ImgId, url};
use crate::model::message::Message;
use crate::ui::res::Res;
use crate::ui::text;
use parley::InlineBox;

// a gigantified emote's height before the display scale (ext .hs-mc-gigantified: 56px, twice the usual box)
const GIANT_PX: u16 = 56;

// words become text or emote boxes. twitch's own emotes come by position, the rest by name. `giant`: the
// last emote is drawn big, decoded at that height through the same picture path (the cache's cap holds).
// `big`: Some(the width a picture may take) when the row may draw a line of only pictures big. the line is
// read as it is drawn, one `only::Kind` per word, and when it is only pictures it is built again at twice the
// height from the same words (an overlay, a modifier, an emoji all grow with their picture). a picture that
// would not fit the row keeps the line as it was. true = it was drawn big
#[allow(clippy::too_many_arguments)]
pub(super) fn body(
  res: &mut Res,
  chan: &str,
  m: &Message,
  fold: Option<&std::ops::Range<usize>>,
  (giant, big): (bool, Option<f32>),
  s: &mut String,
  ib: &mut Vec<InlineBox>,
  boxes: &mut Vec<EmoteBox>,
) -> (Words, bool) {
  let (at, slots, held) = (s.len(), ib.len(), boxes.len());
  let (words, read) = lay_words(res, chan, m, fold, giant, 1, s, ib, boxes);
  let Some(room) = big.filter(|_| read.verdict().big) else { return (words, false) };
  s.truncate(at);
  ib.truncate(slots);
  boxes.truncate(held);
  let (words, _) = lay_words(res, chan, m, fold, false, BIG_SCALE, s, ib, boxes);
  if ib[slots..].iter().all(|b| b.width <= room) {
    return (words, true);
  }
  s.truncate(at);
  ib.truncate(slots);
  boxes.truncate(held);
  (lay_words(res, chan, m, fold, giant, 1, s, ib, boxes).0, false)
}

// the picture for an emote word at the height the line draws it. a big line asks for the provider's file
// that is tall enough (the 2x where chat has the 1x), and while that file fails it draws chat's own at the
// big height. the clock is the picture's own frames, whichever file it is: see Images::frame_at
fn pic(res: &mut Res, url: &str, px: u16, scaled: bool) -> ImgId {
  if scaled && let Some(r) = url::rung(url, res.images.px(px)) {
    let id = res.images.id(&r, px);
    if !res.images.failed(id) {
      return id;
    }
  }
  res.images.id(url, px)
}

// the words of the line at `scale` times the emote height (1 = usual). also what each word was drawn as
#[allow(clippy::too_many_arguments)]
fn lay_words(
  res: &mut Res,
  chan: &str,
  m: &Message,
  fold: Option<&std::ops::Range<usize>>,
  giant: bool,
  scale: u16,
  s: &mut String,
  ib: &mut Vec<InlineBox>,
  boxes: &mut Vec<EmoteBox>,
) -> (Words, only::Read) {
  let links = if res.links { crate::model::links::find(&m.body) } else { Vec::new() };
  // a feed post's bold / underlined runs, in bytes of the body
  let marks = m.x.as_deref().map_or(&[][..], |x| &x.marks[..]);
  let mut runs = Vec::new();
  let mut styled = Vec::new();
  let mut people = Vec::new();
  // `px` is the emote's usual height, what a word is looked up at; `dpx` the height it is drawn at
  let px = res.look.emote_px();
  let dpx = px * scale;
  let h = res.images.px(dpx);
  let mut read = only::Read::default();
  let mut prev_emote = false;
  // a lone emoji just written as text (where it starts in `s`, its body bytes): an overlay emote right
  // after it stacks on it, like on an emote
  let mut emoji: Option<(usize, usize, usize)> = None;
  let mut first = true;
  let mut at = 0usize;
  // the last emote drawn as a base: its box and its native size
  let mut last: Option<(usize, (u16, u16))> = None;
  // the top picture of the stack being built, at its drawn size, and the room the layers under it take
  let (mut layer, mut under) = ((0u32, 0u32), (0f32, 0f32));
  // the modifier words of the line and the emote each changes (`w! Kappa`, `Kappa ffzX`); the ones for
  // an emote still to come wait here
  let binds = bindings(res, chan, m, fold, px);
  let mut ahead = Fx::default();
  for (i, word) in m.body.split(' ').enumerate() {
    let (a, b) = (at, at + word.len());
    at = b + 1;
    // a link folded into its picture is still a link: not a picture of the line
    if fold.is_some_and(|f| f.start == a && f.end == b) {
      read.push(only::Kind::Text);
      continue;
    }
    // a modifier changes its emote and leaves the line: before any emote of its own name
    if let Some(&Some(to)) = binds.get(i) {
      if to > i {
        read.push(only::Kind::Mod);
        ahead.add(word);
        continue;
      }
      if prev_emote && let Some(top) = boxes.last() {
        read.push(only::Kind::Mod);
        let mut fx = top.fx.iter().find(|f| f.0 as usize == top.zw.len()).map(|f| f.1.clone()).unwrap_or_default();
        fx.add(word);
        settle(ib, boxes, layer, under, fx);
        continue;
      }
    }
    let (found, blocked) = look(res, chan, m, (a, b), word, px);
    match found {
      // a blocked overlay just vanishes; a blocked base keeps its slot
      Some((_, _, true, _)) if (prev_emote || emoji.is_some()) && blocked => continue,
      // an overlay on an emoji: the emoji's text becomes a slot with the emoji drawn under the overlay
      Some((url, dims, true, _)) if !prev_emote && let Some((p, ea, eb)) = emoji.take() => {
        read.push(only::Kind::Overlay);
        s.truncate(p);
        let img = pic(res, &url, dpx, scale > 1);
        let w = res.images.width(img, dims).max(h);
        (layer, under) = ((res.images.width(img, dims), h), (h as f32, h as f32));
        ib.push(inline(boxes.len(), s.len(), w, h));
        boxes.push(EmoteBox {
          img,
          zw: Vec::new(),
          zw_words: Vec::new(),
          bg: None,
          word: (a as u32, b as u32),
          blocked: false,
          stand_in: None,
          glyph: Some(m.body[ea..eb].into()),
          fx: Vec::new(),
        });
        settle(ib, boxes, layer, under, std::mem::take(&mut ahead));
        prev_emote = true;
      }
      Some((url, dims, true, _)) if prev_emote => {
        read.push(only::Kind::Overlay);
        let img = pic(res, &url, dpx, scale > 1);
        // an overlay wider than its base widens the slot: the whole stack is the slot, so each animation
        // frame repaints all of it (a slot only as wide as the base left the overlay's sides frozen)
        let w = res.images.width(img, dims) as f32;
        let id = boxes.len().wrapping_sub(1) as u64;
        if let Some(slot) = ib.iter_mut().rev().find(|i| i.id == id) {
          (layer, under) = ((w as u32, h), (slot.width, slot.height));
          slot.width = slot.width.max(w);
        }
        if let Some(last) = boxes.last_mut() {
          last.zw.push(img);
          last.zw_words.push((a as u32, b as u32));
        }
        settle(ib, boxes, layer, under, std::mem::take(&mut ahead));
        continue;
      }
      Some((url, dims, _, _)) => {
        // a blocked emote is a dashed box that holds its place, not a picture
        read.push(if blocked { only::Kind::Text } else { only::Kind::Emote });
        if !first {
          s.push(' ');
        }
        let img = pic(res, &url, dpx, scale > 1);
        let w = res.images.width(img, dims);
        emoji = None;
        (layer, under) = ((w, h), (0.0, h as f32));
        ib.push(inline(boxes.len(), s.len(), w, h));
        last = Some((boxes.len(), dims));
        boxes.push(EmoteBox {
          img,
          zw: Vec::new(),
          zw_words: Vec::new(),
          bg: None,
          word: (a as u32, b as u32),
          blocked,
          stand_in: None,
          glyph: None,
          fx: Vec::new(),
        });
        settle(ib, boxes, layer, under, std::mem::take(&mut ahead));
        prev_emote = true;
      }
      None => {
        // modifiers wait for an emote, never for a later one past text
        ahead = Fx::default();
        // a run carries on over the space before this word only when that word is text too
        let joined = !first && !prev_emote;
        if !first {
          s.push(' ');
        }
        let p0 = s.len();
        s.push_str(word);
        if !word.is_empty() {
          // emoji written as text are pictures too, one each; any other word is a word
          let pictures = text::emoji_only(word);
          if pictures == 0 {
            read.push(only::Kind::Text);
          }
          (0..pictures).for_each(|_| read.push(only::Kind::Emoji));
          emoji = (pictures == 1).then_some((p0, a, b));
          for mk in marks {
            let (lo, hi) = (mk.range.start as usize, mk.range.end as usize);
            if lo < b && a < hi {
              let from = if lo > a {
                p0 + lo - a
              } else if joined && lo < a {
                p0 - 1
              } else {
                p0
              };
              styled.push((from..p0 + hi.min(b) - a, mk.style));
            }
          }
        }
        // the words of a link answer clicks
        let mut linked = false;
        for (k, _) in links.iter().enumerate().filter(|(_, l)| l.range.start < b && a < l.range.end) {
          runs.push((p0..s.len(), k));
          linked = true;
        }
        // names in the words (a link's words are not names)
        if !linked && word.len() >= 3 {
          for f in crate::model::chatters::names(word) {
            let Some(p) = res.chatters.person(m.platform, &word[f.name.clone()], f.at) else { continue };
            people.push(Mention {
              range: p0 + f.full.start..p0 + f.full.end,
              login: word[f.name].to_ascii_lowercase().into(),
              color: p.color,
              paint: p.paint,
              art: None,
            });
          }
        }
        prev_emote = word.is_empty() && prev_emote;
      }
    }
    first = false;
  }
  if let (true, Some((k, dims))) = (giant, last) {
    gigantify(res, k, dims, ib, boxes);
  }
  ((links, runs, people, styled), read)
}

// the emote in box `k` drawn at the big height: its picture asked for at that size, its slot widened to
// match. one with overlays stacked on it, or a blocked one, stays as it was
fn gigantify(res: &mut Res, k: usize, dims: (u16, u16), ib: &mut [InlineBox], boxes: &mut [EmoteBox]) {
  let Some(b) = boxes.get_mut(k).filter(|b| b.zw.is_empty() && !b.blocked) else { return };
  let Some(url) = res.images.url(b.img).map(Box::<str>::from) else { return };
  let big = res.look.emote_px().max(GIANT_PX);
  let img = res.images.id(&url, big);
  let (w, h) = (res.images.width(img, dims), res.images.px(big));
  let (w, h) = b.fx.first().map_or((w, h), |f| f.1.size((w, h)));
  b.img = img;
  if let Some(slot) = ib.iter_mut().rev().find(|i| i.id == k as u64) {
    slot.width = w as f32;
    slot.height = h as f32;
  }
}
