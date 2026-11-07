// new versions: the latest github release, checked now and then (github sees an ip asking, nothing
// else; off with `check for updates`). installing it is one click: the binary is downloaded, its
// ed25519 signature checked against the keys built in here, and it replaces this one in place, then
// the app starts again. a copy the package manager owns (/usr/bin) is left to the package manager.
// the signature (`<asset>.sig2`) covers the release's version, this machine's asset name and the
// binary's sha256, so an old signed build can't pass as a new release, nor another arch's as ours
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

use super::{NetEvent, Out};

const LATEST: &str = "https://api.github.com/repos/mellen9999/heatsync-chat/releases/latest";
pub const PAGE: &str = "https://github.com/mellen9999/heatsync-chat/releases/latest";
// the public halves of the keys a release may be signed with (the private halves never leave the
// release machine). the first signs releases; the second is a spare kept offline, so a lost or leaked
// signing key can be replaced by a release signed with the spare, without stranding installed copies
const KEYS: [[u8; 32]; 2] = [
  [
    0xae, 0x71, 0x6b, 0x0a, 0x24, 0x99, 0x5a, 0x7d, 0x7c, 0xf7, 0x4f, 0x62, 0x46, 0x37, 0x6a, 0xf9, 0x22, 0xa1, 0x68,
    0x92, 0x96, 0x14, 0x32, 0x8b, 0xc3, 0x99, 0xa6, 0x64, 0x97, 0x14, 0x43, 0xe7,
  ],
  [
    0x10, 0x6f, 0xd7, 0x64, 0x59, 0x70, 0x6e, 0x9b, 0x3d, 0xc4, 0xfa, 0x4a, 0xfc, 0x4b, 0x22, 0x90, 0xe2, 0x8e, 0x8b,
    0xaa, 0x00, 0x0f, 0x56, 0x00, 0x73, 0x1c, 0xfc, 0x7e, 0x8e, 0x17, 0xbf, 0x30,
  ],
];
// first line of everything a signature covers: no other signed thing can pass for an update
const DOMAIN: &str = "heatsync-chat update v2\n";
const MAX_BIN: usize = 64 << 20;
const MAX_JSON: usize = 1 << 20;
const MAX_SIG: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
  pub version: Box<str>,
  // this machine's binary and its v2 signature, if the release carries them
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

// the swap, the restart and the names differ by platform
#[cfg_attr(unix, path = "update_unix.rs")]
#[cfg_attr(windows, path = "update_windows.rs")]
mod os;
use os::{asset_name, managed, replace};
pub use os::{restart, tidy};

// github's answer: the tag and this machine's two files. urls must be github's own, https
pub fn parse(v: &Value) -> Option<Release> {
  let tag = v.get("tag_name")?.as_str()?;
  let version = tag.strip_prefix('v').unwrap_or(tag);
  // digits and dots only: it goes into the signed message as written
  if tag.len() > 32 || !newer(tag, "0") || !version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
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
  let bin = url_of(&name).zip(url_of(&format!("{name}.sig2"))).map(|(b, s)| (b.into(), s.into()));
  Some(Release { version: version.into(), bin })
}

// what a signature covers: the header, then version, asset name and the binary's sha256 (hex), a line each
fn signed_message(version: &str, asset: &str, bin: &[u8]) -> Vec<u8> {
  use std::fmt::Write;
  let sum = ring::digest::digest(&ring::digest::SHA256, bin);
  let mut m = format!("{DOMAIN}{version}\n{asset}\n");
  for b in sum.as_ref() {
    let _ = write!(m, "{b:02x}");
  }
  m.push('\n');
  m.into_bytes()
}

// is `sig` a trusted key's signature for exactly this version, asset name and binary
pub fn verify(version: &str, asset: &str, bin: &[u8], sig: &[u8]) -> bool {
  verify_with(&KEYS, version, asset, bin, sig)
}

fn verify_with(keys: &[[u8; 32]], version: &str, asset: &str, bin: &[u8], sig: &[u8]) -> bool {
  // a newline in either would let one message read as another
  if version.contains('\n') || asset.contains('\n') {
    return false;
  }
  let msg = signed_message(version, asset, bin);
  keys.iter().any(|k| ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, k).verify(&msg, sig).is_ok())
}

// the copy can replace itself: its folder is ours to write (not a package manager's /usr/bin)
pub fn self_updates() -> bool {
  exe().and_then(Path::parent).is_some_and(|d| {
    let probe = d.join(".heatsync-chat.probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok && !managed(d)
  })
}

async fn fetch(http: &reqwest::Client, url: &str, max: usize, secs: u64) -> Result<Vec<u8>, String> {
  let res = http.get(url).header("accept", "application/octet-stream").timeout(Duration::from_secs(secs)).send().await;
  super::http::capped(res, max).await
}

pub async fn run(http: reqwest::Client, cmd: Cmd, out: Out) {
  match cmd {
    Cmd::Check => {
      let res = fetch(&http, LATEST, MAX_JSON, 15).await.ok();
      let rel = res.and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| parse(&v));
      if let Some(r) = rel.filter(offered) {
        let _ = out.send(NetEvent::Update(r));
      }
    }
    Cmd::Install(r) => {
      let res = install(&http, &r).await;
      let _ = out.send(NetEvent::Updated(res));
    }
  }
}

// worth a ↑: newer than this one and built for this machine (no dead button where there's no build)
fn offered(r: &Release) -> bool {
  newer(&r.version, current()) && r.bin.is_some()
}

// download, check the signature, swap the file in one rename. Err says what went wrong in plain words
async fn install(http: &reqwest::Client, r: &Release) -> Result<(), String> {
  let (bin, sig) = r.bin.as_ref().ok_or("there is no download for this computer yet")?;
  let exe = exe().ok_or("can't find where this app is installed")?;
  let bin = fetch(http, bin, MAX_BIN, 300).await.map_err(download_failed)?;
  let sig = fetch(http, sig, MAX_SIG, 30).await.map_err(download_failed)?;
  if !verify(&r.version, &asset_name(), &bin, &sig) {
    return Err("the download isn't signed by heatsync: not installed".into());
  }
  // the write + fsync of a big binary blocks: not on the net runtime, where it would stall chat
  off_thread(move || replace(exe, &bin)).await.and_then(|r| r.map_err(replace_failed))
}

// the technical reason goes to stderr, the screen gets plain words
fn download_failed(e: String) -> String {
  eprintln!("update download: {e}");
  "the download didn't work, try again later".into()
}

fn replace_failed(e: impl std::fmt::Display) -> String {
  eprintln!("update replace: {e}");
  "couldn't put the new version in place, nothing was changed".into()
}

// `f` on a blocking thread
async fn off_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
  tokio::task::spawn_blocking(f).await.map_err(replace_failed)
}

#[cfg(test)]
mod tests {
  use super::*;
  use ring::signature::{Ed25519KeyPair, KeyPair};
  use serde_json::json;

  const VERSION: &str = "0.2.1";
  const ASSET: &str = "heatsync-chat-x86_64-linux";
  const BIN: &[u8] = b"heatsync-chat update test";

  fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
  }

  // a throwaway key
  struct Key(Ed25519KeyPair);

  impl Key {
    fn new() -> Key {
      let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
      Key(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap())
    }

    fn public(&self) -> [u8; 32] {
      self.0.public_key().as_ref().try_into().unwrap()
    }

    fn sign(&self, version: &str, asset: &str, bin: &[u8]) -> Vec<u8> {
      self.0.sign(&signed_message(version, asset, bin)).as_ref().to_vec()
    }
  }

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

  fn release(tag: &str, files: &[(String, String)]) -> Value {
    json!({"tag_name": tag, "assets": files.iter().map(|(n, u)| json!({"name": n, "browser_download_url": u})).collect::<Vec<_>>()})
  }

  #[test]
  fn only_github_release_files_are_taken() {
    let name = asset_name();
    let ok = format!("https://github.com/mellen9999/heatsync-chat/releases/download/v0.2.0/{name}");
    let v = release("v0.2.0", &[(name.clone(), ok.clone()), (format!("{name}.sig2"), format!("{ok}.sig2"))]);
    let r = parse(&v).unwrap();
    assert_eq!(&*r.version, "0.2.0");
    assert!(r.bin.is_some());
    let evil = release(
      "v0.2.0",
      &[(name.clone(), "https://evil.example/x".into()), (format!("{name}.sig2"), format!("{ok}.sig2"))],
    );
    assert_eq!(parse(&evil).unwrap().bin, None);
    assert!(parse(&json!({"tag_name": "nightly"})).is_none());
  }

  #[test]
  fn a_release_without_a_v2_signature_has_no_build_for_us() {
    // the old `.sig` (bare binary bytes) is never enough: that is the rollback hole
    let name = asset_name();
    let ok = format!("https://github.com/mellen9999/heatsync-chat/releases/download/v0.2.0/{name}");
    let old = release("v0.2.0", &[(name.clone(), ok.clone()), (format!("{name}.sig"), format!("{ok}.sig"))]);
    assert_eq!(parse(&old).unwrap().bin, None);
    // and a build for some other machine is not ours
    let other = release(
      "v0.2.0",
      &[("heatsync-chat-riscv64-linux".into(), ok.clone()), ("heatsync-chat-riscv64-linux.sig2".into(), ok)],
    );
    assert_eq!(parse(&other).unwrap().bin, None);
  }

  #[test]
  fn only_plain_dotted_numbers_are_versions() {
    for bad in ["v+1.2.3", "v1.2.3\n", "v1.2.3 ", "v1. 2", "vv1.2.3", "1.2.3-rc1"] {
      assert!(parse(&json!({"tag_name": bad, "assets": []})).is_none(), "{bad:?}");
    }
    assert_eq!(&*parse(&json!({"tag_name": "1.2.3", "assets": []})).unwrap().version, "1.2.3");
  }

  #[test]
  fn no_button_without_a_build_for_this_machine() {
    let name = asset_name();
    let u = format!("https://github.com/mellen9999/heatsync-chat/releases/download/v99.0.0/{name}");
    let built =
      parse(&release("v99.0.0", &[(name.clone(), u.clone()), (format!("{name}.sig2"), format!("{u}.sig2"))])).unwrap();
    assert!(offered(&built));
    assert!(!offered(&Release { bin: None, ..built.clone() }), "no asset for this machine: no ↑");
    assert!(!offered(&Release { version: "0.0.1".into(), ..built }), "older is not offered");
  }

  #[test]
  fn the_message_is_header_version_asset_and_hash() {
    assert_eq!(
      String::from_utf8(signed_message(VERSION, ASSET, b"")).unwrap(),
      "heatsync-chat update v2\n0.2.1\nheatsync-chat-x86_64-linux\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n"
    );
  }

  #[test]
  fn a_signature_round_trips_and_binds_version_asset_and_binary() {
    let key = Key::new();
    let keys = [key.public()];
    let sig = key.sign(VERSION, ASSET, BIN);
    assert!(verify_with(&keys, VERSION, ASSET, BIN, &sig));
    assert!(!verify_with(&keys, "0.2.0", ASSET, BIN, &sig), "an older version's signature is not this release's");
    assert!(!verify_with(&keys, "0.2.10", ASSET, BIN, &sig));
    assert!(!verify_with(&keys, VERSION, "heatsync-chat-aarch64-linux", BIN, &sig), "another arch's build is not ours");
    assert!(!verify_with(&keys, VERSION, ASSET, b"heatsync-chat update tesT", &sig), "one byte off fails");
    assert!(!verify_with(&keys, VERSION, ASSET, b"", &sig));
  }

  #[test]
  fn the_old_bare_binary_signature_is_refused() {
    // v1 signed just the binary bytes: that exact signature must not pass as a release
    let key = Key::new();
    let v1 = key.0.sign(BIN);
    assert!(!verify_with(&[key.public()], VERSION, ASSET, BIN, v1.as_ref()));
  }

  #[test]
  fn either_trusted_key_signs_and_a_stranger_does_not() {
    let (a, b, stranger) = (Key::new(), Key::new(), Key::new());
    let keys = [a.public(), b.public()];
    assert!(verify_with(&keys, VERSION, ASSET, BIN, &a.sign(VERSION, ASSET, BIN)));
    assert!(verify_with(&keys, VERSION, ASSET, BIN, &b.sign(VERSION, ASSET, BIN)));
    assert!(!verify_with(&keys, VERSION, ASSET, BIN, &stranger.sign(VERSION, ASSET, BIN)));
    assert!(!verify_with(&[], VERSION, ASSET, BIN, &a.sign(VERSION, ASSET, BIN)), "no keys trusts nothing");
  }

  #[test]
  fn a_newline_cannot_shift_the_fields() {
    // genuinely signed messages, but "a\nb" + "c" and "a" + "b\nc" are the same bytes, so neither is accepted
    let key = Key::new();
    let keys = [key.public()];
    assert!(!verify_with(&keys, "0.2.1\nx", "y", BIN, &key.sign("0.2.1\nx", "y", BIN)));
    assert!(!verify_with(&keys, "0.2.1", "x\ny", BIN, &key.sign("0.2.1", "x\ny", BIN)));
  }

  #[test]
  fn the_built_in_keys_check_what_release_sh_signs() {
    // made by packaging/release.sh's signing step (printf the message, openssl pkeyutl -sign -rawin),
    // once with each key, over version 0.2.1, asset heatsync-chat-x86_64-linux and the bytes in BIN
    let sign_pem = unhex(
      "5f8b9dbba5acacfce878a4222ab661a96c0447d59e8f0d63cb16f26221971e23a47c99a3a57d89d1fe037d468a240a9fecd6bf117b3f2086724662b36c8e3b0b",
    );
    let backup_pem = unhex(
      "92c3121c6952797ceea203750dd7a63f849a9d48dfe3b69ccb936baa97958c91cf4f9cb708406f6a808ebe382766444f104122951d85ac7e0cdf3ab103217a08",
    );
    for (sig, key) in [(&sign_pem, &KEYS[0]), (&backup_pem, &KEYS[1])] {
      assert!(verify(VERSION, ASSET, BIN, sig));
      assert!(verify_with(&[*key], VERSION, ASSET, BIN, sig), "each signature belongs to its own key");
      assert!(!verify("0.2.2", ASSET, BIN, sig));
      assert!(!verify(VERSION, "heatsync-chat-aarch64-linux", BIN, sig));
      assert!(!verify(VERSION, ASSET, b"heatsync-chat update tesT", sig));
    }
    assert!(!verify_with(&[KEYS[1]], VERSION, ASSET, BIN, &sign_pem), "the keys are different keys");
  }

  #[test]
  fn a_bad_signature_is_refused() {
    assert!(!verify(VERSION, ASSET, BIN, &[0u8; 64]));
    assert!(!verify(VERSION, ASSET, BIN, b"short"));
    assert!(!verify(VERSION, ASSET, BIN, b""));
  }

  // the install's file work runs on a blocking thread, so the net runtime keeps serving chat meanwhile
  #[tokio::test(flavor = "current_thread")]
  async fn the_install_write_leaves_the_runtime_thread() {
    let here = std::thread::current().id();
    let there = off_thread(|| std::thread::current().id()).await.unwrap();
    assert_ne!(here, there);
    let d = std::env::temp_dir().join(format!("hsc-off-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let exe: &'static Path = Box::leak(d.join("app").into_boxed_path());
    std::fs::write(exe, b"old bytes").unwrap();
    off_thread(move || replace(exe, b"new bytes")).await.unwrap().unwrap();
    assert_eq!(std::fs::read(exe).unwrap(), b"new bytes");
    let _ = std::fs::remove_dir_all(&d);
  }
}
