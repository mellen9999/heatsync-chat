// the floating menus: a pop (login, line actions, confirms), the user card, the shift+right-click menu on
// an emote, and the menu on a right-clicked tab. one is open at a time: every way to open one comes from
// a click or key that the dispatch hands to the open one first (see `on_key`, `click`, `right_click`), so
// a second can never be asked for while one shows. one slot says so, instead of four options that could
// disagree.
//
// the full-window layers (form, screen, picture, picker, prompt) and tab mode are NOT in here: a few
// of them do open over a menu or each other (ctrl-; over a pop, a tab-bar click over a picture), so
// they keep their own fields and `on_key`'s order decides who answers first
use super::App;
use super::card::Card;
use super::emote::Open;
use super::pop::Pop;
use super::tabs::TabMenu;

pub(super) enum Overlay {
  Pop(Pop),
  Card(Card),
  Emote(Open),
  Tab(TabMenu),
}

impl App {
  pub(super) fn pop(&self) -> Option<&Pop> {
    match &self.overlay {
      Some(Overlay::Pop(p)) => Some(p),
      _ => None,
    }
  }

  pub(super) fn card(&self) -> Option<&Card> {
    match &self.overlay {
      Some(Overlay::Card(c)) => Some(c),
      _ => None,
    }
  }

  // the menu on an emote
  pub(super) fn menu(&self) -> Option<&Open> {
    match &self.overlay {
      Some(Overlay::Emote(o)) => Some(o),
      _ => None,
    }
  }

  pub(super) fn tab_menu(&self) -> Option<&TabMenu> {
    match &self.overlay {
      Some(Overlay::Tab(m)) => Some(m),
      _ => None,
    }
  }

  pub(super) fn card_mut(&mut self) -> Option<&mut Card> {
    match &mut self.overlay {
      Some(Overlay::Card(x)) => Some(x),
      _ => None,
    }
  }

  // each close drops the overlay only if it is that one
  pub(super) fn close_pop(&mut self) {
    if self.pop().is_some() {
      self.overlay = None;
    }
  }

  pub(super) fn close_card(&mut self) {
    if self.card().is_some() {
      self.overlay = None;
    }
  }

  pub(super) fn close_menu(&mut self) {
    if self.menu().is_some() {
      self.overlay = None;
    }
  }

  pub(super) fn close_tab_menu(&mut self) {
    if self.tab_menu().is_some() {
      self.overlay = None;
    }
  }

  pub(super) fn take_pop(&mut self) -> Option<Pop> {
    match self.overlay.take() {
      Some(Overlay::Pop(p)) => Some(p),
      other => {
        self.overlay = other;
        None
      }
    }
  }

  pub(super) fn take_card(&mut self) -> Option<Card> {
    match self.overlay.take() {
      Some(Overlay::Card(c)) => Some(c),
      other => {
        self.overlay = other;
        None
      }
    }
  }

  pub(super) fn take_menu(&mut self) -> Option<Open> {
    match self.overlay.take() {
      Some(Overlay::Emote(o)) => Some(o),
      other => {
        self.overlay = other;
        None
      }
    }
  }

  pub(super) fn take_tab_menu(&mut self) -> Option<TabMenu> {
    match self.overlay.take() {
      Some(Overlay::Tab(m)) => Some(m),
      other => {
        self.overlay = other;
        None
      }
    }
  }
}
