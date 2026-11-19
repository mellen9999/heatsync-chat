// the hover hold (ext main-overlay.js): while the pointer rests on an emote, a name or a link in a line, a
// list that follows its newest line stops sliding, so the thing under the pointer stays under it. lines
// keep arriving and show the moment the pointer lets go, when the list pins back to its tail.
//
// what starts it: the pointer MOVES onto such a thing while the list is at its tail. a resting pointer never
// does (a line sliding an emote under it, or a window that gets the pointer back, is not a move), and the
// first move after the pointer arrives only says where it is. what ends it: the pointer off every such
// thing for a beat (so sweeping across emotes does not stutter), leaving the chat or the window at once, or
// any real scroll (then it is a plain pause). zero cost while the pointer is off the chat: nothing is asked
use super::ChatView;
use std::time::{Duration, Instant};

// off every held thing this long and the hold lets go
const LET_GO: Duration = Duration::from_millis(120);

#[derive(Default)]
pub struct Hold {
  held: bool,
  last: Option<(i32, i32)>,
  release: Option<Instant>,
}

impl Hold {
  pub fn held(&self) -> bool {
    self.held
  }

  // when the pending let-go falls due
  pub fn due(&self) -> Option<Instant> {
    self.release
  }

  // the pointer is at `pos`: true when it moved (after the first sighting), the only time the thing under
  // it is worth looking up
  pub fn moved(&mut self, pos: (i32, i32)) -> bool {
    let arrived = self.last.is_none();
    let moved = self.last != Some(pos);
    self.last = Some(pos);
    moved && !arrived
  }

  // what the moved pointer is on: a held thing (`over`) starts a hold at the tail and cancels a pending
  // let-go; anything else arms the let-go. true = a hold started
  fn on(&mut self, over: bool, tail: bool, now: Instant) -> bool {
    if over {
      self.release = None;
      if self.held || !tail {
        return false;
      }
      self.held = true;
      return true;
    }
    if self.held && self.release.is_none() {
      self.release = Some(now + LET_GO);
    }
    false
  }

  // forget the pointer (it left the chat or the window); true when a hold was on
  fn leave(&mut self) -> bool {
    self.last = None;
    self.release = None;
    std::mem::take(&mut self.held)
  }

  // a real scroll took over: no hold, no pending let-go
  fn cancel(&mut self) {
    self.held = false;
    self.release = None;
  }
}

impl ChatView {
  #[cfg(test)]
  pub fn held(&self) -> bool {
    self.hold.held()
  }

  pub fn hold_due(&self) -> Option<Instant> {
    self.hold.due()
  }

  // the pointer is at `pos` over the chat: has it moved (so the caller looks up what is under it)?
  pub fn hold_moved(&mut self, pos: (i32, i32)) -> bool {
    self.hold.moved(pos)
  }

  // the moved pointer is (or is not) on a held thing
  pub fn hold_over(&mut self, over: bool, now: Instant) {
    let tail = !self.top && self.scroll == 0;
    // rows that arrive during the hold are counted but not shown: take_paint keeps the view where it is
    self.hold.on(over, tail, now);
  }

  // the pointer left the chat, or the window lost it
  pub fn hold_leave(&mut self) -> bool {
    let was = self.hold.leave();
    if was {
      self.pin_back();
    }
    was
  }

  // the let-go beat ran out
  pub fn hold_tick(&mut self, now: Instant) -> bool {
    if self.hold.due().is_none_or(|t| t > now) {
      return false;
    }
    self.hold.release = None;
    if std::mem::take(&mut self.hold.held) {
      self.pin_back();
    }
    true
  }

  // a scroll the reader asked for ends the hold and leaves a plain pause where it is
  pub(super) fn hold_cancel(&mut self) {
    self.hold.cancel();
  }

  // lines that came during the hold show, and the list is at its tail again (even if nothing came: the
  // box may have changed size meanwhile)
  fn pin_back(&mut self) {
    self.scroll = 0;
    self.unread = 0;
    self.stale = true;
  }
}
