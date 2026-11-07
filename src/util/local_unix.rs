// the local calendar time of a moment, from libc so dst/tz follow the system
use super::Local;

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
