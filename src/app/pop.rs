// the one floating menu for choices that are not about an emote or a tab: the login chooser (ctrl-l)
// and, further on, the actions on a chat line. keyboard j/k/enter/esc and the mouse both drive it
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::net::Cmd;
use crate::net::auth::Provider;
use crate::ui::canvas::Rect;
use crate::ui::menu::Menu;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
  Login(Provider),
}

pub struct Pop {
  menu: Menu<Act>,
}

// the chooser rows, plain words
fn login_rows() -> Vec<(Act, String)> {
  [Provider::Twitch, Provider::Kick, Provider::Youtube]
    .into_iter()
    .map(|p| (Act::Login(p), p.name().to_owned()))
    .collect()
}

impl App {
  // ctrl-l: logged in just says who; logged out asks which platform
  pub(super) fn login(&mut self) {
    if let Some(me) = &self.me {
      let on = me.linked();
      let on = if on.is_empty() { String::new() } else { format!(" · {on}") };
      self.status = format!("logged in as {}{on} (ctrl-shift-l logs out)", me.username);
      self.bar_dirty = true;
      return self.wake();
    }
    let at = (self.canvas.w / 2 - 80, self.bar().bottom());
    self.pop = Some(Pop { menu: Menu::new(login_rows(), at).titled("log in with") });
    self.pop_changed();
  }

  pub(super) fn pop_changed(&mut self) {
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.bar_dirty = true;
    self.wake();
  }

  fn pop_pick(&mut self) {
    let Some(p) = self.pop.take() else { return };
    self.pop_changed();
    match p.menu.chosen() {
      Some(Act::Login(provider)) => {
        self.status = format!("finish logging in with {} in your browser…", provider.name());
        self.net.send(Cmd::Login(provider));
      }
      None => {}
    }
  }

  pub(super) fn pop_key(&mut self, key: Key<&str>) {
    let Some(p) = &mut self.pop else { return };
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => p.menu.step(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => p.menu.step(-1),
      Key::Named(NamedKey::Enter) => return self.pop_pick(),
      Key::Named(NamedKey::Escape) => self.pop = None,
      _ => return,
    }
    self.pop_changed();
  }

  pub(super) fn pop_click(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    let row = self.pop.as_ref().and_then(|p| p.menu.row_at(x, y, line));
    match (row, &mut self.pop) {
      (Some(i), Some(p)) => {
        p.menu.at = i;
        self.pop_pick();
      }
      _ => {
        self.pop = None;
        self.pop_changed();
      }
    }
  }

  pub(super) fn pop_hover(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    if let Some(p) = &mut self.pop
      && let Some(i) = p.menu.row_at(x, y, line)
      && i != p.menu.at
    {
      p.menu.at = i;
      self.wake();
    }
  }

  pub(super) fn draw_pop(&mut self) -> Option<Rect> {
    let bounds = Rect::new(0, 0, self.canvas.w, (self.canvas.h - self.input_h).max(0));
    let p = self.pop.as_mut()?;
    Some(p.menu.draw(&mut self.canvas, &mut self.res.text, bounds))
  }
}
