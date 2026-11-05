// heatsync plus tenure (ext lib/plus-tenure.js): a bare `+` after an active plus member's name, its
// color the tenure. the ext's grey ramp folds onto the palette: dim under 3 years, white from 3, yellow
// (its medal gold) from 5. `/api/paints` answers `plus_since` only while they are entitled
use crate::ui::theme;

const MONTH_MS: i64 = 2_629_746_000;

// whole months on plus since `since` (unix ms)
pub fn months(since: i64, now: i64) -> i64 {
  ((now - since) / MONTH_MS).max(0)
}

pub fn ink(since: i64, now: i64) -> u32 {
  match months(since, now) {
    m if m >= 60 => theme::YELLOW,
    m if m >= 36 => theme::WHITE,
    _ => theme::DIM,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tenure_walks_the_palette_ramp() {
    let now = 100 * MONTH_MS;
    assert_eq!(ink(now, now), theme::DIM);
    assert_eq!(ink(now - 35 * MONTH_MS, now), theme::DIM);
    assert_eq!(ink(now - 36 * MONTH_MS, now), theme::WHITE);
    assert_eq!(ink(now - 60 * MONTH_MS, now), theme::YELLOW);
    // a clock behind the server's: no negative tenure
    assert_eq!(months(now + MONTH_MS, now), 0);
  }
}
