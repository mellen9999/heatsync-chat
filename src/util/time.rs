// wall clock helpers; local time via libc so dst/tz follow the system
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

// the wall clock in local time: "16:05", or the ext's 12h form "4:05pm"
pub fn clock(ms: i64, twelve: bool) -> String {
  let t: libc::time_t = (ms / 1000) as libc::time_t;
  // SAFETY: localtime_r writes only into the tm we hand it
  let mut tm: libc::tm = unsafe { std::mem::zeroed() };
  let ok = !unsafe { libc::localtime_r(&t, &mut tm) }.is_null();
  if !ok {
    return "--:--".into();
  }
  hm(tm.tm_hour, tm.tm_min, twelve)
}

// a moment broken into local calendar parts
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
  pub y: i32,
  pub mo: u32,
  pub d: u32,
  pub h: u32,
  pub mi: u32,
  pub s: u32,
}

impl Local {
  // None when the system can't say (a time out of range)
  pub fn of(ms: i64) -> Option<Local> {
    let t: libc::time_t = ms.div_euclid(1000) as libc::time_t;
    // SAFETY: localtime_r writes only into the tm we hand it
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
      return None;
    }
    let u = |v: i32| v.max(0) as u32;
    Some(Local {
      y: tm.tm_year + 1900,
      mo: u(tm.tm_mon + 1),
      d: u(tm.tm_mday),
      h: u(tm.tm_hour),
      mi: u(tm.tm_min),
      s: u(tm.tm_sec),
    })
  }

  // "2026-11-06"
  pub fn day(self) -> String {
    format!("{:04}-{:02}-{:02}", self.y, self.mo, self.d)
  }

  // "09:05:03"
  pub fn hms(self) -> String {
    format!("{:02}:{:02}:{:02}", self.h, self.mi, self.s)
  }
}

fn hm(h: i32, m: i32, twelve: bool) -> String {
  if !twelve {
    return format!("{h:02}:{m:02}");
  }
  let h12 = if h % 12 == 0 { 12 } else { h % 12 };
  format!("{h12}:{m:02}{}", if h >= 12 { "pm" } else { "am" })
}

// "hh:mm" in local time
pub fn hhmm(ms: i64) -> String {
  clock(ms, false)
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

// unix ms → (year, month, day, hour, minute, second) in utc
fn civil(ms: i64) -> (i64, i64, i64, i64, i64, i64) {
  let secs = ms.div_euclid(1000);
  let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
  // civil from days (howard hinnant)
  let z = days + 719468;
  let era = z.div_euclid(146097);
  let doe = z - era * 146097;
  let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  let y = yoe + era * 400 + i64::from(m <= 2);
  (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

// "2026-10-30": the utc day (the archive partitions on it)
pub fn utc_day(ms: i64) -> String {
  let (y, m, d, ..) = civil(ms);
  format!("{y:04}-{m:02}-{d:02}")
}

// "10-30 22:26:51": utc, as the archive keeps it
pub fn utc_stamp(ms: i64) -> String {
  let (_, m, d, h, mi, s) = civil(ms);
  format!("{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

// "90", "30s", "10m", "2h", "3d" → seconds. None = not a length
pub fn span(s: &str) -> Option<u64> {
  let s = s.trim().to_ascii_lowercase();
  let (num, mult) = match s.chars().last()? {
    'd' => (&s[..s.len() - 1], 86400),
    'h' => (&s[..s.len() - 1], 3600),
    'm' => (&s[..s.len() - 1], 60),
    's' => (&s[..s.len() - 1], 1),
    c if c.is_ascii_digit() => (&s[..], 1),
    _ => return None,
  };
  num.parse::<u64>().ok()?.checked_mul(mult)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn twelve_hour_clock_is_the_exts() {
    assert_eq!(hm(0, 5, true), "12:05am");
    assert_eq!(hm(9, 30, true), "9:30am");
    assert_eq!(hm(12, 0, true), "12:00pm");
    assert_eq!(hm(16, 5, true), "4:05pm");
    assert_eq!(hm(23, 59, true), "11:59pm");
    assert_eq!(hm(16, 5, false), "16:05");
    assert_eq!(hm(0, 5, false), "00:05");
  }

  #[test]
  fn spans() {
    assert_eq!(span("90"), Some(90));
    assert_eq!(span("10M"), Some(600));
    assert_eq!(span("2h"), Some(7200));
    assert_eq!(span("3d"), Some(259200));
    assert_eq!(span("x"), None);
    assert_eq!(span(""), None);
    assert_eq!(span("99999999999999999999d"), None);
  }

  #[test]
  fn iso() {
    assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_iso("2026-10-30T22:26:51.099Z"), Some(1793399211099));
    assert_eq!(parse_iso("garbage"), None);
  }

  #[test]
  fn utc_day_and_stamp_round_trip_across_midnight() {
    let t = parse_iso("2026-10-30T22:26:51.099Z").unwrap();
    assert_eq!((utc_day(t).as_str(), utc_stamp(t).as_str()), ("2026-10-30", "10-30 22:26:51"));
    let before = parse_iso("2026-10-31T23:59:59.999Z").unwrap();
    assert_eq!(utc_day(before), "2026-10-31");
    assert_eq!(utc_day(before + 1), "2026-11-01", "one ms later is the next utc day");
    assert_eq!(utc_day(parse_iso("2024-02-29T12:00:00Z").unwrap()), "2024-02-29");
    assert_eq!(utc_day(0), "1970-01-01");
    assert_eq!(utc_day(-1), "1969-12-31");
  }
}
