// the pinned-line bar under the tabs (ext pin-bar.js): newest unhidden pin of the active tab's feeds.
// click opens/folds it, the × (or x in chat mode) hides that pin for this session
use super::App;
use crate::model::message::Platform;
use crate::model::pin::Pin;
use crate::ui::canvas::Rect;
use crate::ui::pin_bar;
use crate::ui::theme;

impl App {
  pub(super) fn pin_area(&self) -> Rect {
    Rect::new(0, self.bar().h, self.canvas.w, self.pin_h)
  }

  fn cur_pin(&self) -> Option<&Pin> {
    self.pins.current(&self.tabs.get(self.active)?.sources)
  }

  pub(super) fn on_pin(&mut self, p: Platform, key: &str, pin: Option<Pin>) {
    self.pins.set(p, key, pin);
    if self.tabs.get(self.active).is_some_and(|t| t.sources.iter().any(|s| s.0 == p && *s.1 == *key)) {
      self.wake();
    }
  }

  // repaint the bar if what it shows (or the room it gets) changed; the chat below re-lays out
  pub(super) fn sync_pin(&mut self) -> Option<Rect> {
    let pin = self.cur_pin().cloned();
    let sig = (pin.as_ref().map(|p| p.id.clone()), self.pin_open, self.canvas.w, self.res.text.line_px as i32);
    if sig == self.pin_sig {
      return None;
    }
    self.pin_sig = sig;
    let old = self.pin_h;
    self.pin_h = pin.as_ref().map_or(0, |p| pin_bar::height(&mut self.res.text, p, self.canvas.w, self.pin_open));
    let area = Rect::new(0, self.bar().h, self.canvas.w, self.pin_h.max(old));
    self.canvas.fill(area, theme::BLACK);
    let bar = self.pin_area();
    if let Some(p) = &pin {
      pin_bar::draw(&mut self.canvas, &mut self.res.text, bar, p, self.pin_open);
    }
    if self.pin_h != old
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
    if self.pin_h == 0 || !area.contains(x, y) {
      return false;
    }
    if pin_bar::hide_rect(&mut self.res.text, area).contains(x, y) {
      self.pin_hide();
    } else {
      self.pin_open = !self.pin_open;
      self.wake();
    }
    true
  }

  pub(super) fn pin_hide(&mut self) {
    if let Some(id) = self.cur_pin().map(|p| p.id.clone()) {
      self.pins.hide(&id);
      self.pin_open = false;
      self.wake();
    }
  }
}
