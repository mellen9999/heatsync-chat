// this process's memory for the bench: (working set, peak working set, private, working set minus private) in
// kb. windows has no anonymous/file split; "private" (the commit charge: memory only this process owns) is
// the nearest to linux's anon, and the rest of the working set is shared pages (the binary, dlls, mapped fonts)
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

pub fn kb() -> (u64, u64, u64, u64) {
  // SAFETY: a plain query of our own process into a struct whose size is set first
  let mut m: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
  m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
  if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb) } == 0 {
    return (0, 0, 0, 0);
  }
  let k = |b: usize| b as u64 >> 10;
  let (ws, private) = (k(m.WorkingSetSize), k(m.PagefileUsage));
  (ws, k(m.PeakWorkingSetSize), private, ws.saturating_sub(private))
}
