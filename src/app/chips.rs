// emotes in the message box (the ext's input preview): which words show as their emote, and when a
// typed word turns into one. a chip is only a name in the line's set; the text sent is unchanged
use super::App;
use crate::ui::input_bar::Chip;
use crate::ui::res::EMOTE_PX;
use crate::util::time::now_ms;

// an emote as the box resolves it: (url, native size, zero-width)
type Found = (Box<str>, (u16, u16), bool);

impl App {
  pub(super) fn own_id(&self) -> Option<u64> {
    self.me.as_ref().and_then(|m| m.twitch_id)
  }

  // your own heatsync emote of this name
  fn owned_emote(&self, word: &str) -> Option<Found> {
    let x = self.res.senders.peek(self.own_id()?, word, now_ms())?;
    Some((x.url.clone(), (x.w, x.h), x.zero_width))
  }

  // render order, like a line you sent: the channel's set, your inventory, then globals
  fn box_emote(&self, word: &str) -> Option<Found> {
    let chan = self.tabs.get(self.active).map_or("", |t| &*t.view.channel);
    let e = |e: &crate::emotes::Emote| (e.url.clone(), (e.w, e.h), e.zero_width);
    let found = self.res.emotes.channel(chan, word).map(e).or_else(|| self.owned_emote(word));
    let found =
      found.or_else(|| self.res.emotes.twitch_global(word).map(e)).or_else(|| self.res.emotes.global(word).map(e));
    found.filter(|(url, _, _)| !self.res.blocked.contains(word, url))
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

  // the line's chip words as boxes; an overlay right after an emote stacks onto it
  pub(super) fn input_chips(&mut self) -> Vec<Chip> {
    let h = self.res.images.px(EMOTE_PX);
    let mut out: Vec<Chip> = Vec::new();
    for r in self.line.chip_ranges() {
      let Some((url, dims, zw)) = self.box_emote(&self.line.text[r.clone()]) else { continue };
      let img = self.res.images.id(&url, EMOTE_PX);
      if zw
        && let Some(last) = out.last_mut()
        && last.range.end + 1 == r.start
      {
        last.layers.push(img);
        last.range.end = r.end;
        continue;
      }
      let w = self.res.images.width(img, dims);
      out.push(Chip { range: r, layers: vec![img], w, h });
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
      l.chips.contains(prev).then_some(())?;
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
