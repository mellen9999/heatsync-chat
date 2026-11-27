// youtube's own backlog of a room: the relay sends it on subscribe, richer than the archive's copy of the same lines
use super::*;

impl ChatView {
  // youtube's own backlog of a room (sent on subscribe) meets the archive's lean copy of the same lines: a line
  // already up as history takes the richer one in its place (a line a mod took back stays grey), a line not up
  // yet goes in by its time like any history, and one said live is left as it is. the selection stays
  pub fn enrich(&mut self, msgs: Vec<Message>) {
    let at: std::collections::HashMap<&str, usize> =
      self.rows.iter().enumerate().filter(|(_, r)| !r.msg.id.is_empty()).map(|(i, r)| (&*r.msg.id, i)).collect();
    let mut swaps = Vec::new();
    let mut fresh = Vec::new();
    for mut m in msgs {
      match at.get(&*m.id) {
        Some(&i) if self.rows[i].msg.has(flag::HISTORY) => {
          m.flags |= self.rows[i].msg.flags & (flag::CLEARED | flag::DELETED);
          swaps.push((self.rows[i].msg.id.clone(), m));
        }
        Some(_) => {}
        None => fresh.push(m),
      }
    }
    for (id, m) in swaps {
      self.replace(&id, m);
    }
    self.prepend(fresh);
  }
}
