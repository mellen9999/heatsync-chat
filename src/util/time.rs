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
