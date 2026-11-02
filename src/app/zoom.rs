// zoom: one persisted multiplier on the display scale, so text, emotes, badges, the input bar and
// the picker all grow together. ctrl-= / ctrl-+ in, ctrl-- out, ctrl-0 reset, ctrl+wheel, /zoom 1.5
use winit::keyboard::Key;

use super::App;
use crate::model::settings::{ZOOM_STEP, clamp_zoom};

// pixel-wheel travel (touchpads) that counts as one step
const WHEEL_STEP_PX: f32 = 60.0;

impl App {
  // the window scale the renderer runs at: display scale x zoom
  pub(super) fn scale(&self) -> f32 {
    self.dpi * self.set.scale()
  }

  pub(super) fn zoom_to(&mut self, pct: u16) {
    let pct = clamp_zoom(pct as i32);
    if pct != self.set.zoom {
      self.set.zoom = pct;
      self.set.save();
      self.apply_scale();
    }
    self.say(format!("zoom {}%", self.set.zoom));
  }

  pub(super) fn zoom_by(&mut self, steps: i32) {
    self.zoom_to(clamp_zoom(self.set.zoom as i32 + steps * ZOOM_STEP as i32));
  }

  // every size derives from the scale: re-lay out and repaint it all. images drop their decoded
  // frames and reload at the new height (slots key on logical height, so none are orphaned)
  pub(super) fn apply_scale(&mut self) {
    self.res.set_scale(self.scale());
    let w = self.canvas.w;
    for v in self.tabs.iter_mut().flat_map(|t| t.views()) {
      v.relayout();
      v.resize(w);
      v.invalidate();
    }
    self.damage_hist = [None; 2];
    self.bar_dirty = true;
    self.input_dirty = true;
    self.trim_soon();
    self.wake();
  }

  // ctrl-= / ctrl-+ / ctrl-- / ctrl-0 in every mode. true when handled
  pub(super) fn zoom_key(&mut self, key: &Key<&str>) -> bool {
    if !self.mods.control_key() {
      return false;
    }
    match key {
      Key::Character("=" | "+") => self.zoom_by(1),
      Key::Character("-" | "_") => self.zoom_by(-1),
      Key::Character("0") => self.zoom_to(100),
      _ => return false,
    }
    true
  }

  // ctrl+wheel; `lines` for notched wheels, `px` for smooth ones. true when it zoomed
  pub(super) fn zoom_wheel(&mut self, lines: Option<f32>, px: f32) -> bool {
    if !self.mods.control_key() {
      self.zoom_acc = 0.0;
      return false;
    }
    let steps = match lines {
      Some(l) => l.signum() as i32 * (l != 0.0) as i32,
      None => {
        self.zoom_acc += px;
        let n = (self.zoom_acc / WHEEL_STEP_PX).trunc();
        self.zoom_acc -= n * WHEEL_STEP_PX;
        n as i32
      }
    };
    if steps != 0 {
      self.zoom_by(steps);
    }
    true
  }
}
