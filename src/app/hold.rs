// the hover hold, wired to the pointer: see ui/chat_view/hold.rs for the rule. only a pointer that moved over
// the chat is looked at, and nothing is asked while it is anywhere else
use std::time::Instant;

use super::App;

impl App {
  // the pointer moved: is it on an emote, a name or a link in a line?
  pub(super) fn hold_pointer(&mut self) {
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    let free = self.tip_free() && self.img_view.is_none() && self.tour.is_none() && chat.contains(x, y);
    if !free {
      return self.hold_leave();
    }
    let res = &mut self.res;
    let Some(t) = self.tabs.get_mut(self.active) else { return };
    let v = &mut t.view;
    if !v.hold_moved((x, y)) {
      return;
    }
    let over = v.emote_at(chat, x, y, res).is_some()
      || v.who_at(chat, x, y, res).is_some()
      || v.link_at(chat, x, y, res).is_some();
    v.hold_over(over, Instant::now());
  }

  // the pointer left the chat or the window: a hold lets go at once and the list pins back to its tail
  pub(super) fn hold_leave(&mut self) {
    if self.tabs.get_mut(self.active).is_some_and(|t| t.view.hold_leave()) {
      self.wake();
    }
  }

  pub(super) fn hold_due(&self) -> Option<Instant> {
    self.tabs.get(self.active).and_then(|t| t.view.hold_due())
  }

  pub(super) fn hold_tick(&mut self, now: Instant) {
    if self.tabs.get_mut(self.active).is_some_and(|t| t.view.hold_tick(now)) {
      self.wake();
    }
  }
}
