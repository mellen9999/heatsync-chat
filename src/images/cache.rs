// raw image bytes on disk under $XDG_CACHE_HOME/heatsync-chat/img, keyed by the sha256 of the url.
// decoded frames are cheap to rebuild from here, so memory can evict freely. what you looked at is
// yours: the folders are owner-only (0700) and so is every file (0600)
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[cfg(not(test))]
fn dir() -> Option<PathBuf> {
  let base = std::env::var_os("XDG_CACHE_HOME")
    .map(PathBuf::from)
    .filter(|p| p.is_absolute())
    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
  Some(base.join("heatsync-chat").join("img"))
}

// tests never touch the real cache: each test thread gets its own scratch folder
#[cfg(test)]
fn dir() -> Option<PathBuf> {
  Some(crate::store::scratch().join("cache").join("heatsync-chat").join("img"))
}

// sha256 of the url, hex: stable across runs, and nobody can aim two urls at one file
fn key(url: &str) -> String {
  const HEX: &[u8; 16] = b"0123456789abcdef";
  let sum = ring::digest::digest(&ring::digest::SHA256, url.as_bytes());
  let mut s = String::with_capacity(64);
  for b in sum.as_ref() {
    s.push(HEX[usize::from(b >> 4)] as char);
    s.push(HEX[usize::from(b & 15)] as char);
  }
  s
}

// the 16-digit names the old (fnv) keys made: nothing reads them any more
fn legacy(name: &str) -> bool {
  name.len() == 16 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

// a folder that exists and is ours alone: made 0700 when missing, tightened when an older version left it open
fn private(d: &Path) -> bool {
  match std::fs::metadata(d) {
    Ok(m) if !m.is_dir() => false,
    Ok(m) if m.permissions().mode() & 0o077 != 0 => {
      std::fs::set_permissions(d, PermissionsExt::from_mode(0o700)).is_ok()
    }
    Ok(_) => true,
    Err(_) => std::fs::DirBuilder::new().recursive(true).mode(0o700).create(d).is_ok(),
  }
}

// at start: both folders owner-only (the files are tightened as `prune` walks them)
pub fn init() {
  if let Some(d) = dir() {
    if let Some(parent) = d.parent() {
      private(parent);
    }
    private(&d);
  }
}

pub fn read(url: &str) -> Option<Vec<u8>> {
  std::fs::read(dir()?.join(key(url))).ok()
}

pub fn write(url: &str, bytes: &[u8]) {
  let Some(d) = dir() else { return };
  if !private(&d) {
    return;
  }
  let k = key(url);
  let tmp = d.join(format!(".{k}.tmp"));
  let _ = std::fs::remove_file(&tmp);
  let put = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp);
  if put.and_then(|mut f| f.write_all(bytes)).is_ok() && std::fs::rename(&tmp, d.join(k)).is_ok() {
    return;
  }
  let _ = std::fs::remove_file(tmp);
}

// keep the cache bounded: drop least-recently-written files past `max` bytes. on the way: files from
// the old key scheme go, and any file an older version left readable by others goes owner-only
pub fn prune(max: u64) {
  let Some(d) = dir() else { return };
  let Ok(rd) = std::fs::read_dir(&d) else { return };
  let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
  for e in rd.filter_map(Result::ok) {
    let Ok(m) = e.metadata() else { continue };
    if !m.is_file() {
      continue;
    }
    if legacy(&e.file_name().to_string_lossy()) {
      let _ = std::fs::remove_file(e.path());
      continue;
    }
    if m.permissions().mode() & 0o077 != 0 {
      let _ = std::fs::set_permissions(e.path(), PermissionsExt::from_mode(0o600));
    }
    files.push((m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()));
  }
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

#[cfg(test)]
mod tests {
  use super::*;

  fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
  }

  fn fresh() -> PathBuf {
    let d = dir().unwrap();
    let _ = std::fs::remove_dir_all(crate::store::scratch());
    d
  }

  #[test]
  fn keys_are_sha256() {
    // the well-known sha256("abc")
    assert_eq!(key("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_ne!(key("https://cdn.7tv.app/emote/a/1x.webp"), key("https://cdn.7tv.app/emote/b/1x.webp"));
    assert!(!legacy(&key("x")) && legacy("0123456789abcdef") && !legacy("0123456789abcdeg"));
  }

  #[test]
  fn what_goes_in_comes_out_and_stays_private() {
    let d = fresh();
    assert!(read("https://a/b").is_none());
    write("https://a/b", b"bytes");
    assert_eq!(read("https://a/b").as_deref(), Some(&b"bytes"[..]));
    write("https://a/b", b"newer");
    assert_eq!(read("https://a/b").as_deref(), Some(&b"newer"[..]));
    assert_eq!(mode(&d), 0o700);
    assert_eq!(mode(d.parent().unwrap()), 0o700);
    assert_eq!(mode(&d.join(key("https://a/b"))), 0o600);
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no temp file left behind");
  }

  #[test]
  fn init_and_prune_tighten_an_older_cache_and_drop_its_old_keys() {
    let d = fresh();
    std::fs::create_dir_all(&d).unwrap();
    std::fs::set_permissions(&d, PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::set_permissions(d.parent().unwrap(), PermissionsExt::from_mode(0o755)).unwrap();
    let (old, kept) = (d.join("cbf29ce484222325"), d.join(key("https://a/kept")));
    std::fs::write(&old, b"fnv era").unwrap();
    std::fs::write(&kept, b"sha era").unwrap();
    std::fs::set_permissions(&kept, PermissionsExt::from_mode(0o644)).unwrap();
    init();
    assert_eq!((mode(&d), mode(d.parent().unwrap())), (0o700, 0o700));
    prune(1 << 20);
    assert!(!old.exists(), "the old key scheme is gone");
    assert_eq!(mode(&kept), 0o600);
    assert_eq!(read("https://a/kept").as_deref(), Some(&b"sha era"[..]));
  }

  #[test]
  fn prune_drops_the_oldest_first_past_the_cap() {
    let d = fresh();
    for (i, u) in ["https://a/1", "https://a/2", "https://a/3"].into_iter().enumerate() {
      write(u, &[0; 100]);
      let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + i as u64);
      std::fs::File::options().write(true).open(d.join(key(u))).unwrap().set_modified(t).unwrap();
    }
    prune(250);
    assert!(read("https://a/1").is_none());
    assert!(read("https://a/2").is_some() && read("https://a/3").is_some());
  }
}
