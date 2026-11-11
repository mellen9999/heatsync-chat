// the heatsync mark on rows already here: a line heatsync says it sent flips to [H], and one send on two
// feeds is one row
use super::*;

impl ChatView {
  // a line born [H] with a send id that another feed's row here already carries is that send's second copy
  pub(super) fn fold_of(&mut self, m: &Message) -> bool {
    m.x.as_ref().is_some_and(|x| x.origin && !x.send.is_empty() && !self.sends.claim(&x.send, m.platform))
  }

  // heatsync says these lines (message id → send id) went through heatsync: the rows here for them flip
  // [T]/[K]/[Y] → [H] in place, the only change a mark may make; a second feed's copy of a send folds away
  pub fn flip_origin(&mut self, p: Platform, ids: &std::collections::HashMap<Box<str>, Box<str>>) -> bool {
    let mut any = false;
    for r in self.rows.iter_mut().filter(|r| r.msg.platform == p && r.msg.kind.said() && !r.msg.id.is_empty()) {
      let Some(sid) = ids.get(&r.msg.id) else { continue };
      if r.msg.x.as_ref().is_some_and(|x| x.origin) {
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
}
