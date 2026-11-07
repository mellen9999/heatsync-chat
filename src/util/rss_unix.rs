// this process's memory for the bench, from /proc: (resident, peak, anonymous, file-backed) in kb
pub fn kb() -> (u64, u64, u64, u64) {
  let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
  let field = |k: &str| {
    s.lines()
      .find(|l| l.starts_with(k))
      .and_then(|l| l.split_whitespace().nth(1))
      .and_then(|v| v.parse().ok())
      .unwrap_or(0)
  };
  (field("VmRSS:"), field("VmHWM:"), field("RssAnon:"), field("RssFile:"))
}
