// a moderator's removals: a deleted line, a banned or timed-out chatter, youtube's own deletions. a removed line
// greys in place, never vanishes. one send on several feeds is one row with its other copies folded under it
// (heat.rs), and a moderator removes ONE feed's copy, which may be a folded one: the send's row greys with it,
// or the line a mod took down stays on screen
use super::*;

// the heatsync send a line is a copy of (none for a line heatsync did not send, or one sent before send ids)
fn send_of(m: &Message) -> Option<&str> {
  m.x.as_deref().map(|x| &*x.send).filter(|s| !s.is_empty())
}

impl ChatView {
  pub fn clear_msg(&mut self, id: &str) {
    self.clear_where(flag::DELETED, |m| &*m.id == id);
  }

  // a post its author deleted: its line stays where it is but the words, emotes and picture are gone
  pub fn empty_post(&mut self, id: &str, said: &str) {
    for r in self.rows.iter_mut().filter(|r| &*r.msg.id == id) {
      let m = &mut r.msg;
      m.body = said.into();
      (m.emotes, m.hs) = (Box::new([]), Box::new([]));
      if let Some(x) = m.x.as_mut() {
        (x.pic, x.react) = (None, Box::new([]));
      }
      Self::touch(r);
      self.stale = true;
    }
  }

  // youtube lines by message id or author channel id (a ban takes back their backlog)
  pub fn clear_yt(&mut self, ids: &[Box<str>], authors: &[Box<str>]) {
    let by_id = |m: &Message| !m.id.is_empty() && ids.contains(&m.id);
    let by_author = |m: &Message| !m.user_id.is_empty() && authors.contains(&m.user_id);
    self.clear_where(flag::DELETED, |m| m.platform == Platform::Youtube && by_id(m));
    self.clear_where(0, |m| m.platform == Platform::Youtube && by_author(m));
  }

  // a ban or timeout on `p`: that feed's lines of the login. a tab on twitch and kick carries two people who
  // share a name, and a twitch ban greyed the kick one too
  pub fn clear_user(&mut self, p: Platform, login: &str) {
    self.clear_where(0, |m| m.platform == p && &*m.login == login && m.kind != Kind::System);
  }

  // an unban on `p`: the grey a ban or timeout put on the login's lines goes; a deleted line stays grey
  pub fn lift_ban(&mut self, p: Platform, login: &str) {
    for r in self.rows.iter_mut() {
      let m = &mut r.msg;
      if m.platform == p && &*m.login == login && m.flags & flag::CLEARED != 0 && m.flags & flag::DELETED == 0 {
        m.flags &= !flag::CLEARED;
        Self::touch(r);
        self.stale = true;
      }
    }
  }

  // grey every line `hit` picks, then every copy of the sends they belong to. `why` is DELETED for a deleted line
  // (grey for good), 0 for a ban or timeout (an unban lifts it)
  fn clear_where(&mut self, why: u16, hit: impl Fn(&Message) -> bool) {
    let sends = self.grey(why, hit);
    if !sends.is_empty() {
      self.grey(why, |m| send_of(m).is_some_and(|s| sends.iter().any(|t| **t == *s)));
    }
  }

  // grey what `hit` picks; the sends the greyed lines are copies of
  fn grey(&mut self, why: u16, hit: impl Fn(&Message) -> bool) -> Vec<Box<str>> {
    let mut sends = Vec::new();
    // a delete reaches a line a ban already greyed: it becomes a deleted one, which an unban leaves alone
    let new = |f: u16| f & flag::CLEARED == 0 || f & why != why;
    for r in self.rows.iter_mut().filter(|r| new(r.msg.flags) && hit(&r.msg)) {
      r.msg.flags |= flag::CLEARED | why;
      Self::touch(r);
      self.stale = true;
      sends.extend(send_of(&r.msg).map(Box::from));
    }
    sends
  }
}
