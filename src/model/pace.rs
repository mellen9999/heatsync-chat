// youtube chat, dripped (ext social.js YT POLL SMOOTHING): heatsync polls youtube every few seconds and
// sends each poll as one batch, which would land all at once. each channel's batch is let out one line
// at a time, spaced by the real gaps between the lines (as youtube stamped them), never closer than 60ms
// nor further than 400ms. a quiet channel's first line shows at once; a backlog past 1.5s flushes the
// oldest so nothing goes stale. times are ms
use std::collections::{HashMap, VecDeque};

use super::message::Message;

const MIN_MS: i64 = 60;
const MAX_MS: i64 = 400;
const BURST_MS: i64 = 1500;

#[derive(Default)]
struct Chan {
  q: VecDeque<Message>,
  // when the last line went out, and its own stamp
  out_at: i64,
  out_ts: i64,
  due: Option<i64>,
}

impl Chan {
  fn gap(&self, next: &Message) -> i64 {
    if self.out_ts <= 0 || next.ts <= self.out_ts {
      return MIN_MS;
    }
    (next.ts - self.out_ts).clamp(MIN_MS, MAX_MS)
  }

  fn out(&mut self, m: Message, now: i64, to: &mut Vec<Message>) {
    (self.out_at, self.out_ts) = (now, m.ts);
    to.push(m);
  }
}

#[derive(Default)]
pub struct Pacer {
  chans: HashMap<Box<str>, Chan>,
}

impl Pacer {
  // a batch arrived: what shows now (the rest waits for `due`)
  pub fn push(&mut self, key: &str, msgs: Vec<Message>, now: i64) -> Vec<Message> {
    let c = self.chans.entry(key.into()).or_default();
    let mut now_out = Vec::new();
    for m in msgs {
      if c.q.is_empty() && now - c.out_at >= MAX_MS {
        c.out(m, now, &mut now_out);
        continue;
      }
      c.q.push_back(m);
      while c.q.len() as i64 * MIN_MS > BURST_MS {
        let m = c.q.pop_front().expect("over the cap");
        c.out(m, now, &mut now_out);
      }
    }
    if c.due.is_none()
      && let Some(next) = c.q.front()
    {
      c.due = Some(now + (c.gap(next) - (now - c.out_at)).max(0));
    }
    now_out
  }

  // the lines whose turn has come, per channel
  pub fn due(&mut self, now: i64) -> Vec<(Box<str>, Vec<Message>)> {
    let mut out = Vec::new();
    for (key, c) in &mut self.chans {
      let mut got = Vec::new();
      while c.due.is_some_and(|d| d <= now) {
        let Some(m) = c.q.pop_front() else {
          c.due = None;
          break;
        };
        c.out(m, now, &mut got);
        c.due = c.q.front().map(|n| now + c.gap(n));
      }
      if !got.is_empty() {
        out.push((key.clone(), got));
      }
    }
    self.chans.retain(|_, c| !c.q.is_empty() || now - c.out_at < MAX_MS);
    out
  }

  // when the next line is due
  pub fn next(&self) -> Option<i64> {
    self.chans.values().filter_map(|c| c.due).min()
  }

  // everything waiting for one channel, now (a removal must see every line first; a channel left)
  pub fn flush(&mut self, key: &str) -> Vec<Message> {
    self.chans.remove(key).map(|c| c.q.into()).unwrap_or_default()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::Platform;

  fn at(ts: i64) -> Message {
    Message::system(ts, Platform::Youtube, "x")
  }

  #[test]
  fn a_quiet_channels_first_line_shows_at_once_and_a_batch_drips() {
    let mut p = Pacer::default();
    let now = 10_000;
    // youtube stamped them 200ms, 1s and 10ms apart
    let out = p.push("c", vec![at(1000), at(1200), at(2200), at(2210)], now);
    assert_eq!(out.len(), 1, "the first at once");
    assert_eq!(p.next(), Some(now + 200), "the next at its real gap");
    assert!(p.due(now + 199).is_empty());
    assert_eq!(p.due(now + 200)[0].1.len(), 1);
    assert_eq!(p.next(), Some(now + 600), "a long gap waits at most 400ms");
    p.due(now + 600);
    assert_eq!(p.next(), Some(now + 660), "a short gap is at least 60ms");
    p.due(now + 660);
    assert_eq!(p.next(), None, "all out");
  }

  #[test]
  fn a_backlog_flushes_its_oldest() {
    let mut p = Pacer::default();
    let out = p.push("c", (0..40).map(|i| at(i * 10)).collect(), 5_000);
    assert_eq!(out.len(), 40 - (BURST_MS / MIN_MS) as usize, "the queue never holds more than 1.5s");
    assert_eq!(p.flush("c").len(), (BURST_MS / MIN_MS) as usize);
    assert_eq!(p.next(), None);
  }

  #[test]
  fn a_late_lagging_wakeup_lets_out_everything_due() {
    let mut p = Pacer::default();
    p.push("c", vec![at(0), at(100), at(200)], 1_000);
    let got: usize = p.due(9_000).iter().map(|(_, m)| m.len()).sum();
    assert!(got >= 1);
    // never more than one line per pass beyond what is due: drain fully over time
    let mut t = 9_000;
    while p.next().is_some() {
      t += 400;
      p.due(t);
    }
    assert!(p.flush("c").is_empty());
  }
}
