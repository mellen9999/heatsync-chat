// emotes in the message box: which words show as their emote, and when a typed word turns into one. a chip
// is only a name in the line's set, the text sent is unchanged; which picture a name is comes from the emote
// law (emotes/draft.rs), with the draft's own picks
use super::App;
use crate::emotes::block::{Blocked, Scope};
use crate::emotes::draft::{Draft, Found, From};
use crate::emotes::mods::Fx;

// a stack's pictures, and the modifiers folded onto each (by layer)
type Read = (Vec<Found>, Vec<(u8, Fx)>);
use crate::model::catalog::{Rec, Registry};
use crate::model::message::Platform;
use crate::model::tabs::Kind;
use crate::ui::chat_view::{EmoteHit, Layer};
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
    let leg = self.leg();
    Draft {
      emotes: &self.res.emotes,
      senders: &self.res.senders,
      personal: &self.res.personal,
      me: self.own_id(),
      inventory: self.res.look.inventory_emotes,
      leg,
      // no room (a post, a whisper): no feeds either
      legs: if leg.is_some() { self.legs().into_iter().map(|(p, _)| p).collect() } else { Vec::new() },
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
    let base = crate::emotes::overlay_base(word);
    let hidden = |url: &str| blocked.contains(word, url) || base.is_some_and(|b| blocked.contains(b, url));
    d.word(Some(picks), word).map(|f| f.0).filter(|(url, _, _)| !hidden(url))
  }

  // a box word as the emote a click on it means: the picture drawn there (the one the word sends), filed
  // for a block the way a chat line files the same picture (emotes/resolve.rs)
  fn box_hit(&self, word: &str) -> Option<EmoteHit> {
    let d = self.draft();
    // the name the picture is filed under: `name0` read as an overlay is `name`'s emote
    let (name, (url, _, zero_width), from) = d.named(Some(&self.line.picks), word)?;
    // a picture you blocked is not drawn in the box (`drawn`), so it is not there to click
    if self.res.blocked.contains(word, &url) || self.res.blocked.contains(name, &url) {
      return None;
    }
    let owners = |p| self.res.emotes.owner_on(d.chan, p, name).is_some_and(|x| *x.url == *url);
    let scope = match from {
      From::Pick | From::Own => Scope::Hs,
      From::Room if d.inventory && d.leg.is_some_and(owners) => Scope::Hs,
      From::Room | From::Personal => Scope::Channel,
      From::Native | From::Global => Scope::Global,
    };
    Some(EmoteHit { name: name.into(), url, zero_width, scope, over: Vec::new() })
  }

  // the emote in the message box under the pointer. a stack answers as on a chat line: its base, with the
  // overlays on it
  pub(super) fn box_emote_under_pointer(&self) -> Option<EmoteHit> {
    let (x, y) = self.pointer;
    let r = self.input_hit.as_ref().filter(|_| self.input_area().contains(x, y))?.chip_at(x, y)?;
    let mut found = self.line.text.get(r)?.split(' ').filter_map(|w| self.box_hit(w));
    let mut hit = found.next()?;
    hit.over = found.map(|h| Layer { name: h.name, url: h.url, scope: h.scope }).collect();
    Some(hit)
  }

  // an emote here under this name with this picture, as a pick: tab's, a picker tile's, one clicked in chat
  pub(super) fn pick_of(&self, name: &str, url: &str) -> Rec {
    let (res, chan, me) = (&self.res, self.tabs.get(self.active).map_or("", |t| &*t.view.channel), self.own_id());
    let e = |e: &crate::emotes::Emote| (&*e.url == url).then_some(e.zero_width);
    let owner = self.leg().map_or_else(Vec::new, |p| res.emotes.owner_all(chan, p));
    let room = self.leg().map_or_else(Vec::new, |p| res.emotes.channel_all(chan, p));
    let zw = (room.iter())
      .chain(&res.emotes.global_all())
      .find_map(|(n, _, x)| (*n == name).then(|| e(x)).flatten())
      .or_else(|| owner.iter().find_map(|(n, x)| (*n == name).then(|| e(x)).flatten()))
      .or_else(|| me.and_then(|id| res.personal.peek(id, name)).and_then(e))
      .or_else(|| res.emotes.sub(name).and_then(e))
      .or_else(|| {
        me.and_then(|id| res.senders.peek(id, name, now_ms())).filter(|x| &*x.url == url).map(|x| x.zero_width)
      })
      // a favorite nothing here holds is picked from its record
      .or_else(|| res.favorites.zero_width(name, url));
    Rec { url: url.into(), zero_width: zw.unwrap_or(false), provider: crate::emotes::draft::host(url) }
  }

  // an emote of this name went into the box on purpose (the picker, a click in chat): it is this draft's
  // pick, shown as its picture
  pub(super) fn insert_pick(&mut self, name: &str, rec: Rec) {
    let url = rec.url.clone();
    self.line.picks.put(name, Some(rec));
    self.line.insert_chip(name);
    self.used_emote(name, &url);
  }

  // an emote went into the box: the picker's recents (the picture) and tab's ranking (the name) learn it
  pub(super) fn used_emote(&mut self, name: &str, url: &str) {
    self.recents.record(name, url);
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

  // the line's emote stacks as boxes: a base and the overlays on it, one box, with the modifier words folded
  // into it drawn as the change they make (`Okayge p!` is a partying Okayge, as the line will read)
  pub(super) fn input_chips(&mut self) -> Vec<Chip> {
    self.sync_overlays();
    let h = self.res.images.px(EMOTE_PX);
    let binds = self.line.mod_binds();
    // every stack's words read once, against one draft: its pictures, and each folded word on its picture
    let d = self.draft();
    let read = |r: &std::ops::Range<usize>| -> Read {
      let (mut found, mut at, mut layer_of) = (Vec::new(), r.start, Vec::new());
      for w in self.line.text[r.clone()].split(' ') {
        let wr = at..at + w.len();
        at = wr.end + 1;
        if binds.iter().any(|(m, _)| *m == wr) {
          continue;
        }
        if let Some(f) = Self::drawn(&d, &self.line.picks, &self.res.blocked, w) {
          layer_of.push((wr, found.len() as u8));
          found.push(f);
        }
      }
      let mut fx: Vec<(u8, Fx)> = Vec::new();
      for (m, to) in &binds {
        let Some(&(_, k)) = layer_of.iter().find(|(wr, _)| wr == to) else { continue };
        let i = fx.iter().position(|f| f.0 == k).unwrap_or_else(|| {
          fx.push((k, Fx::default()));
          fx.len() - 1
        });
        fx[i].1.add(&self.line.text[m.clone()]);
      }
      (found, fx)
    };
    let stacks: Vec<(std::ops::Range<usize>, Read)> =
      self.line.stacks().into_iter().map(|r| (r.clone(), read(&r))).collect();
    let mut out: Vec<Chip> = Vec::new();
    for (r, (found, fx)) in stacks {
      let words = &self.line.text[r.clone()];
      // a stack that starts on a lone emoji (not an emote): the emoji is drawn under its overlays
      let first = words.split(' ').next().unwrap_or("");
      let glyph = (!self.line.chips.contains(first) && crate::ui::text::lone_emoji(first)).then(|| first.into());
      let Some(dims) = found.first().map(|f| f.1) else { continue };
      let layers: Vec<_> = found.iter().map(|f| self.res.images.id(&f.0, EMOTE_PX)).collect();
      let w = self.res.images.width(layers[0], dims);
      // the slot is the widest, tallest layer once turned and stretched
      let (w, h) = (0..layers.len()).fold((0, 0), |(mw, mh), k| {
        let lw = if k == 0 { w } else { self.res.images.width(layers[k], found[k].1) };
        let (lw, lh) = fx.iter().find(|f| f.0 as usize == k).map_or((lw, h), |f| f.1.size((lw, h)));
        (mw.max(lw), mh.max(lh))
      });
      let w = if glyph.is_some() { w.max(h) } else { w };
      out.push(Chip { range: r, glyph, layers, w, h, fx });
    }
    out
  }

  // the picks in a line that were a choice: the ones that change what their word sends from here. a tab
  // that took the first answer is what typing the name gives, so it is not one. these come back with the
  // line when it is recalled (model/line.rs `keep`): recalled in another channel, a word you tab-completed
  // is that channel's emote, never the first one's, and a copy you picked on purpose is still that copy
  pub(super) fn chosen(&self, text: &str, picked: &Registry) -> Registry {
    let mut out = picked.clone();
    out.prune(text);
    let d = self.draft();
    out.retain(|name, rec| d.word(None, name).is_none_or(|(f, _)| *f.0 != *rec.url));
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
