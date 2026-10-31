// the emote hover popup: the pointer resting on an emote (in chat or the picker) opens it, and it
// closes when the pointer leaves, the chat scrolls or a key goes down. hover is only looked at when
// the pointer moves, so a still pointer costs nothing; only an animated emote ticks its frames
use std::time::{Duration, Instant};

use super::App;
use crate::emotes::block::Scope;
use crate::model::picker::Sec;
use crate::ui::canvas::Rect;
use crate::ui::tip::Tip;

// repaint ceiling for an animated emote, same as chat
const ANIM_MIN_MS: u32 = 33;

// an emote under the pointer: name, url, in your own set, from a heatsync inventory
type Cand = (Box<str>, Box<str>, bool, bool);

impl App {
  fn tip_free(&self) -> bool {
    self.form.is_none()
      && self.screen.is_none()
      && self.pop.is_none()
      && self.card.is_none()
      && self.tab_menu.is_none()
      && self.menu.is_none()
  }

  fn tip_target(&mut self) -> Option<Cand> {
    if !self.tip_free() {
      return None;
    }
    let (x, y) = self.pointer;
    if self.picker.is_some() {
      let g = self.picker_geo();
      let p = self.picker.as_ref()?;
      let it =
        g.grid.contains(x, y).then(|| p.cell_at(x - g.grid.x, y - g.grid.y, g.m)).flatten().and_then(|i| p.item(i))?;
      let mine = it.sec == Sec::Mine;
      return Some((it.name.clone(), it.url.clone(), mine, mine));
    }
    let hit = self.emote_under_pointer()?;
    let owned = self.owned(&hit);
    Some((hit.name, hit.url, owned, hit.scope == Scope::Hs))
  }

  // the pointer moved: open, keep or close the popup
  pub(super) fn tip_hover(&mut self) {
    let want = self.tip_target();
    match (want, &self.tip) {
      (None, None) => {}
      (None, Some(_)) => self.hide_tip(),
      (Some((n, u, ..)), Some(t)) if t.is(&n, &u) => {}
      (Some((n, u, owned, hs)), _) => {
        self.hide_tip();
        let blocked = self.res.blocked.contains(&n, &u);
        self.tip = Some(Tip::new(&n, &u, owned, hs, blocked, self.pointer));
        self.tip_repaint();
      }
    }
  }

  // whatever sat under the popup repaints (the chat redraws whole while it is up; the picker once)
  fn tip_repaint(&mut self) {
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    if let Some(p) = &mut self.picker {
      p.view.forget();
    }
    self.wake();
  }

  pub(super) fn hide_tip(&mut self) {
    if let Some(mut t) = self.tip.take() {
      t.release(&mut self.res);
      self.tip_next = None;
      self.tip_repaint();
    }
  }

  // painted last, over everything. also asks for the big picture the first time
  pub(super) fn draw_tip(&mut self) -> Option<Rect> {
    let bounds = self.chat_area();
    let t = self.tip.as_mut()?;
    let (r, ms) = t.draw(&mut self.canvas, &mut self.res, bounds)?;
    self.tip_next = ms.map(|ms| Instant::now() + Duration::from_millis(ms.max(ANIM_MIN_MS) as u64));
    Some(r)
  }
}
