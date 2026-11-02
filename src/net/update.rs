// new versions: the latest github release, checked now and then (github sees an ip asking, nothing
// else; off with `check for updates`). installing it is one click: the binary is downloaded, its
// ed25519 signature checked against the key built in here, and it replaces this one in place, then
// the app starts again. a copy the package manager owns (/usr/bin) is left to the package manager
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

use super::{NetEvent, Out};

const LATEST: &str = "https://api.github.com/repos/mellen9999/heatsync-chat/releases/latest";
pub const PAGE: &str = "https://github.com/mellen9999/heatsync-chat/releases/latest";
// the release signing key's public half (the private half never leaves the release machine)
const KEY: [u8; 32] = [
  0xae, 0x71, 0x6b, 0x0a, 0x24, 0x99, 0x5a, 0x7d, 0x7c, 0xf7, 0x4f, 0x62, 0x46, 0x37, 0x6a, 0xf9, 0x22, 0xa1, 0x68,
  0x92, 0x96, 0x14, 0x32, 0x8b, 0xc3, 0x99, 0xa6, 0x64, 0x97, 0x14, 0x43, 0xe7,
];
const MAX_BIN: usize = 64 << 20;
const MAX_JSON: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
  pub version: Box<str>,
  // this machine's binary and its signature, if the release carries them
  bin: Option<(Box<str>, Box<str>)>,
}

pub enum Cmd {
  Check,
  Install(Release),
}

// the running binary's path, read before anything replaces it (after, /proc names the old one deleted)
pub fn exe() -> Option<&'static Path> {
  static EXE: OnceLock<Option<PathBuf>> = OnceLock::new();
  EXE.get_or_init(|| std::env::current_exe().ok()).as_deref()
}

pub fn current() -> &'static str {
  env!("CARGO_PKG_VERSION")
}

// "v0.2.10" > "0.2.9"; anything unreadable is never newer
pub fn newer(tag: &str, than: &str) -> bool {
  let parse = |s: &str| -> Option<Vec<u64>> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let v: Option<Vec<u64>> = s.split('.').map(|p| p.parse().ok()).collect();
    v.filter(|v| (1..=4).contains(&v.len()))
  };
  matches!((parse(tag), parse(than)), (Some(a), Some(b)) if a > b)
}

fn asset_name() -> String {
  format!("heatsync-chat-{}-linux", std::env::consts::ARCH)
}

// github's answer: the tag and this machine's two files. urls must be github's own, https
pub fn parse(v: &Value) -> Option<Release> {
  let tag = v.get("tag_name")?.as_str()?;
  if tag.len() > 32 || !newer(tag, "0") {
    return None;
  }
  let url_of = |name: &str| {
    v.get("assets")?.as_array()?.iter().find_map(|a| {
      (a.get("name")?.as_str()? == name)
        .then(|| a.get("browser_download_url")?.as_str())?
        .filter(|u| u.len() < 512 && u.starts_with("https://github.com/mellen9999/heatsync-chat/releases/download/"))
    })
  };
  let name = asset_name();
  let bin = url_of(&name).zip(url_of(&format!("{name}.sig"))).map(|(b, s)| (b.into(), s.into()));
  Some(Release { version: tag.trim_start_matches('v').into(), bin })
}

pub fn verify(bin: &[u8], sig: &[u8]) -> bool {
  ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, KEY).verify(bin, sig).is_ok()
}

// the copy can replace itself: its folder is ours to write (not a package manager's /usr/bin)
pub fn self_updates() -> bool {
  exe().and_then(Path::parent).is_some_and(|d| {
    let probe = d.join(".heatsync-chat.probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok && !d.starts_with("/usr")
  })
}

async fn fetch(http: &reqwest::Client, url: &str, max: usize, secs: u64) -> Result<Vec<u8>, String> {
  let mut r = http
    .get(url)
    .header("accept", "application/octet-stream")
    .timeout(Duration::from_secs(secs))
    .send()
    .await
    .map_err(|e| e.to_string())?;
  if !r.status().is_success() {
    return Err(format!("http {}", r.status().as_u16()));
  }
  let mut buf = Vec::new();
  while let Some(c) = r.chunk().await.map_err(|e| e.to_string())? {
    if buf.len() + c.len() > max {
      return Err("too big".into());
    }
    buf.extend_from_slice(&c);
  }
  Ok(buf)
}

pub async fn run(http: reqwest::Client, cmd: Cmd, out: Out) {
  match cmd {
    Cmd::Check => {
      let res = fetch(&http, LATEST, MAX_JSON, 15).await.ok();
      let rel = res.and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| parse(&v));
      if let Some(r) = rel.filter(|r| newer(&r.version, current())) {
        let _ = out.send(NetEvent::Update(r));
      }
    }
    Cmd::Install(r) => {
      let res = install(&http, &r).await;
      let _ = out.send(NetEvent::Updated(res));
    }
  }
}

// download, check the signature, swap the file in one rename. Err says what went wrong in plain words
async fn install(http: &reqwest::Client, r: &Release) -> Result<(), String> {
  let (bin, sig) = r.bin.as_ref().ok_or("no build for this machine in that release")?;
  let exe = exe().ok_or("can't tell where this app lives")?;
  let bin = fetch(http, bin, MAX_BIN, 300).await.map_err(|e| format!("download failed ({e})"))?;
  let sig = fetch(http, sig, 256, 30).await.map_err(|e| format!("download failed ({e})"))?;
  if !verify(&bin, &sig) {
    return Err("the download isn't signed by heatsync: not installed".into());
  }
  let tmp = exe.with_file_name(".heatsync-chat.new");
  let write = || -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(&tmp, &bin)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&tmp, exe)
  };
  write().map_err(|e| {
    let _ = std::fs::remove_file(&tmp);
    format!("couldn't replace the app ({e})")
  })
}

// start the new copy in this one's place, same arguments
pub fn restart() -> std::io::Error {
  use std::os::unix::process::CommandExt;
  match exe() {
    Some(p) => std::process::Command::new(p).args(std::env::args_os().skip(1)).exec(),
    None => std::io::Error::other("no path"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn versions_compare_by_number() {
    assert!(newer("v0.2.0", "0.1.0"));
    assert!(newer("0.10.0", "0.9.9"));
    assert!(!newer("v0.1.0", "0.1.0"));
    assert!(!newer("v0.1", "0.1.0"), "shorter is not newer");
    for bad in ["", "v", "latest", "1.x", "v1.2.3.4.5", "-1"] {
      assert!(!newer(bad, "0.0.1"), "{bad}");
    }
  }

  #[test]
  fn only_github_release_files_are_taken() {
    let name = asset_name();
    let ok = format!("https://github.com/mellen9999/heatsync-chat/releases/download/v0.2.0/{name}");
    let v = json!({"tag_name": "v0.2.0", "assets": [
      {"name": name, "browser_download_url": ok},
      {"name": format!("{name}.sig"), "browser_download_url": format!("{ok}.sig")},
    ]});
    let r = parse(&v).unwrap();
    assert_eq!(&*r.version, "0.2.0");
    assert!(r.bin.is_some());
    let evil = json!({"tag_name": "v0.2.0", "assets": [
      {"name": name, "browser_download_url": "https://evil.example/x"},
      {"name": format!("{name}.sig"), "browser_download_url": format!("{ok}.sig")},
    ]});
    assert_eq!(parse(&evil).unwrap().bin, None);
    assert!(parse(&json!({"tag_name": "nightly"})).is_none());
  }

  #[test]
  fn the_release_key_checks_its_own_signature() {
    // signed once with the release key (openssl pkeyutl -sign -rawin)
    let sig = [
      0x75, 0x04, 0xee, 0x0b, 0x25, 0x93, 0x91, 0xf9, 0xea, 0x42, 0x6b, 0x6a, 0xea, 0x45, 0xb1, 0xbe, 0x8a, 0x32, 0xa5,
      0x44, 0xda, 0x34, 0x32, 0x18, 0xb2, 0x1f, 0xc7, 0xd6, 0x92, 0x96, 0x18, 0xeb, 0x3e, 0x34, 0xe3, 0x57, 0x0f, 0x59,
      0x57, 0x63, 0xa3, 0xb3, 0x0b, 0x00, 0xe7, 0x9b, 0x80, 0x47, 0xf7, 0x6a, 0x7f, 0x09, 0x78, 0xe0, 0x57, 0xad, 0xe7,
      0x4a, 0x5a, 0x6c, 0x21, 0xe9, 0x48, 0x08,
    ];
    assert!(verify(b"heatsync-chat update test", &sig));
    assert!(!verify(b"heatsync-chat update tesT", &sig), "one byte off fails");
  }

  #[test]
  fn a_bad_signature_is_refused() {
    assert!(!verify(b"binary", &[0u8; 64]));
    assert!(!verify(b"binary", b"short"));
  }
}
