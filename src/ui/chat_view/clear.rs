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
    self.clear_where(|m| &*m.id == id);
  }

  // youtube lines by message id or author channel id (a ban takes back their backlog)
  pub fn clear_yt(&mut self, ids: &[Box<str>], authors: &[Box<str>]) {
    self.clear_where(|m| {
      m.platform == Platform::Youtube
        && ((!m.id.is_empty() && ids.contains(&m.id)) || (!m.user_id.is_empty() && authors.contains(&m.user_id)))
    });
  }

  // a ban or timeout on `p`: that feed's lines of the login. a tab on twitch and kick carries two people who
  // share a name, and a twitch ban greyed the kick one too
  pub fn clear_user(&mut self, p: Platform, login: &str) {
    self.clear_where(|m| m.platform == p && &*m.login == login && m.kind != Kind::System);
  }

  // grey every line `hit` picks, then every copy of the sends they belong to
  fn clear_where(&mut self, hit: impl Fn(&Message) -> bool) {
    let sends = self.grey(hit);
    if !sends.is_empty() {
      self.grey(|m| send_of(m).is_some_and(|s| sends.iter().any(|t| **t == *s)));
    }
  }

  // grey what `hit` picks; the sends the greyed lines are copies of
  fn grey(&mut self, hit: impl Fn(&Message) -> bool) -> Vec<Box<str>> {
    let mut sends = Vec::new();
    for r in self.rows.iter_mut().filter(|r| r.msg.flags & flag::CLEARED == 0 && hit(&r.msg)) {
      r.msg.flags |= flag::CLEARED;
      Self::touch(r);
      self.stale = true;
      sends.extend(send_of(&r.msg).map(Box::from));
    }
    sends
  }
}
