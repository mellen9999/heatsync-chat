// heatsync's marks before a twitch name, after the badges: the sub flair (a badge picture), a flame for a
// known heatsync user, the heat bar. what each line wears is decided by `heatsync::marks::wear`; this only
// draws it. a line with none of them adds nothing to the text and allocates nothing
use super::EmoteBox;
use super::layout::{gap, push_badge};
use crate::cosmetics::id::PaintId;
use crate::cosmetics::plus as tenure;
use crate::heatsync::marks::wear;
use crate::model::message::{Message, Platform};
use crate::ui::res::Res;
use crate::ui::text::{Ink, Span};
use parley::InlineBox;

const FLAME: &str = "\u{1f525}";

pub(super) fn marks(
  res: &mut Res,
  m: &Message,
  s: &mut String,
  spans: &mut Vec<Span>,
  ib: &mut Vec<InlineBox>,
  boxes: &mut Vec<EmoteBox>,
) {
  let look = res.look;
  if !m.kind.said() || !(look.sub_flair || look.flame || look.heat_bars) {
    return;
  }
  // your own kick or youtube line wears your twitch self's marks: a send to several feeds keeps whichever
  // echo lands first, so the row standing for it is often not the twitch copy (ext hsMarksSelfRowHtml)
  let own = res.senders.own_twitch(m).map(|(login, id)| (Box::<str>::from(login), id));
  if m.platform != Platform::Twitch && own.is_none() {
    return;
  }
  let (login, uid, paint) = match &own {
    Some((login, id)) => (&**login, Some(*id), Some(PaintId::Twitch(*id))),
    None => (&*m.login, m.user_id.parse::<u64>().ok(), PaintId::of_message(m)),
  };
  // asking is part of looking: seeing a chatter queues them for the next batch
  let flair = uid.filter(|_| look.sub_flair).and_then(|id| res.flairs.get(id)).map(|b| b.url.clone());
  let heat = if look.heat_bars { res.heats.get(login) } else { 0 };
  let known = uid.is_some_and(|id| res.senders.known(id));
  let mine = own.is_some() || uid.is_some_and(|id| res.senders.is_me(id));
  let badged = paint.is_some_and(|id| res.cosmetics.hs_badge(&id).is_some());
  let w = wear(look.flame, look.heat_bars, known, mine, badged, heat);
  if let Some(url) = flair {
    push_badge(res, &url, None, s, ib, boxes);
  }
  if w.flame {
    let a = s.len();
    s.push_str(FLAME);
    spans.push(Span { range: a..s.len(), ink: Ink(crate::ui::theme::WHITE), bold: false });
    ib.push(gap(s.len(), res.images.px(3)));
  }
  if let Some(bar) = w.bar {
    let a = s.len();
    s.push_str(bar.glyph);
    spans.push(Span { range: a..s.len(), ink: Ink(bar.ink), bold: false });
    ib.push(gap(s.len(), res.images.px(3)));
  }
}

// what trails a name: the plus `+` (ext .hs-plus-tenure: bold, 4px before it, 2px after), then the picked heatsync
// badge (ext .hs-badge), on a sender and on a reply's parent, never on an @name in the words. the `+` is
// colored by tenure, the badge by its catalogue entry
pub(super) fn after_name(res: &mut Res, uid: &PaintId, s: &mut String, spans: &mut Vec<Span>, ib: &mut Vec<InlineBox>) {
  if let Some(since) = res.cosmetics.plus(uid) {
    ib.push(gap(s.len(), res.images.px(4)));
    let a = s.len();
    s.push('+');
    spans.push(Span { range: a..s.len(), ink: Ink(tenure::ink(since, crate::util::time::now_ms())), bold: true });
    ib.push(gap(s.len(), res.images.px(2)));
  }
  if let Some(d) = res.cosmetics.hs_badge(uid) {
    ib.push(gap(s.len(), res.images.px(4)));
    let a = s.len();
    s.push_str(d.glyph);
    spans.push(Span { range: a..s.len(), ink: Ink(d.ink), bold: false });
    ib.push(gap(s.len(), res.images.px(2)));
  }
}
