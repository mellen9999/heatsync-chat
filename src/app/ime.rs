// the input method: whether the focused thing takes typed text, and keeping the window's state in step

use super::App;
use crate::model::picker::Focus;
use crate::model::vi::Mode;
use crate::ui::canvas::Rect;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::Ime as WinIme;

// the window's side of the input method: whether it currently lets one run, the caret rect, and the
// rect last sent
#[derive(Default)]
pub(super) struct ImeWin {
  pub(super) on: bool,
  pub(super) area: Option<Rect>,
  pub(super) sent: Option<Rect>,
}

impl App {
  // does the focused thing take typed text (so an input method may run)?
  pub(super) fn wants_text(&self) -> bool {
    if self.form.is_some() || self.screen.as_ref().is_some_and(|s| s.typing()) {
      return true;
    }
    match &self.picker {
      Some(p) => p.focus == Focus::Search,
      None => self.prompt.is_some() || self.mode == Mode::Insert,
    }
  }

  // keep the window's input-method state in step: allowed only while text is accepted,
  // candidate window parked at the caret
  pub(super) fn sync_ime(&mut self) {
    let want = self.wants_text();
    if !want && self.ime.composing() {
      self.ime.clear();
      self.input_dirty = true;
      self.wake();
    }
    let Some(g) = &self.gfx else { return };
    if want != self.ime_win.on {
      self.ime_win.on = want;
      g.window.set_ime_allowed(want);
      self.ime_win.sent = None;
    }
    if want && self.ime_win.area != self.ime_win.sent {
      self.ime_win.sent = self.ime_win.area;
      if let Some(r) = self.ime_win.area {
        g.window.set_ime_cursor_area(
          PhysicalPosition::new(r.x, r.y),
          PhysicalSize::new(r.w.max(1) as u32, r.h.max(1) as u32),
        );
      }
    }
  }

  pub(super) fn on_ime(&mut self, ev: WinIme) {
    match ev {
      WinIme::Enabled => self.ime_win.sent = None,
      WinIme::Preedit(text, cursor) => {
        if self.wants_text() {
          self.ime.set(&text, cursor);
          self.input_dirty = true;
          self.wake();
        }
      }
      WinIme::Commit(text) => {
        let text = self.ime.commit(&text);
        if self.wants_text() && !text.is_empty() {
          if let Some(f) = &mut self.form {
            f.fields[f.focus].insert(&text);
            f.err = None;
          } else if let Some(sc) = self.screen.as_mut().filter(|s| s.typing()) {
            sc.query_mut().insert(&text);
          } else if let Some(p) = &mut self.picker {
            p.type_str(&text);
            self.picker_reveal();
          } else if let Some(p) = &mut self.prompt {
            p.insert(&text);
            if self.find.on {
              self.find_live();
            }
          } else {
            self.complete = None;
            self.line.checkpoint(false);
            self.line.insert(&text);
          }
        }
        self.input_dirty = true;
        self.wake();
      }
      WinIme::Disabled => {
        self.ime.clear();
        self.input_dirty = true;
        self.wake();
      }
    }
  }
}
