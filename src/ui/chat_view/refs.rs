// emote refs that reach a line after it is on screen: heatsync's name-matches for a twitch line (by line id)
// and a sender's pick (their app's vouch, by who said it). only the line they land on lays out again
use super::*;

impl ChatView {
  // heatsync's refs for a twitch line landed after it: added to that line, which alone lays out again.
  // false = no such line here
  pub fn merge_hs(&mut self, id: &str, refs: &[crate::model::message::HsEmote]) -> bool {
    let mut hit = false;
    for r in self.rows.iter_mut().filter(|r| r.msg.platform == Platform::Twitch && &*r.msg.id == id) {
      hit = true;
      if crate::net::relay::merge_hs(&mut r.msg.hs, refs) {
        Self::touch(r);
        self.stale = true;
      }
    }
    hit
  }

  // a sender's pick landed after their line: it goes on the newest line of theirs (no older than `since`)
  // that names the word and has no pick for it yet, which alone lays out again. false = no such line here
  pub fn late_pick(&mut self, login: &str, pick: &crate::model::message::HsEmote, since: i64) -> bool {
    let wants = |m: &Message| {
      m.kind.said()
        && *m.login == *login
        && crate::heatsync::vouches::names(&m.body, &pick.0)
        && m.x.as_ref().is_none_or(|x| x.picks.iter().all(|p| p.0 != pick.0))
    };
    let Some(r) = self.rows.iter_mut().rev().take_while(|r| r.msg.ts >= since).find(|r| wants(&r.msg)) else {
      return false;
    };
    crate::heatsync::vouches::merge(&mut r.msg.extra().picks, vec![pick.clone()]);
    Self::touch(r);
    self.stale = true;
    true
  }
}
