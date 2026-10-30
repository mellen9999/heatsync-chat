// raw image bytes on disk under $XDG_CACHE_HOME/heatsync-chat/img, keyed by url hash.
// decoded frames are cheap to rebuild from here, so memory can evict freely
use std::path::PathBuf;

fn dir() -> Option<PathBuf> {
  let base = std::env::var_os("XDG_CACHE_HOME")
    .map(PathBuf::from)
    .filter(|p| p.is_absolute())
    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
  Some(base.join("heatsync-chat").join("img"))
}

// fnv-1a 64: stable across runs (std's hasher is randomly seeded)
fn key(url: &str) -> String {
  let h = url
    .bytes()
    .fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
  format!("{h:016x}")
}

pub fn read(url: &str) -> Option<Vec<u8>> {
  std::fs::read(dir()?.join(key(url))).ok()
}

pub fn write(url: &str, bytes: &[u8]) {
  let Some(d) = dir() else { return };
  if std::fs::create_dir_all(&d).is_err() {
    return;
  }
  let k = key(url);
  let tmp = d.join(format!(".{k}.tmp"));
  if std::fs::write(&tmp, bytes).is_ok() {
    let _ = std::fs::rename(tmp, d.join(k));
  }
}

// keep the cache bounded: drop least-recently-written files past `max` bytes
pub fn prune(max: u64) {
  let Some(d) = dir() else { return };
  let Ok(rd) = std::fs::read_dir(&d) else { return };
  let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
    .filter_map(Result::ok)
    .filter_map(|e| {
      let m = e.metadata().ok()?;
      m.is_file()
        .then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
    })
    .collect();
  let mut total: u64 = files.iter().map(|f| f.1).sum();
  if total <= max {
    return;
  }
  files.sort_by_key(|f| f.0);
  for (_, len, p) in files {
    if total <= max {
      break;
    }
    if std::fs::remove_file(p).is_ok() {
      total -= len;
    }
  }
}
