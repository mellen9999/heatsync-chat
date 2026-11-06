// a feed post's reaction chips (ext .hs-feed-chip), the lightest way to read: on the post's own line right
// after its last word when they fit, else one extra line, never more (what does not fit folds into a dim
// `+N`). a chip is the emote's picture at text height and its count right after; the count is dim, and yours
// is in reverse video, white behind black. the picture is the chip's own url through the image loader:
// never a word the chat resolves to an emote
use super::EmoteBox;
use super::layout::inline;
use crate::model::reaction::Reaction;
use crate::ui::res::Res;
use crate::ui::text::{Ink, Span};
use crate::ui::theme;
use parley::InlineBox;

// the picture's height: one text line (logical px)
const PIC_PX: u16 = 16;

// where a chip sits in the laid-out text, its emote, and the picture box inside it (a click on that counts)
pub(super) struct Spot {
  pub range: std::ops::Range<usize>,
  pub emote: u64,
  pub boxed: Option<u64>,
}

// the first `shown` chips, then `+N` for the rest, appended after whatever the row holds
pub(super) fn chips(
  res: &mut Res,
  list: &[Reaction],
  shown: usize,
  s: &mut String,
  spans: &mut Vec<Span>,
  ib: &mut Vec<InlineBox>,
  boxes: &mut Vec<EmoteBox>,
) -> Vec<Spot> {
  let mut spots = Vec::with_capacity(shown);
  let h = res.images.px(PIC_PX);
  let word = |s: &mut String, spans: &mut Vec<Span>, t: &str, ink: u32| {
    let a = s.len();
    s.push_str(t);
    spans.push(Span { range: a..s.len(), ink: Ink(ink), bold: false });
  };
  for r in list.iter().take(shown) {
    if !s.is_empty() && !s.ends_with(['\n', ' ']) {
      s.push(' ');
    }
    let a = s.len();
    let dim = if r.mine { theme::BLACK | Ink::REVERSE } else { theme::DIM | Ink::LIFT };
    let img = r.url.as_deref().map(|u| res.images.id(u, PIC_PX)).filter(|&id| !res.images.failed(id));
    let mut boxed = None;
    if let Some(img) = img {
      let w = res.images.width(img, (PIC_PX, PIC_PX));
      boxed = Some(boxes.len() as u64);
      ib.push(inline(boxes.len(), s.len(), w, h));
      boxes.push(EmoteBox {
        img,
        zw: Vec::new(),
        zw_words: Vec::new(),
        bg: r.mine.then_some(theme::WHITE),
        word: (0, 0),
        blocked: false,
        stand_in: None,
        glyph: None,
      });
      word(s, spans, &r.count.to_string(), dim);
    } else {
      word(s, spans, &format!("{} {}", r.name, r.count), dim);
    }
    spots.push(Spot { range: a..s.len(), emote: r.id, boxed });
  }
  if shown < list.len() {
    s.push(' ');
    word(s, spans, &format!("+{}", list.len() - shown), theme::DIM | Ink::LIFT);
  }
  spots
}
