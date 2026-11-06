// pushes that mean "ask the server again" (follows, blocked emotes, blocks, mention rules): a frame flood must not become
// a request flood. the first push goes at once; ones inside `GAP` of the last ask fold into one trailing ask
// when the gap ends. pure: the app feeds it the clock
use std::time::{Duration, Instant};

pub const GAP: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
  Follows,
  EmoteBlocks,
  Blocks,
  MentionRules,
}

const ALL: [Ask; 4] = [Ask::Follows, Ask::EmoteBlocks, Ask::Blocks, Ask::MentionRules];

fn slot(a: Ask) -> usize {
  a as usize
}

#[derive(Default)]
pub struct Gates {
  last: [Option<Instant>; 4],
  due: [Option<Instant>; 4],
}

impl Gates {
  // a push arrived. true = ask now; false = it waits for (or is folded into) the trailing ask
  pub fn hit(&mut self, a: Ask, now: Instant) -> bool {
    let i = slot(a);
    match self.last[i] {
      Some(t) if now.duration_since(t) < GAP => {
        self.due[i].get_or_insert(t + GAP);
        false
      }
      _ => {
        self.last[i] = Some(now);
        self.due[i] = None;
        true
      }
    }
  }

  // the soonest trailing ask (None = none waiting)
  pub fn due(&self) -> Option<Instant> {
    self.due.iter().flatten().min().copied()
  }

  // the trailing asks whose time has come, each marked as asked now
  pub fn take(&mut self, now: Instant) -> Vec<Ask> {
    let mut out = Vec::new();
    for a in ALL {
      let i = slot(a);
      if self.due[i].is_some_and(|t| t <= now) {
        self.due[i] = None;
        self.last[i] = Some(now);
        out.push(a);
      }
    }
    out
  }

  // signed out or another account: nothing waits
  pub fn clear(&mut self) {
    *self = Self::default();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_first_push_goes_and_a_burst_folds_into_one_trailing_ask() {
    let mut g = Gates::default();
    let t0 = Instant::now();
    assert!(g.hit(Ask::Follows, t0));
    for i in 1..=200 {
      assert!(!g.hit(Ask::Follows, t0 + Duration::from_millis(i * 10)));
    }
    assert_eq!(g.due(), Some(t0 + GAP));
    assert!(g.take(t0 + GAP - Duration::from_millis(1)).is_empty());
    assert_eq!(g.take(t0 + GAP), vec![Ask::Follows]);
    assert!(g.due().is_none());
    // the trailing ask counts as an ask: the gap starts again from it
    assert!(!g.hit(Ask::Follows, t0 + GAP + Duration::from_secs(1)));
    assert!(g.hit(Ask::Follows, t0 + GAP + GAP));
  }

  #[test]
  fn each_ask_has_its_own_gap() {
    let mut g = Gates::default();
    let t0 = Instant::now();
    assert!(g.hit(Ask::Follows, t0) && g.hit(Ask::Blocks, t0) && g.hit(Ask::EmoteBlocks, t0));
    assert!(!g.hit(Ask::Blocks, t0));
    g.clear();
    assert!(g.due().is_none() && g.hit(Ask::Blocks, t0));
  }
}
