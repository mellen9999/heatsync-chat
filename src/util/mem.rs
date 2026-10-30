// glibc keeps freed heap around (big emote-set json parses leave ~15MB parked).
// cap the per-thread arenas and hand pages back after bursts of transient work
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn limit_arenas() {
  // SAFETY: plain allocator tuning, called before any other thread exists
  unsafe {
    libc::mallopt(libc::M_ARENA_MAX, 2);
  }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn trim() {
  // SAFETY: releases free heap pages; no pointers are invalidated
  unsafe {
    libc::malloc_trim(0);
  }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn limit_arenas() {}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn trim() {}
