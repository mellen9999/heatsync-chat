// the mentions, whispers and feed tabs' "something new" (ext seen-state.js): lit while the newest
// arrival is later than the moment you last looked. the looked-at moment is shared through heatsync
// (GET/POST /api/user/seen-state, `seen:update` frames), so clearing it on one client clears it on all
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
  Mentions,
  Whispers,
  // the feed tab (the server's name for it)
  Live,
  // the notifs tab: only ever cleared by a look here or a seen:update from another client; the server's own
  // `inbox` time belongs to the site's dm badge, so a look here is never written to it
  Inbox,
}

pub const ALL: [Surface; 4] = [Surface::Mentions, Surface::Whispers, Surface::Live, Surface::Inbox];

// a server time later than this is junk (year 2286); 0 and below are "never"
const MAX_AT: i64 = 9_999_999_999_999;

impl Surface {
  pub fn key(self) -> &'static str {
    match self {
      Surface::Mentions => "mentions",
      Surface::Whispers => "whispers",
      Surface::Live => "live",
      Surface::Inbox => "inbox",
    }
  }

  // the surfaces shared with the account (seen-state, `seen:update`): every one but the notifs tab, whose
  // read state is the account's unread count. the server's `inbox` time is the site's dm badge
  pub fn written(self) -> bool {
    self != Surface::Inbox
  }

  pub fn parse(s: &str) -> Option<Self> {
    ALL.into_iter().find(|x| x.key() == s)
  }
}

// a seen time from the server, bounded
pub fn at(v: Option<&Value>) -> Option<i64> {
  v?.as_i64().filter(|&t| t > 0 && t <= MAX_AT)
}

#[derive(Default)]
pub struct Unseen {
  seen: [i64; 4],
  latest: [i64; 4],
}

impl Unseen {
  pub fn lit(&self, s: Surface) -> bool {
    self.latest[s as usize] > self.seen[s as usize]
  }

  // something arrived at `at`
  pub fn note(&mut self, s: Surface, at: i64) {
    let l = &mut self.latest[s as usize];
    *l = (*l).max(at);
  }

  // you looked (here, or on another client) at `at`. never moves back, like the server's GREATEST
  pub fn saw(&mut self, s: Surface, at: i64) {
    let v = &mut self.seen[s as usize];
    *v = (*v).max(at);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn lit_until_looked_at_and_never_unseen_by_an_older_clear() {
    let mut u = Unseen::default();
    assert!(!u.lit(Surface::Mentions));
    u.note(Surface::Mentions, 100);
    assert!(u.lit(Surface::Mentions) && !u.lit(Surface::Whispers));
    u.saw(Surface::Mentions, 100);
    assert!(!u.lit(Surface::Mentions));
    // a stale clear from another client can't relight or rewind it
    u.saw(Surface::Mentions, 50);
    assert!(!u.lit(Surface::Mentions));
    u.note(Surface::Mentions, 120);
    assert!(u.lit(Surface::Mentions), "a newer arrival lights it again");
    // a clear made elsewhere after it puts it out
    u.saw(Surface::Mentions, 130);
    assert!(!u.lit(Surface::Mentions));
  }

  #[test]
  fn hostile_frames_are_bounded() {
    assert_eq!(Surface::parse("live"), Some(Surface::Live));
    assert_eq!(Surface::parse("inbox"), Some(Surface::Inbox));
    assert_eq!(Surface::parse("tabs"), None);
    assert_eq!(at(Some(&json!(1_700_000_000_000i64))), Some(1_700_000_000_000));
    for bad in [json!(0), json!(-5), json!("1"), json!(1.5), json!(MAX_AT + 1)] {
      assert_eq!(at(Some(&bad)), None, "{bad}");
    }
    assert_eq!(at(None), None);
  }
}
