// a picture, full size in the chat area (ext img-view.js): click one in chat or a thread and it fills
// the pane, as wide as it fits (never blown up past its own size), top first. the tab reads ← back;
// a click, esc, q or ← back closes it. over a picture the pointer is a magnifier (+), over the view
// one with a minus. keys: + / - / 0 zoom, h j k l or arrows move, the wheel scrolls
use crate::i18n::t;
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::images::ImgId;
use crate::ui::canvas::Rect;
use crate::ui::theme;

// decoded at most this tall (logical px; its own size when smaller): sharp at fit, room to zoom in
const VIEW_PX: u16 = 2048;
// zoom over fit, stepped
const ZOOMS: [f32; 7] = [1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];

pub(super) struct ImgView {
  id: ImgId,
  zoom: usize,
  // the picture's top-left offset from where fit puts it, in screen px (≤ 0: moved into it)
  pan: (i32, i32),
}

impl App {
  // a click on a picture in the chat opens it (a video's still plays the video). true when it did
  pub(super) fn img_press(&mut self) -> bool {
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    let res = &mut self.res;
    let Some((small, play)) = self.tabs.get_mut(self.active).and_then(|t| t.view.pic_at(chat, x, y, res)) else {
      return false;
    };
    if let Some(url) = play {
      self.play(&url);
      return true;
    }
    let Some(url) = self.res.images.url(small).map(Box::<str>::from) else { return false };
    let id = self.res.images.id_fit(&url, VIEW_PX, u32::from(u16::MAX));
    self.img_view = Some(ImgView { id, zoom: 0, pan: (0, 0) });
    self.takeover_changed();
    true
  }

  pub(super) fn img_close(&mut self) {
    if let Some(v) = self.img_view.take() {
      self.res.images.release(v.id);
      self.takeover_changed();
    }
  }

  // ← back, the input bar, the pane: all change when the view opens or closes
  fn takeover_changed(&mut self) {
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.bar_dirty = true;
    self.input_dirty = true;
    self.cursor_sync();
    self.wake();
  }

  // the picture's size on screen: fit to the pane's width, times the zoom
  fn img_size(&self, (w, h): (u32, u32)) -> (i32, i32) {
    let Some(v) = &self.img_view else { return (0, 0) };
    let fit = (self.chat_area().w as f32 / w.max(1) as f32).min(1.0);
    let k = fit * ZOOMS[v.zoom];
    ((w as f32 * k).round().max(1.0) as i32, (h as f32 * k).round().max(1.0) as i32)
  }

  // keep the picture covering the pane where it can: no empty strip past an edge it overhangs
  fn img_clamp(&mut self) {
    let area = self.chat_area();
    let Some(dims) = self.img_view.as_ref().and_then(|v| self.res.images.dims(v.id)) else { return };
    let (dw, dh) = self.img_size(dims);
    let Some(v) = &mut self.img_view else { return };
    let x = if dw > area.w { v.pan.0.clamp(area.w - dw - (area.w - dw) / 2, -(area.w - dw) / 2) } else { 0 };
    v.pan = (x, v.pan.1.clamp((area.h - dh).min(0), 0));
  }

  fn img_zoom(&mut self, to: usize) {
    let area = self.chat_area();
    let Some(v) = &mut self.img_view else { return };
    let to = to.min(ZOOMS.len() - 1);
    if to == v.zoom {
      return;
    }
    // the middle of the pane stays on the same spot of the picture
    let k = ZOOMS[to] / ZOOMS[v.zoom];
    let (cx, cy) = (area.w / 2, area.h / 2);
    v.pan = (((v.pan.0 - cx) as f32 * k) as i32 + cx, ((v.pan.1 - cy) as f32 * k) as i32 + cy);
    v.zoom = to;
    self.img_moved();
  }

  fn img_pan(&mut self, dx: i32, dy: i32) {
    if let Some(v) = &mut self.img_view {
      v.pan = (v.pan.0 + dx, v.pan.1 + dy);
    }
    self.img_moved();
  }

  fn img_moved(&mut self) {
    self.img_clamp();
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.wake();
  }

  pub(super) fn img_wheel(&mut self, px: i32) {
    self.img_pan(0, px);
  }

  pub(super) fn img_key(&mut self, k: Key<&str>) {
    let area = self.chat_area();
    let (sx, sy) = ((area.w / 8).max(1), (area.h / 8).max(1));
    let zoom = self.img_view.as_ref().map_or(0, |v| v.zoom);
    match k {
      Key::Named(NamedKey::Escape) | Key::Character("q") => self.img_close(),
      Key::Character("+" | "=") => self.img_zoom(zoom + 1),
      Key::Character("-") => self.img_zoom(zoom.saturating_sub(1)),
      Key::Character("0") => self.img_zoom(0),
      Key::Character("h") | Key::Named(NamedKey::ArrowLeft) => self.img_pan(sx, 0),
      Key::Character("l") | Key::Named(NamedKey::ArrowRight) => self.img_pan(-sx, 0),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => self.img_pan(0, sy),
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => self.img_pan(0, -sy),
      Key::Named(NamedKey::PageUp) => self.img_pan(0, area.h - sy),
      Key::Named(NamedKey::PageDown) | Key::Named(NamedKey::Space) => self.img_pan(0, sy - area.h),
      _ => {}
    }
  }

  // the pane: black, the picture centered across and from the top down, a dim word while it loads
  pub(super) fn draw_img_view(&mut self) -> Option<Rect> {
    let area = self.chat_area();
    let id = self.img_view.as_ref()?.id;
    self.canvas.fill(area, theme::BLACK);
    self.next_anim = None;
    if self.res.images.failed(id) {
      self.img_close();
      self.say(t("app.img_view.cant_open"));
      return Some(area);
    }
    self.img_clamp();
    let dims = self.res.images.dims(id);
    let pan = self.img_view.as_ref()?.pan;
    let size = dims.map(|d| self.img_size(d));
    match (self.res.images.frame(id), size) {
      (Some((f, w, h, next)), Some((dw, dh))) => {
        let x = area.x + (area.w - dw) / 2 + pan.0;
        self.canvas.rgba_to(x, area.y + pan.1, (w as i32, h as i32), (dw, dh), &f.rgba, area);
        self.next_anim = next.map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms.max(33) as u64));
      }
      _ => {
        let l = self.res.text.layout(t("app.img_view.loading"), &[], &[], f32::MAX);
        self.res.text.draw(&l, &mut self.canvas, area.x as f32 + theme::PAD_X, area.y as f32, area, |_, _| {});
      }
    }
    Some(area)
  }
}
