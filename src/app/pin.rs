// the pinned-line bar under the tabs (ext pin-bar.js): newest unhidden pin of the active tab's feeds.
// click opens/folds it, the × (or space x in vi normal mode) hides that pin for this session
use super::App;
use crate::model::message::Platform;
use crate::model::pin::{Pin, Pins};
use crate::ui::canvas::Rect;
use crate::ui::{pin_bar, theme};

// the pinned-lines bar: the pins per room, its height (0 = none), whether it is open, and what it last drew
#[derive(Default)]
pub(super) struct PinBar {
  pub(super) pins: Pins,
  pub(super) h: i32,
  pub(super) open: bool,
  pub(super) sig: (Option<Box<str>>, bool, i32, i32),
}

impl App {
  pub(super) fn pin_area(&self) -> Rect {
    let b = self.body();
    Rect::new(b.x, b.y, b.w, self.pin_bar.h)
  }

  fn cur_pin(&self) -> Option<&Pin> {
    self.pin_bar.pins.current(&self.tabs.get(self.active)?.sources)
  }

  pub(super) fn on_pin(&mut self, p: Platform, key: &str, pin: Option<Pin>) {
    // pin frames are broadcast for every room: only rooms we are in may take a slot
    if !self.tabs.iter().any(|t| t.sources.iter().any(|s| s.0 == p && *s.1 == *key)) {
      return;
    }
    self.pin_bar.pins.set(p, key, pin);
    if self.tabs.get(self.active).is_some_and(|t| t.sources.iter().any(|s| s.0 == p && *s.1 == *key)) {
      self.wake();
    }
  }

  // repaint the bar if what it shows (or the room it gets) changed; the chat below re-lays out
  pub(super) fn sync_pin(&mut self) -> Option<Rect> {
    let pin = self.cur_pin().cloned();
    let sig = (pin.as_ref().map(|p| p.id.clone()), self.pin_bar.open, self.body().w, self.res.text.line_px as i32);
    if sig == self.pin_bar.sig {
      return None;
    }
    self.pin_bar.sig = sig;
    let old = self.pin_bar.h;
    let w = self.body().w;
    self.pin_bar.h = pin.as_ref().map_or(0, |p| pin_bar::height(&mut self.res.text, p, w, self.pin_bar.open));
    let area = Rect { h: self.pin_bar.h.max(old), ..self.pin_area() };
    self.canvas.fill(area, theme::BLACK);
    let bar = self.pin_area();
    if let Some(p) = &pin {
      pin_bar::draw(&mut self.canvas, &mut self.res.text, bar, p, self.pin_bar.open);
    }
    if self.pin_bar.h != old
      && let Some(t) = self.tabs.get_mut(self.active)
    {
      t.view.invalidate();
    }
    Some(area)
  }

  // true when the click landed on the bar
  pub(super) fn pin_click(&mut self) -> bool {
    let (x, y) = self.pointer;
    let area = self.pin_area();
    if self.pin_bar.h == 0 || !area.contains(x, y) {
      return false;
    }
    if pin_bar::hide_rect(&mut self.res.text, area).contains(x, y) {
      self.pin_hide();
    } else {
      self.pin_bar.open = !self.pin_bar.open;
      self.wake();
    }
    true
  }

  pub(super) fn pin_hide(&mut self) {
    if let Some(id) = self.cur_pin().map(|p| p.id.clone()) {
      self.pin_bar.pins.hide(&id);
      self.pin_bar.open = false;
      self.wake();
    }
  }
}
