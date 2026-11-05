// the pointer: clicks, right clicks, and what it rests on in the input bar

use super::App;
use super::overlay::Overlay;
use crate::model::tabs::Kind;
use crate::model::vi::Mode;
use crate::ui::input_bar;

// which of the buttons the pointer is on: the logo and upload buttons and the "log in" link in the input
// box, then the tab bar's + gear ? and update buttons
#[derive(Default)]
pub(super) struct Hover {
  pub(super) btn: bool,
  pub(super) up: bool,
  pub(super) link: bool,
  pub(super) plus: bool,
  pub(super) gear: bool,
  pub(super) keys: bool,
  pub(super) update: bool,
}

impl App {
  // click a line to select it; click the input to type
  pub(super) fn click(&mut self) {
    self.hide_tip();
    if self.form.is_some() {
      return self.form_click();
    }
    if self.screen.is_some() {
      // the tab bar stays live: a tab closes the screen first; the gear and the ? swap or close it
      if !self.bar().contains(self.pointer.0, self.pointer.1) {
        return self.screen_click();
      }
      if self.bar_screen_click() {
        return;
      }
      self.close_screen();
    }
    match self.overlay {
      Some(Overlay::Pop(_)) => return self.pop_click(),
      Some(Overlay::Card(_)) => return self.card_click(),
      Some(Overlay::Tab(_)) => return self.tab_menu_click(),
      Some(Overlay::Emote(_)) => return self.menu_click(),
      None => {}
    }
    let (x, y) = self.pointer;
    if self.bar().contains(x, y) {
      return self.bar_click();
    }
    if self.img_view.is_some() {
      // the picture is its own way out
      return self.img_close();
    }
    if self.suggest_click() {
      return;
    }
    if self.pin_click() || self.pred_click() {
      return;
    }
    let (chat, input) = (self.chat_area(), self.input_area());
    if self.over_button() {
      return self.toggle_picker();
    }
    if self.over_upload() {
      return self.pick_file();
    }
    if self.over_login() {
      return self.login();
    }
    if self.picker.is_some() {
      return self.picker_click();
    }
    if self.jump_click() {
      return;
    }
    if input.contains(x, y) {
      // the cursor goes where you click, like any text box
      if let Some(i) = self.input_hit.as_ref().map(|h| h.byte_at(x, y)) {
        self.complete = None;
        self.line.seal();
        self.line.cursor = i.min(self.line.text.len());
        self.input_dirty = true;
      }
      return self.enter_insert();
    }
    if !chat.contains(x, y) {
      return;
    }
    if self.vote_press()
      || self.held_press()
      || self.img_press()
      || self.reveal_press()
      || self.thread_press()
      || self.link_press()
      || self.tag_press()
      || self.emote_press(false)
      || self.name_press()
      || self.reply_press()
    {
      return;
    }
    let res = &mut self.res;
    let picked = self.tabs.get_mut(self.active).is_some_and(|t| t.view.select_at(chat, y, res));
    // a click on nothing, outside the lines of an open reply thread, goes back
    if !picked && self.chain_shown() {
      self.chain_close();
      return self.wake();
    }
    if picked && self.tabs[self.active].kind == Kind::Mentions {
      self.jump_to_mention();
    } else if picked && self.set.vi && self.mode == Mode::Insert {
      // a picked line's keys (r, m, c) are normal mode's
      self.leave_insert();
    }
    self.wake();
  }

  // the pointer moved: the box's buttons light up while it is on them
  pub(super) fn input_hover(&mut self) {
    let (over, up, link) = (self.over_button(), self.over_upload(), self.over_login());
    if over != self.hover.btn || up != self.hover.up || link != self.hover.link {
      (self.hover.btn, self.hover.up, self.hover.link) = (over, up, link);
      self.input_dirty = true;
      self.wake();
    }
  }

  // the pointer is on the empty box's "ctrl-l log in"
  pub(super) fn over_login(&self) -> bool {
    let (x, y) = self.pointer;
    self.input_h > 0 && self.login_link.is_some_and(|r| r.contains(x, y))
  }

  pub(super) fn over_button(&self) -> bool {
    let (x, y) = self.pointer;
    self.input_h > 0 && input_bar::button(self.input_area(), self.res.text.line_px as i32).contains(x, y)
  }

  pub(super) fn right_click(&mut self) {
    if self.picker.is_some() || self.form.is_some() {
      return;
    }
    if self.screen.is_some() {
      return self.close_screen();
    }
    if self.pop().is_some() {
      self.close_pop();
      return self.pop_changed();
    }
    if self.card().is_some() {
      self.close_card();
      return self.pop_changed();
    }
    if self.tab_menu().is_some() {
      self.close_tab_menu();
      return self.tab_menu_changed();
    }
    if self.bar().contains(self.pointer.0, self.pointer.1) {
      return self.bar_right_click();
    }
    if self.menu().is_some() {
      self.close_menu();
      return self.menu_changed();
    }
    if !self.emote_press(true) {
      self.line_right_click();
    }
  }
}
