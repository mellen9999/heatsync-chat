// wall clock helpers; local time via libc so dst/tz follow the system
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

// "hh:mm" in local time
pub fn hhmm(ms: i64) -> String {
  let t: libc::time_t = (ms / 1000) as libc::time_t;
  // SAFETY: localtime_r writes only into the tm we hand it
  let mut tm: libc::tm = unsafe { std::mem::zeroed() };
  let ok = !unsafe { libc::localtime_r(&t, &mut tm) }.is_null();
  if !ok {
    return "--:--".into();
  }
  format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

// "2026-10-30T22:26:51.099Z" → unix ms (utc only; that's all the archive emits)
pub fn parse_iso(s: &str) -> Option<i64> {
  let b = s.as_bytes();
  if b.len() < 19 || b[4] != b'-' || b[10] != b'T' {
    return None;
  }
  let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
  let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
  let ms = s.get(20..23).and_then(|f| f.parse::<i64>().ok()).filter(|_| b.get(19) == Some(&b'.')).unwrap_or(0);
  // days from civil (howard hinnant)
  let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
  let era = y2.div_euclid(400);
  let yoe = y2 - era * 400;
  let doy = (153 * m2 + 2) / 5 + d - 1;
  let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  let days = era * 146097 + doe - 719468;
  Some(((days * 24 + h) * 60 + mi) * 60_000 + se * 1000 + ms)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn iso() {
    assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_iso("2026-10-30T22:26:51.099Z"), Some(1793399211099));
    assert_eq!(parse_iso("garbage"), None);
  }
}
