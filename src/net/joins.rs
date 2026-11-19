// twitch drops joins past 20 per 10 seconds without a word (a normal login and an anonymous one alike), so a
// window with more channels than that ends up deaf in the rest. every join goes through this queue: at most
// 20 per window, the remainder waits its turn, one JOIN line per burst
use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::Instant;

const BUDGET: usize = 20;
// twitch's 10s plus a margin for the wire (a window counted on their side can lag ours)
const WINDOW: Duration = Duration::from_secs(11);

#[derive(Default)]
pub struct Joins {
  queue: VecDeque<Box<str>>,
  sent: VecDeque<Instant>,
}

impl Joins {
  pub fn push(&mut self, ch: Box<str>) {
    if !self.queue.contains(&ch) {
      self.queue.push_back(ch);
    }
  }

  pub fn waiting(&self) -> bool {
    !self.queue.is_empty()
  }

  // a channel parted before its turn never needs a join: true = it was waiting (no PART to send)
  pub fn cancel(&mut self, ch: &str) -> bool {
    let n = self.queue.len();
    self.queue.retain(|c| &**c != ch);
    self.queue.len() != n
  }

  fn forget_old(&mut self, now: Instant) {
    while self.sent.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
      self.sent.pop_front();
    }
  }

  // when the next join may go (None = nothing waits)
  pub fn due(&mut self, now: Instant) -> Option<Instant> {
    if self.queue.is_empty() {
      return None;
    }
    self.forget_old(now);
    Some(if self.sent.len() < BUDGET { now } else { self.sent[self.sent.len() - BUDGET] + WINDOW })
  }

  // the JOIN line for what the window has room for right now
  pub fn take(&mut self, now: Instant) -> Option<String> {
    self.forget_old(now);
    let room = BUDGET.saturating_sub(self.sent.len()).min(self.queue.len());
    if room == 0 {
      return None;
    }
    let names: Vec<String> = self.queue.drain(..room).map(|c| format!("#{c}")).collect();
    self.sent.extend(std::iter::repeat_n(now, room));
    Some(format!("JOIN {}", names.join(",")))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn chans(n: usize) -> Joins {
    let mut j = Joins::default();
    for i in 0..n {
      j.push(format!("c{i}").into());
    }
    j
  }

  fn count(line: &str) -> usize {
    line.matches('#').count()
  }

  #[test]
  fn twenty_go_at_once_and_the_rest_wait_out_the_window() {
    let t0 = Instant::now();
    let mut j = chans(45);
    assert_eq!(count(&j.take(t0).unwrap()), 20);
    assert_eq!(j.due(t0), Some(t0 + WINDOW), "the next burst is a window later");
    assert!(j.take(t0 + Duration::from_secs(5)).is_none(), "nothing before then");
    let t1 = t0 + WINDOW;
    assert_eq!(count(&j.take(t1).unwrap()), 20);
    let t2 = t1 + WINDOW;
    assert_eq!(count(&j.take(t2).unwrap()), 5);
    assert_eq!(j.due(t2), None);
  }

  #[test]
  fn every_channel_is_joined_exactly_once() {
    let t0 = Instant::now();
    let mut j = chans(61);
    let mut seen = Vec::new();
    for k in 0..4 {
      let now = t0 + WINDOW * k;
      seen.extend(j.take(now).into_iter().flat_map(|l| l[5..].split(',').map(str::to_owned).collect::<Vec<_>>()));
    }
    let all = seen.len();
    seen.sort();
    seen.dedup();
    assert_eq!((all, seen.len()), (61, 61));
  }

  // the window slides: a join a bit later does not free twenty
  #[test]
  fn a_late_join_counts_against_the_budget() {
    let t0 = Instant::now();
    let mut j = chans(19);
    j.take(t0).unwrap();
    let t1 = t0 + Duration::from_secs(8);
    j.push("late".into());
    assert_eq!(j.take(t1).as_deref(), Some("JOIN #late"));
    for i in 0..5 {
      j.push(format!("more{i}").into());
    }
    assert!(j.take(t1).is_none(), "20 in the last window already");
    assert_eq!(j.due(t1), Some(t0 + WINDOW), "the 19 age out first");
    assert_eq!(count(&j.take(t0 + WINDOW).unwrap()), 5);
  }

  #[test]
  fn a_few_channels_are_one_line_with_no_wait() {
    let t0 = Instant::now();
    let mut j = chans(3);
    assert_eq!(j.due(t0), Some(t0));
    assert_eq!(j.take(t0).as_deref(), Some("JOIN #c0,#c1,#c2"));
  }

  #[test]
  fn a_channel_parted_in_line_is_never_joined_and_one_never_doubles() {
    let t0 = Instant::now();
    let mut j = chans(2);
    j.push("c0".into());
    assert!(j.cancel("c1"));
    assert!(!j.cancel("c1"));
    assert_eq!(j.take(t0).as_deref(), Some("JOIN #c0"));
  }
}
