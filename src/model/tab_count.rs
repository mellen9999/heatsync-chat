// the number after a channel tab's name (ext tab-counter.js), one setting, three ways:
//   unread: messages since you last had the tab open (none on the tab you're reading)
//   heat:   messages in the last 60 seconds, the tab you're reading too (how busy it is)
//   off
// heat is a ring of per-second counts advanced by elapsed time on every touch, so a quiet stretch
// costs nothing and a number is never stale
use super::settings::Counter;

const BUCKETS: usize = 90;
const WINDOW: usize = 60;
// a busy channel must not keep widening its tab
const MAX: u32 = 999;

pub struct Count {
  unread: u32,
  ring: [u16; BUCKETS],
  // the second the ring's last slot is
  sec: i64,
}

impl Default for Count {
  fn default() -> Self {
    Self { unread: 0, ring: [0; BUCKETS], sec: 0 }
  }
}

impl Count {
  fn advance(&mut self, now: i64) {
    let d = now - self.sec;
    if d <= 0 {
      return;
    }
    self.sec = now;
    if d >= BUCKETS as i64 {
      self.ring = [0; BUCKETS];
    } else {
      self.ring.rotate_left(d as usize);
      self.ring[BUCKETS - d as usize..].fill(0);
    }
  }

  // one message arrived (`now` in seconds)
  pub fn bump(&mut self, active: bool, now: i64) {
    self.advance(now);
    let last = &mut self.ring[BUCKETS - 1];
    *last = last.saturating_add(1);
    if !active {
      self.unread = self.unread.saturating_add(1);
    }
  }

  pub fn read(&mut self) {
    self.unread = 0;
  }

  pub fn heat(&mut self, now: i64) -> u32 {
    self.advance(now);
    self.ring[BUCKETS - WINDOW..].iter().map(|&n| u32::from(n)).sum()
  }

  // what the tab shows; empty = nothing
  pub fn text(&mut self, mode: Counter, active: bool, now: i64) -> String {
    let n = match mode {
      Counter::Off => 0,
      Counter::Unread if active => 0,
      Counter::Unread => self.unread,
      Counter::Heat => self.heat(now),
    };
    match n {
      0 => String::new(),
      n if n > MAX => format!("{MAX}+"),
      n => n.to_string(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn unread_counts_away_and_clears_on_read() {
    let mut c = Count::default();
    c.bump(true, 100);
    assert_eq!(c.text(Counter::Unread, false, 100), "", "read while it came in");
    for _ in 0..3 {
      c.bump(false, 101);
    }
    assert_eq!(c.text(Counter::Unread, false, 101), "3");
    assert_eq!(c.text(Counter::Unread, true, 101), "", "never on the tab you're reading");
    c.read();
    assert_eq!(c.text(Counter::Unread, false, 101), "");
    assert_eq!(c.text(Counter::Off, false, 101), "");
  }

  #[test]
  fn heat_is_the_last_minute() {
    let mut c = Count::default();
    c.bump(true, 1000);
    c.bump(false, 1030);
    assert_eq!(c.text(Counter::Heat, true, 1030), "2", "the tab you're reading counts too");
    assert_eq!(c.heat(1060), 1, "the first fell out of the minute");
    assert_eq!(c.heat(5000), 0, "a long quiet clears it");
  }

  #[test]
  fn capped() {
    let mut c = Count::default();
    for _ in 0..1200 {
      c.bump(false, 7);
    }
    assert_eq!(c.text(Counter::Unread, false, 7), "999+");
  }
}
