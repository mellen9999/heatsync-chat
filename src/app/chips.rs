// emotes in the message box: which words show as their emote, and when a typed word turns into one. a chip
// is only a name in the line's set, the text sent is unchanged; which picture a name is comes from the emote
// law (emotes/draft.rs), with the draft's own picks
use super::App;
use crate::emotes::block::Blocked;
use crate::emotes::draft::{Draft, Found};
use crate::model::catalog::{Rec, Registry};
use crate::model::message::Platform;
use crate::model::tabs::Kind;
use crate::ui::input_bar::Chip;
use crate::ui::res::EMOTE_PX;
use crate::util::time::now_ms;

impl App {
  pub(super) fn own_id(&self) -> Option<u64> {
    self.me.as_ref().and_then(|m| m.twitch_id)
  }

  // your own heatsync emote of this name
  fn owned_emote(&self, word: &str) -> Option<Found> {
    let x = self.res.senders.peek(self.res.look.inventory_emotes.then(|| self.own_id()).flatten()?, word, now_ms())?;
    Some((x.url.clone(), (x.w, x.h), x.zero_width))
  }

  // the platform a line written now resolves against (the emote law: of the feeds it goes to, twitch, else
  // kick, else youtube). None where the box writes no chat line: a post, a reply, a whisper
  pub(super) fn leg(&self) -> Option<Platform> {
    if self.feedish() || self.edit.is_some() || matches!(self.kind(), Kind::Whispers | Kind::Mentions) {
      return None;
    }
    let legs = self.legs();
    let first = [Platform::Twitch, Platform::Kick, Platform::Youtube];
    first.into_iter().find(|p| legs.iter().any(|(l, _)| l == p)).or_else(|| self.target().map(|t| t.0))
  }

  // what the box's words are read against: the room this line goes to, and what is yours
  pub(super) fn draft(&self) -> Draft<'_> {
    Draft {
      emotes: &self.res.emotes,
      senders: &self.res.senders,
      personal: &self.res.personal,
      me: self.own_id(),
      inventory: self.res.look.inventory_emotes,
      leg: self.leg(),
      chan: self.tabs.get(self.active).map_or("", |t| &*t.view.channel),
      now: now_ms(),
    }
  }

  // what the name will be once sent, by the emote law's send order (emotes/draft.rs): the chip is what
  // ships, and what the line draws when it comes back. a picture you blocked is not shown
  fn box_emote(&self, word: &str) -> Option<Found> {
    Self::drawn(&self.draft(), &self.line.picks, &self.res.blocked, word)
  }

  fn drawn(d: &Draft, picks: &Registry, blocked: &Blocked, word: &str) -> Option<Found> {
    d.word(Some(picks), word).map(|f| f.0).filter(|(url, _, _)| !blocked.contains(word, url))
  }

  // an emote here under this name with this picture, as a pick: tab's, a picker tile's, one clicked in chat
  pub(super) fn pick_of(&self, name: &str, url: &str) -> Rec {
    let (res, chan, me) = (&self.res, self.tabs.get(self.active).map_or("", |t| &*t.view.channel), self.own_id());
    let e = |e: &crate::emotes::Emote| (&*e.url == url).then_some(e.zero_width);
    let owner = self.leg().map_or_else(Vec::new, |p| res.emotes.owner_all(chan, p));
    let zw = (res.emotes.channel_all(chan).iter())
      .chain(&res.emotes.global_all())
      .find_map(|(n, _, x)| (*n == name).then(|| e(x)).flatten())
      .or_else(|| owner.iter().find_map(|(n, x)| (*n == name).then(|| e(x)).flatten()))
      .or_else(|| me.and_then(|id| res.personal.peek(id, name)).and_then(e))
      .or_else(|| res.emotes.sub(name).and_then(e))
      .or_else(|| {
        me.and_then(|id| res.senders.peek(id, name, now_ms())).filter(|x| &*x.url == url).map(|x| x.zero_width)
      });
    Rec { url: url.into(), zero_width: zw.unwrap_or(false), provider: crate::emotes::draft::host(url) }
  }

  // an emote of this name went into the box on purpose (the picker, a click in chat): it is this draft's
  // pick, shown as its picture
  pub(super) fn insert_pick(&mut self, name: &str, rec: Rec) {
    self.line.picks.put(name, Some(rec));
    self.line.insert_chip(name);
    self.used_emote(name);
  }

  // an emote went into the box: the picker's recents and tab's ranking both learn it
  pub(super) fn used_emote(&mut self, name: &str) {
    self.recents.record(name);
    self.frecency.record(name, now_ms());
  }

  // the picture an emote name shows in the box
  pub(super) fn box_url(&self, word: &str) -> Option<Box<str>> {
    self.box_emote(word).map(|f| f.0)
  }

  // which chips are overlays, so the line knows its stacks (drawing, and vi stepping over them whole)
  pub(super) fn sync_overlays(&mut self) {
    let d = self.draft();
    let over = |w: &str| Self::drawn(&d, &self.line.picks, &self.res.blocked, w).is_some_and(|f| f.2);
    let ov = self.line.chips.iter().filter(|w| over(w)).cloned().collect();
    self.line.overlays = ov;
  }

  // the line's emote stacks as boxes: a base and the overlays on it, one box
  pub(super) fn input_chips(&mut self) -> Vec<Chip> {
    self.sync_overlays();
    let h = self.res.images.px(EMOTE_PX);
    // every stack's words read once, against one draft
    let d = self.draft();
    let read = |r: &std::ops::Range<usize>| -> Vec<Found> {
      let words = self.line.text[r.clone()].split(' ');
      words.filter_map(|w| Self::drawn(&d, &self.line.picks, &self.res.blocked, w)).collect()
    };
    let stacks: Vec<(std::ops::Range<usize>, Vec<Found>)> =
      self.line.stacks().into_iter().map(|r| (r.clone(), read(&r))).collect();
    let mut out: Vec<Chip> = Vec::new();
    for (r, found) in stacks {
      let words = &self.line.text[r.clone()];
      // a stack that starts on a lone emoji (not an emote): the emoji is drawn under its overlays
      let first = words.split(' ').next().unwrap_or("");
      let glyph = (!self.line.chips.contains(first) && crate::ui::text::lone_emoji(first)).then(|| first.into());
      let Some(dims) = found.first().map(|f| f.1) else { continue };
      let layers: Vec<_> = found.iter().map(|f| self.res.images.id(&f.0, EMOTE_PX)).collect();
      let w = self.res.images.width(layers[0], dims);
      let w = if glyph.is_some() { w.max(h) } else { w };
      out.push(Chip { range: r, glyph, layers, w, h });
    }
    out
  }

  // a space was just typed: the word before it turns into its emote when it's yours, or when it's an
  // overlay landing on an emote (other names stay words, so "what" never turns into a picture)
  pub(super) fn chip_spaced(&mut self) {
    let l = &self.line;
    let Some(head) = l.text[..l.cursor].strip_suffix(' ') else { return };
    let a = head.rfind(' ').map_or(0, |i| i + 1);
    let word = &head[a..];
    if word.is_empty() || l.chips.contains(word) {
      return;
    }
    let owned = self.owned_emote(word).filter(|(url, _, _)| !self.res.blocked.contains(word, url));
    let on_base = || {
      let before = head[..a].strip_suffix(' ')?;
      let prev = &before[before.rfind(' ').map_or(0, |i| i + 1)..];
      (l.chips.contains(prev) || crate::ui::text::lone_emoji(prev)).then_some(())?;
      self.box_emote(word).filter(|f| f.2)
    };
    if owned.is_some() || on_base().is_some() {
      let w = word.into();
      self.line.chips.insert(w);
    }
  }

  // a recalled message shows every emote it names, as it looked when sent
  pub(super) fn chip_recalled(&mut self) {
    let words: Vec<Box<str>> =
      self.line.text.split(' ').filter(|w| !w.is_empty() && self.box_emote(w).is_some()).map(Into::into).collect();
    self.line.chips.extend(words);
  }
}
