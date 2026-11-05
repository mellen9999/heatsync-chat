// the mentions, whispers and feed tabs light up for something new and go out when you look, here or
// on any other heatsync client (ext seen-state.js bumpSeen / applySeenUpdate)
use super::App;
use crate::model::tabs::Kind;
use crate::model::unseen::Surface;
use crate::net::{Cmd, social};
use crate::util::time::now_ms;

pub(super) fn surface(k: Kind) -> Option<Surface> {
  match k {
    Kind::Mentions => Some(Surface::Mentions),
    Kind::Whispers => Some(Surface::Whispers),
    Kind::Feed => Some(Surface::Live),
    Kind::Notifs => Some(Surface::Inbox),
    _ => None,
  }
}

impl App {
  fn surface_tab(&self, s: Surface) -> Option<usize> {
    self.tabs.iter().position(|t| surface(t.kind) == Some(s))
  }

  // something new arrived: lit, or seen at once when you are looking at it
  pub(super) fn unseen_note(&mut self, s: Surface) {
    self.unseen_note_at(s, now_ms());
  }

  // the same, for something that arrived at `at`
  pub(super) fn unseen_note_at(&mut self, s: Surface, at: i64) {
    self.unseen.note(s, at);
    if self.surface_tab(s) == Some(self.active) {
      self.unseen_clear(s);
    } else {
      self.unseen_paint(s);
    }
    self.wake();
  }

  // you looked: out here, and on your other clients
  pub(super) fn unseen_clear(&mut self, s: Surface) {
    let at = now_ms();
    self.unseen.saw(s, at);
    if s == Surface::Inbox {
      // the account keeps its own unread: tell it, once, and only when something was unread
      let unread = self.notifs.unread() > 0;
      self.notifs.read(at);
      if unread && self.me.is_some() {
        self.net.send(Cmd::Social(social::Cmd::NotifsRead));
      }
      self.bar_dirty = true;
    }
    if self.me.is_some() && s.written() {
      self.net.send(Cmd::Social(social::Cmd::SeenBump { surface: s, at }));
    }
    self.unseen_paint(s);
  }

  // you looked on another client (or before this one started)
  pub(super) fn on_seen(&mut self, s: Surface, at: i64) {
    self.unseen.saw(s, at);
    if s == Surface::Inbox {
      self.notifs.read(at);
      self.bar_dirty = true;
    }
    self.unseen_paint(s);
  }

  fn unseen_paint(&mut self, s: Surface) {
    let Some(i) = self.surface_tab(s) else { return };
    let lit = self.unseen.lit(s) && i != self.active;
    if self.tabs[i].unread != lit {
      self.tabs[i].unread = lit;
      self.bar_dirty = true;
      self.wake();
    }
  }
}
