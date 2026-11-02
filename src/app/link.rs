// links in chat: a click opens it in the browser, `o` on a picked line opens its first one,
// the pointer turns into a hand over one. failures land on the status line
use winit::window::{Cursor, CursorIcon};

use super::App;
use crate::model::links;

impl App {
  fn link_under_pointer(&mut self) -> Option<String> {
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    if !chat.contains(x, y) {
      return None;
    }
    let res = &mut self.res;
    self.tabs.get_mut(self.active)?.view.link_at(chat, x, y, res)
  }

  fn open_link(&mut self, url: &str) {
    if let Err(e) = crate::util::open::open_url(url) {
      self.say(e);
    }
  }

  // true when the press landed on a link and was handled
  pub(super) fn link_press(&mut self) -> bool {
    let Some(url) = self.link_under_pointer() else { return false };
    let (chat, y) = (self.chat_area(), self.pointer.1);
    let res = &mut self.res;
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.select_at(chat, y, res);
    }
    self.open_link(&url);
    self.wake();
    true
  }

  // `o` on a picked line
  pub(super) fn open_selected_link(&mut self) {
    let partial = self.set.partial_links;
    let url = self
      .tabs
      .get(self.active)
      .and_then(|t| t.view.selected())
      .and_then(|m| links::find(&m.body, partial).into_iter().next())
      .map(|l| l.url);
    match url {
      Some(u) => self.open_link(&u),
      None => {
        self.say("no link on that line");
        self.bar_dirty = true;
        self.wake();
      }
    }
  }

  // the pointer's look (only set when it changes): a magnifier with a minus over the open picture, one
  // with a plus over a picture in chat (ext zoom-out / zoom-in), a hand over a link, else the arrow
  pub(super) fn cursor_sync(&mut self) {
    let free = self.form.is_none()
      && self.screen.is_none()
      && self.pop.is_none()
      && self.card.is_none()
      && self.tab_menu.is_none()
      && self.menu.is_none()
      && self.picker.is_none();
    let (x, y) = self.pointer;
    let in_chat = self.chat_area().contains(x, y);
    let icon = if self.img_view.is_some() && in_chat {
      CursorIcon::ZoomOut
    } else if free && in_chat && self.pic_under_pointer() {
      CursorIcon::ZoomIn
    } else if free && self.link_under_pointer().is_some() {
      CursorIcon::Pointer
    } else {
      CursorIcon::Default
    };
    if icon != self.cursor {
      self.cursor = icon;
      if let Some(g) = &self.gfx {
        g.window.set_cursor(Cursor::Icon(icon));
      }
    }
  }

  fn pic_under_pointer(&mut self) -> bool {
    let ((x, y), chat) = (self.pointer, self.chat_area());
    let res = &mut self.res;
    self.tabs.get_mut(self.active).is_some_and(|t| t.view.pic_at(chat, x, y, res).is_some())
  }
}
