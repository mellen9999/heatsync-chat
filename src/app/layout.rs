// where things sit: the tab bar, the body under it, the chat and the input bar inside that

use super::{App, Tab};
use crate::model::settings::{Counter, Side};
use crate::ui::canvas::Rect;

impl App {
  // the tab bar: a row on top or at the bottom, a column on the left or right (ext tabPosition)
  pub(super) fn bar(&self) -> Rect {
    let (w, h) = (self.canvas.w, self.canvas.h);
    let line = self.res.text.line_px as i32;
    // a column is about 13 monospace cells (the ext's 90px), never more than half the window
    let col = ((self.res.text.font_px * 8.0) as i32).min(w / 2);
    match self.set.tab_side {
      Side::Top => Rect::new(0, 0, w, line),
      Side::Bottom => Rect::new(0, (h - line).max(0), w, line),
      Side::Left => Rect::new(0, 0, col, h),
      Side::Right => Rect::new(w - col, 0, col, h),
    }
  }

  // what the tab bar leaves: the pin, the chat and the input bar stack in here
  pub(super) fn body(&self) -> Rect {
    let (w, h, b) = (self.canvas.w, self.canvas.h, self.bar());
    match self.set.tab_side {
      Side::Top => Rect::new(0, b.h, w, h - b.h),
      Side::Bottom => Rect::new(0, 0, w, h - b.h),
      Side::Left => Rect::new(b.w, 0, w - b.w, h),
      Side::Right => Rect::new(0, 0, w - b.w, h),
    }
  }

  pub(super) fn chat_area(&self) -> Rect {
    let b = self.body();
    let top = b.y + self.pin_h + self.pred_h;
    Rect::new(b.x, top, b.w, (b.bottom() - top - self.input_h).max(0))
  }

  pub(super) fn input_area(&self) -> Rect {
    let b = self.body();
    Rect::new(b.x, (b.bottom() - self.input_h).max(b.y), b.w, self.input_h)
  }

  // the tab bar moved: everything below it re-lays out at the new width
  pub(super) fn side_to(&mut self, side: Side) {
    if side != self.set.tab_side {
      self.set.tab_side = side;
      self.set.save();
      self.relayout_body();
    }
  }

  pub(super) fn counter_to(&mut self, c: Counter) {
    if c != self.set.tab_counter {
      self.set.tab_counter = c;
      self.set.save();
      self.bar_dirty = true;
      self.wake();
    }
  }

  // the space under the tabs changed size or place: views take its width, the whole frame repaints
  pub(super) fn relayout_body(&mut self) {
    let w = self.body().w;
    for v in self.tabs.iter_mut().flat_map(Tab::views) {
      v.resize(w);
      v.invalidate();
    }
    self.canvas.fill(self.canvas.bounds(), crate::ui::theme::BLACK);
    self.pin_sig = Default::default();
    self.pred_sig = Default::default();
    self.damage_hist = [None; 2];
    self.expose = true;
    self.bar_dirty = true;
    self.input_dirty = true;
    self.wake();
  }
}
