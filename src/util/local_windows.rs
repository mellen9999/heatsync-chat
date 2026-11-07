// the local calendar time of a moment, from the system's own time zone rules (dst included)
use super::Local;
use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

// 100ns ticks between 1601-01-01 (windows' zero) and 1970-01-01
const EPOCH_GAP_SECS: i64 = 11_644_473_600;

// unix ms as windows ticks; None before 1601
fn ticks(ms: i64) -> Option<u64> {
  let secs = ms.div_euclid(1000).checked_add(EPOCH_GAP_SECS)?;
  let sub = ms.rem_euclid(1000);
  u64::try_from(secs).ok()?.checked_mul(10_000_000)?.checked_add(sub as u64 * 10_000)
}

// None when the system can't say (a time out of range)
pub fn of(ms: i64) -> Option<Local> {
  let t = ticks(ms)?;
  let ft = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
  // SAFETY: both structs are plain data we own; each call fills the one it is given
  let (mut utc, mut local): (SYSTEMTIME, SYSTEMTIME) = unsafe { std::mem::zeroed() };
  unsafe {
    if FileTimeToSystemTime(&ft, &mut utc) == 0
      || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
    {
      return None;
    }
  }
  Some(Local {
    y: i32::from(local.wYear),
    mo: u32::from(local.wMonth),
    d: u32::from(local.wDay),
    h: u32::from(local.wHour),
    mi: u32::from(local.wMinute),
    s: u32::from(local.wSecond),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn unix_moments_become_windows_ticks() {
    assert_eq!(ticks(0), Some(116_444_736_000_000_000));
    assert_eq!(ticks(1), Some(116_444_736_000_010_000));
    assert_eq!(ticks(-1), Some(116_444_735_999_990_000), "just before 1970 is still a moment");
    assert_eq!(ticks(-12_000_000_000_000), None, "before 1601 is not");
  }

  #[test]
  fn the_clock_reads_back_a_known_day() {
    // whatever the zone, a mid-day utc moment is the same calendar day within 14 hours either way
    let ms = 1_793_399_211_099; // 2026-10-30T22:26:51Z
    let t = of(ms).unwrap();
    assert_eq!((t.y, t.mo), (2026, 10));
    assert!(t.d == 30 || t.d == 31, "{t:?}");
    assert_eq!(t.s, 51);
  }
}
