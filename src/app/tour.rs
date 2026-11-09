// the first-run tour (model::tour): six cards over the chat after the first channel is added, once per install,
// /tour brings it back. enter or → next, ← back, esc skips; the last card turns on timestamps, vi and the hover
// reply. a switch goes through `flip`, the very path settings and the slash commands use
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::schema::def;
use crate::model::tour::{Go, SWITCHES, Tour, seen};
use crate::ui::canvas::Rect;
use crate::ui::tour::{self as view, Hit};

impl App {
  // the first channel is open: the tour greets a new install, once
  pub(super) fn tour_first(&mut self) {
    if self.tour.is_none() && !seen() {
      crate::model::tour::mark_seen();
      self.tour_open();
    }
  }

  // /tour: from the first card, over whatever was showing
  pub(super) fn tour_open(&mut self) {
    if self.picker.is_some() {
      self.close_picker();
    }
    self.close_menu();
    self.tour = Some(Tour::default());
    self.input_dirty = true;
    self.wake();
  }

  pub(super) fn tour_close(&mut self) {
    if self.tour.take().is_some() {
      if let Some(t) = self.tabs.get_mut(self.active) {
        t.view.invalidate();
      }
      self.input_dirty = true;
      self.wake();
    }
  }

  fn tour_go(&mut self) {
    if self.tour.as_mut().map(Tour::next) == Some(Go::Done) {
      self.tour_close();
    }
    self.wake();
  }

  // the focused switch, flipped
  fn tour_switch(&mut self, i: usize) {
    if let Some(t) = &mut self.tour {
      t.row = i;
    }
    if let Some(d) = SWITCHES.get(i).and_then(|id| def(id)) {
      self.flip(d);
    }
    self.wake();
  }

  pub(super) fn tour_key(&mut self, key: Key<&str>) {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    let Some(t) = self.tour.as_mut() else { return };
    let on_last = t.last();
    match key {
      Key::Named(NamedKey::Escape) => return self.tour_close(),
      Key::Named(NamedKey::Enter | NamedKey::ArrowRight) => return self.tour_go(),
      Key::Named(NamedKey::ArrowLeft) => t.back(),
      Key::Named(NamedKey::ArrowDown) => t.step(1),
      Key::Named(NamedKey::ArrowUp) => t.step(-1),
      Key::Named(NamedKey::Space) if on_last => {
        let i = t.row;
        return self.tour_switch(i);
      }
      Key::Character("j") if !ctrl && !alt => t.step(1),
      Key::Character("k") if !ctrl && !alt => t.step(-1),
      Key::Character("h") if !ctrl && !alt => t.back(),
      Key::Character("l") if !ctrl && !alt => return self.tour_go(),
      // anything else is not the tour's: nothing behind the card may act while it is up
      _ => return,
    }
    self.wake();
  }

  pub(super) fn tour_geo(&mut self) -> Option<view::Geo> {
    let area = self.chat_area();
    let line = self.res.text.line_px as i32;
    let em = self.res.text.layout("0", &[], &[], f32::MAX).full_width().ceil() as i32;
    Some(view::geo(area, line, em, self.tour.as_ref()?))
  }

  pub(super) fn tour_click(&mut self) {
    let (x, y) = self.pointer;
    let Some(g) = self.tour_geo() else { return };
    let Some(hit) = self.tour.as_ref().and_then(|t| view::hit(&g, t, x, y)) else { return };
    match hit {
      Hit::Next => self.tour_go(),
      Hit::Back => {
        if let Some(t) = &mut self.tour {
          t.back();
        }
        self.wake();
      }
      Hit::Skip => self.tour_close(),
      Hit::Row(i) => self.tour_switch(i),
    }
  }

  pub(super) fn draw_tour(&mut self) -> Option<Rect> {
    let area = self.chat_area();
    let t = self.tour.as_ref()?;
    let rows: Vec<(&str, bool)> =
      SWITCHES.iter().filter_map(|id| def(id)).map(|d| (d.label, d.on(&self.set).unwrap_or(false))).collect();
    view::draw(&mut self.canvas, &mut self.res.text, area, t, &rows, self.pointer);
    Some(area)
  }
}
