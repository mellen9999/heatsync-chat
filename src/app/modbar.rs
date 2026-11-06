// the mod toolbar (ext mod-toolbar.js): while the pointer rests on a line of someone's in a room we
// moderate, the picked buttons (/modbuttons) sit at that line's top right; a click runs the same action
// the line menu offers (mod_run: confirm-before-ban and reasons included). like the jump chip it repaints
// only when its place or hover changes, or when something painted over it. nothing is built while no
// button is picked
use super::App;
use crate::model::message::flag;
use crate::model::modbar::{self, CATALOG};
use crate::model::tabs::Kind;
use crate::ui::canvas::Rect;
use crate::ui::mod_bar::{self, Bar};

// what is on screen now: the strip, and the cell the pointer is on
pub(super) type Shown = (Bar, Option<usize>);

#[derive(Default)]
pub(super) struct ModBar {
  pub(super) shown: Option<Shown>,
  // the pointer left the window: its last place means nothing
  away: bool,
}

impl App {
  // the strip as it should look this frame, with the seq of the line it belongs to
  fn modbar_want(&mut self) -> Option<(Shown, i64)> {
    if self.set.mod_buttons == 0
      || self.modbar.away
      || self.me.is_none()
      || self.overlay.is_some()
      || self.form.is_some()
      || self.screen.is_some()
      || self.picker.is_some()
      || self.img_view.is_some()
    {
      return None;
    }
    let (x, y) = self.pointer;
    let area = self.chat_area();
    if !area.contains(x, y) || self.jump_covers(x, y) || self.logs_shown().is_some() {
      return None;
    }
    let t = self.tabs.get_mut(self.active).filter(|t| matches!(t.kind, Kind::Chat | Kind::Mentions))?;
    let (top, _, seq) = t.view.line_at(area, y, &mut self.res)?;
    let t = self.tabs.get(self.active)?;
    let m = t.view.message(seq)?;
    // someone's line, not ours (the ext skips own lines), in a room we moderate
    if !m.kind.said() || m.login.is_empty() || m.has(flag::OWN) {
      return None;
    }
    let chan = self.room_of(m)?;
    if !self.can_mod(m.platform, &chan) || self.me.as_ref()?.names.iter().any(|n| n.eq_ignore_ascii_case(&m.login)) {
      return None;
    }
    let mask = modbar::visible(self.set.mod_buttons, m.platform, !m.id.is_empty());
    let bar = mod_bar::place(&mut self.res.text, area, top, mask)?;
    Some(((bar, bar.at(x, y)), seq))
  }

  // `full`: the chat was just repainted whole (the strip with it); `anim`: an emote animation repainted
  // this rect. returns what changed on the canvas
  pub(super) fn sync_modbar(&mut self, full: bool, anim: Option<Rect>) -> Option<Rect> {
    let new = self.modbar_want().map(|w| w.0);
    if full {
      self.modbar.shown = None;
    }
    let old = self.modbar.shown;
    let hit = old.zip(anim).is_some_and(|((b, _), a)| !b.rect.intersect(a).is_empty());
    if new == old && !hit {
      return None;
    }
    let mut dirty = None;
    let area = self.chat_area();
    // the chat under a strip that moved or went comes back
    if let Some((b, _)) = old.filter(|o| new.is_none_or(|n| n.0 != o.0))
      && let Some(t) = self.tabs.get_mut(self.active)
    {
      t.view.draw(&mut self.canvas, &mut self.res, area, b.rect);
      dirty = Some(b.rect);
      self.jump_forget_under(b.rect);
    }
    if let Some((b, hover)) = new {
      mod_bar::draw(&mut self.canvas, &mut self.res.text, &b, hover);
      dirty = Some(dirty.map_or(b.rect, |d| crate::ui::union(d, b.rect)));
    }
    self.modbar.shown = new;
    dirty
  }

  // the chat scrolled up by `shift` under a shown strip (a new line at the bottom): its pixels went with
  // it. the strip is forgotten and the chat repainted where its ghost is
  pub(super) fn modbar_shifted(&mut self, shift: i32) -> Option<Rect> {
    let (b, _) = self.modbar.shown.take()?;
    let ghost = Rect::new(b.rect.x, b.rect.y - shift, b.rect.w, b.rect.h).intersect(self.chat_area());
    (!ghost.is_empty()).then_some(ghost)
  }

  // a screen or picker covers the chat: the strip is gone with it
  pub(super) fn modbar_forget(&mut self) {
    self.modbar.shown = None;
  }

  pub(super) fn modbar_leave(&mut self) {
    if !self.modbar.away {
      self.modbar.away = true;
      self.wake();
    }
  }

  // the pointer moved: the strip follows it to another line, lights a cell, or goes
  pub(super) fn modbar_hover(&mut self) {
    self.modbar.away = false;
    if self.set.mod_buttons == 0 && self.modbar.shown.is_none() {
      return;
    }
    if self.modbar_want().map(|w| w.0) != self.modbar.shown {
      self.wake();
    }
  }

  // true when the click landed on the strip (a gap or edge eats it too, so the line's own click stays out)
  pub(super) fn modbar_click(&mut self) -> bool {
    let (x, y) = self.pointer;
    let Some((bar, _)) = self.modbar.shown.filter(|(b, _)| b.rect.contains(x, y)) else { return false };
    let Some(cell) = bar.at(x, y) else { return true };
    let Some(((cur, _), seq)) = self.modbar_want() else { return true };
    // the strip on screen is the one the pointer is on (a stale one is not acted on)
    if cur != bar {
      return true;
    }
    let verb = CATALOG[bar.idx[cell] as usize].verb;
    let who = self.tabs.get(self.active).and_then(|t| t.view.message(seq)).and_then(|m| self.subject_of(m));
    if let Some(who) = who {
      self.mod_run(verb, vec![who], None);
    }
    self.wake();
    true
  }
}
