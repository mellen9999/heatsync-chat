// links in chat: a click opens it in the browser (a video plays: mpv when it is installed), `o` on a
// picked line opens its first one, the pointer turns into a hand over one. failures land in the chat.
// a tab's stream plays the same way (its menu's watch, /watch, space w)
use winit::window::{Cursor, CursorIcon};

use super::App;
use crate::model::{links, video};
use crate::net::Cmd;

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

  pub(super) fn open_link(&mut self, url: &str) {
    if video::playable(url) {
      return self.play(url);
    }
    if let Err(e) = crate::util::open::open_url(url) {
      self.say(e);
    }
  }

  pub(super) fn play(&mut self, url: &str) {
    self.net.send(Cmd::Play(url.into()));
  }

  // the stream of tab `tab`'s nth channel
  pub(super) fn watch(&mut self, tab: usize, n: usize) {
    let url = self.tabs.get(tab).and_then(|t| t.sources.get(n)).and_then(|(p, k)| video::stream(*p, k));
    match url {
      Some(u) => self.play(&u),
      None => self.say("no channel to watch here"),
    }
  }

  // space w: the channel this tab sends to
  pub(super) fn watch_here(&mut self) {
    match self.tabs.get(self.active).filter(|t| !t.sources.is_empty()).map(|t| t.target) {
      Some(n) => self.watch(self.active, n),
      None => self.say("no channel to watch here"),
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
    let (links_on, partial) = (self.set.links, self.set.partial_links);
    let url = self
      .tabs
      .get(self.active)
      .and_then(|t| t.view.selected())
      .filter(|_| links_on)
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
    } else if let Some(still) = (free && in_chat).then(|| self.pic_under_pointer()).flatten() {
      if still { CursorIcon::Pointer } else { CursorIcon::ZoomIn }
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

  // a picture under the pointer: Some(true) when it is a video's still
  fn pic_under_pointer(&mut self) -> Option<bool> {
    let ((x, y), chat) = (self.pointer, self.chat_area());
    let res = &mut self.res;
    self.tabs.get_mut(self.active)?.view.pic_at(chat, x, y, res).map(|(_, play)| play.is_some())
  }
}
