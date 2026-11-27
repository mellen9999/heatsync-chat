// the heatsync mark on rows already here: a line heatsync says it sent flips to [H], and one send on two
// feeds is one row
use super::*;

// how far back a standing row is looked for (the ext's LEG_SCAN_ROWS)
const LEG_ROWS: usize = 600;

impl ChatView {
  // a line born [H] with a send id that another feed's row here already carries is that send's second copy
  pub(super) fn fold_of(&mut self, m: &Message) -> bool {
    m.x.as_ref().is_some_and(|x| x.origin && !x.send.is_empty() && !self.sends.claim(&x.send, m.platform))
  }

  // a row that stands for a send (shown, not a twitch row, no twitch leg lent yet): it can take the twitch room
  fn wants_leg(r: &Row) -> bool {
    r.msg.platform != Platform::Twitch
      && r.msg.kind.said()
      && !r.folded
      && !r.msg.x.as_deref().is_some_and(|x| x.twitch_leg)
  }

  // the twitch copy of the row's send folded into it: it stands for both legs, so it reads its words in the twitch
  // room (emotes::resolve room_platform) and is laid out again. per view: the line another view holds is its own
  fn lend(r: &mut Row) {
    r.msg.extra().twitch_leg = true;
    Self::touch(r);
  }

  // every send whose twitch copy folded away since the last frame: the row that stands for it takes the twitch room
  // (a full repaint: it may be anywhere on screen). settled in `take_paint`, so a burst of rows pays once and a tab
  // nobody is looking at pays nothing; nothing owed (almost always) costs nothing
  pub(super) fn lend_owed(&mut self) {
    for sid in self.sends.take_owed() {
      let standing = |r: &&mut Row| Self::wants_leg(r) && r.msg.x.as_deref().is_some_and(|x| *x.send == *sid);
      if let Some(r) = self.rows.iter_mut().rev().take(LEG_ROWS).find(standing) {
        Self::lend(r);
        self.stale = true;
      }
    }
  }

  // the twitch copy of your own send folded away (app::sentfold drops it before any view sees it): the rows `f` picks
  // among the newest, of another platform, take the twitch room. true = a row changed
  pub fn lend_twitch(&mut self, f: impl Fn(&Message) -> bool) -> bool {
    let mut any = false;
    for r in self.rows.iter_mut().rev().take(LEG_ROWS).filter(|r| Self::wants_leg(r) && f(&r.msg)) {
      Self::lend(r);
      any = true;
    }
    self.stale |= any;
    any
  }

  // heatsync says these lines (message id → send id) went through heatsync: the rows here for them flip
  // [T]/[K]/[Y] → [H] in place, the only change a mark may make; a second feed's copy of a send folds away
  pub fn flip_origin(&mut self, p: Platform, ids: &std::collections::HashMap<Box<str>, Box<str>>) -> bool {
    let mut any = false;
    for r in self.rows.iter_mut().filter(|r| r.msg.platform == p && r.msg.kind.said() && !r.msg.id.is_empty()) {
      let Some(sid) = ids.get(&r.msg.id) else { continue };
      // already [H] by its send id: said. [H] by your own words alone (the own-send notice) it still joins its
      // send here, or it never claims it and another feed's copy stands beside it
      if r.msg.x.as_ref().is_some_and(|x| x.origin && (!x.send.is_empty() || sid.is_empty())) {
        continue;
      }
      let x = r.msg.extra();
      x.origin = true;
      x.send = sid.clone();
      if !sid.is_empty() && !self.sends.claim(sid, p) {
        r.folded = true;
      }
      Self::touch(r);
      any = true;
    }
    self.stale |= any;
    any
  }

  // your twitch look in a room changed: your kick and youtube lines are laid out again (model::own_look)
  pub fn touch_own(&mut self) {
    for r in self.rows.iter_mut().filter(|r| r.msg.platform != Platform::Twitch && r.msg.kind.said()) {
      Self::touch(r);
      self.stale = true;
    }
  }
}
