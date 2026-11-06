// image/video upload to heatsync (the ext's attach button): pick or drop a file, validate it by its
// bytes, post it with the session token, hand back the absolute url. runs on the net thread
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;

use super::{NetEvent, ORIGIN, Out};
use crate::images::url::allowed;

mod strip;

// the ext's own client caps (the server's are 10MB / 50MB)
pub const MAX_IMG: u64 = 5 << 20;
pub const MAX_VID: u64 = 45 << 20;
const MAX_REPLY: usize = 64 << 10;
const TIMEOUT: Duration = Duration::from_secs(120);
const KINDS: &str = "that isn't a picture or video heatsync takes (png jpg gif webp avif bmp tiff mp4 webm mov)";

pub enum Upload {
  Url(String),
  Failed(String),
  // the file dialog was closed
  Cancelled,
  // no zenity/kdialog: the app asks for a path in the input bar instead
  NoChooser,
  // a tab-completion answered: for what was typed (`input`) and the ask it belongs to (`seq`)
  Completed { seq: u64, input: String, text: String, many: Vec<String> },
}

// listing a folder can hang (a slow or dead network mount): it runs on a blocking thread and the app
// hears back as an event, like the clipboard
pub fn complete(seq: u64, input: String, home: Option<String>, out: Out) {
  tokio::task::spawn_blocking(move || {
    let (text, many) = crate::model::pathcomp::complete(&input, home.as_deref());
    out.send(NetEvent::Upload(Upload::Completed { seq, input, text, many }));
  });
}

#[derive(Debug, PartialEq)]
pub struct Media {
  pub mime: &'static str,
  pub ext: &'static str,
  pub video: bool,
}

const fn img(mime: &'static str, ext: &'static str) -> Media {
  Media { mime, ext, video: false }
}

const fn vid(mime: &'static str, ext: &'static str) -> Media {
  Media { mime, ext, video: true }
}

// what the bytes really are, never what the name claims
pub fn sniff(b: &[u8]) -> Result<Media, String> {
  let bad = || Err(KINDS.to_string());
  if b.starts_with(&[0xff, 0xd8, 0xff]) {
    return Ok(img("image/jpeg", "jpg"));
  }
  if b.starts_with(b"\x89PNG\r\n\x1a\n") {
    if animated_png(b) {
      return Err("animated png isn't supported, turn it into a gif or webp first".into());
    }
    return Ok(img("image/png", "png"));
  }
  if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
    return Ok(img("image/gif", "gif"));
  }
  if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
    return Ok(img("image/webp", "webp"));
  }
  if b.len() >= 26 && b.starts_with(b"BM") && b[6..10] == [0; 4] {
    return Ok(img("image/bmp", "bmp"));
  }
  if b.starts_with(&[0x49, 0x49, 0x2a, 0x00]) || b.starts_with(&[0x4d, 0x4d, 0x00, 0x2a]) {
    return Ok(img("image/tiff", "tiff"));
  }
  if b.len() >= 12 && &b[4..8] == b"ftyp" {
    return match &b[8..12] {
      b"avif" | b"avis" => Ok(img("image/avif", "avif")),
      b"qt  " => Ok(vid("video/quicktime", "mov")),
      b"heic" | b"heix" | b"heif" | b"mif1" | b"msf1" | b"hevc" => bad(),
      _ => Ok(vid("video/mp4", "mp4")),
    };
  }
  if b.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) && b[..b.len().min(64)].windows(4).any(|w| w == b"webm") {
    return Ok(vid("video/webm", "webm"));
  }
  bad()
}

// an acTL chunk before the first IDAT makes a png animated
fn animated_png(b: &[u8]) -> bool {
  let mut i = 8;
  while i + 8 <= b.len() {
    let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
    match &b[i + 4..i + 8] {
      b"acTL" => return true,
      b"IDAT" => return false,
      _ => {}
    }
    i = i.saturating_add(12).saturating_add(len);
  }
  false
}

pub fn too_big(m: &Media, len: u64) -> Option<String> {
  let (cap, what) = if m.video { (MAX_VID, "videos") } else { (MAX_IMG, "pictures") };
  (len > cap).then(|| format!("too big ({}MB max for {what})", cap >> 20))
}

// a file the user can name: ~/ expanded, quotes from a drag or paste stripped
pub fn expand(s: &str, home: Option<&str>) -> PathBuf {
  let s = s.trim();
  let s = s.strip_prefix("file://").unwrap_or(s);
  let s = ["\"", "'"].iter().find_map(|q| s.strip_prefix(q)?.strip_suffix(q)).unwrap_or(s);
  match (s.strip_prefix("~/"), home) {
    (Some(rest), Some(h)) => Path::new(h).join(rest),
    _ => PathBuf::from(s),
  }
}

// read the file (bounded) off the async threads, then check it by its bytes
fn load(path: &Path) -> Result<(Vec<u8>, Media), String> {
  let meta = std::fs::metadata(path).map_err(|e| format!("can't open that file ({})", e.kind()))?;
  if !meta.is_file() {
    return Err("that's not a file".into());
  }
  if meta.len() == 0 {
    return Err("that file is empty".into());
  }
  if meta.len() > MAX_VID {
    return Err(format!("too big ({}MB max)", MAX_VID >> 20));
  }
  let mut buf = Vec::with_capacity(meta.len() as usize);
  let f = std::fs::File::open(path).map_err(|e| format!("can't open that file ({})", e.kind()))?;
  f.take(MAX_VID + 1).read_to_end(&mut buf).map_err(|e| format!("can't read that file ({})", e.kind()))?;
  checked(buf, true)
}

// what the bytes are, by the bytes, within the size cap, with gps, camera, comments and the rest taken
// off (never bigger than what came in). a picture from the clipboard is no video
fn checked(buf: Vec<u8>, video_ok: bool) -> Result<(Vec<u8>, Media), String> {
  let m = sniff(&buf)?;
  if m.video && !video_ok {
    return Err("that isn't a picture".into());
  }
  if let Some(e) = too_big(&m, buf.len() as u64) {
    return Err(e);
  }
  Ok((strip::clean(m.mime, buf)?, m))
}

fn multipart(boundary: &str, m: &Media, bytes: &[u8]) -> Vec<u8> {
  // the local file name stays local: the server only needs the extension
  let head = format!(
    "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"upload.{}\"\r\nContent-Type: {}\r\n\r\n",
    m.ext, m.mime
  );
  let mut body = Vec::with_capacity(bytes.len() + head.len() + boundary.len() + 8);
  body.extend_from_slice(head.as_bytes());
  body.extend_from_slice(bytes);
  body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
  body
}

fn boundary() -> String {
  use ring::rand::{SecureRandom, SystemRandom};
  let mut r = [0u8; 12];
  let _ = SystemRandom::new().fill(&mut r);
  r.iter().fold(String::from("hsc"), |mut s, b| {
    s.push_str(&format!("{b:02x}"));
    s
  })
}

// the server answers `/uploads/x`; chat needs it absolute, and it must stay on our own host
pub fn absolute(u: &str) -> Option<String> {
  let full = if u.starts_with('/') && !u.starts_with("//") { format!("{ORIGIN}{u}") } else { u.to_owned() };
  allowed(&full).then_some(full)
}

fn why(code: u16, v: &Value) -> String {
  match code {
    401 | 403 => "log in to upload (ctrl-l)".into(),
    413 => "too big for heatsync".into(),
    429 => "slow down, try again in a minute".into(),
    _ => v.get("error").and_then(Value::as_str).map_or_else(
      || format!("heatsync said http {code}"),
      |e| e.chars().filter(|c| !c.is_control()).take(100).collect(),
    ),
  }
}

async fn post(http: &reqwest::Client, token: &str, path: PathBuf) -> Result<String, String> {
  let p = path.clone();
  let (bytes, m) = tokio::task::spawn_blocking(move || load(&p)).await.map_err(|e| e.to_string())??;
  send(http, token, bytes, m).await
}

async fn post_picture(http: &reqwest::Client, token: &str, bytes: Vec<u8>) -> Result<String, String> {
  let (bytes, m) = tokio::task::spawn_blocking(move || checked(bytes, false)).await.map_err(|e| e.to_string())??;
  send(http, token, bytes, m).await
}

async fn send(http: &reqwest::Client, token: &str, bytes: Vec<u8>, m: Media) -> Result<String, String> {
  let b = boundary();
  let body = multipart(&b, &m, &bytes);
  drop(bytes);
  let mut r = http
    .post(format!("{ORIGIN}/api/upload"))
    .bearer_auth(token)
    .header("content-type", format!("multipart/form-data; boundary={b}"))
    .timeout(TIMEOUT)
    .body(body)
    .send()
    .await
    .map_err(
      |e| if e.is_timeout() { "heatsync took too long".to_string() } else { "couldn't reach heatsync".into() },
    )?;
  let code = r.status().as_u16();
  let mut buf = Vec::new();
  while let Some(c) = r.chunk().await.map_err(|_| "the connection dropped".to_string())? {
    if buf.len() + c.len() > MAX_REPLY {
      return Err("heatsync sent a reply that's too big".into());
    }
    buf.extend_from_slice(&c);
  }
  let v: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
  if !(200..300).contains(&code) {
    return Err(why(code, &v));
  }
  v.get("url")
    .and_then(Value::as_str)
    .and_then(absolute)
    .ok_or_else(|| "heatsync sent back a link i don't trust".into())
}

pub async fn run_picture(http: reqwest::Client, token: String, bytes: Vec<u8>, out: Out) {
  let ev = match post_picture(&http, &token, bytes).await {
    Ok(u) => Upload::Url(u),
    Err(e) => Upload::Failed(e),
  };
  out.send(NetEvent::Upload(ev));
}

pub async fn run(http: reqwest::Client, token: String, path: PathBuf, out: Out) {
  let ev = match post(&http, &token, path).await {
    Ok(u) => Upload::Url(u),
    Err(e) => Upload::Failed(e),
  };
  out.send(NetEvent::Upload(ev));
}

// first line of a dialog's stdout, if it is an absolute path
pub fn parse_pick(stdout: &str) -> Option<PathBuf> {
  let l = stdout.lines().next()?.trim();
  (l.starts_with('/') && !l.contains('\0')).then(|| PathBuf::from(l))
}

enum Ran {
  Picked(PathBuf),
  Closed,
  Missing,
}

fn run_chooser(prog: &str, args: &[&str]) -> Ran {
  let child = Command::new(prog).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).output();
  match child {
    Err(_) => Ran::Missing,
    Ok(o) if o.status.success() => parse_pick(&String::from_utf8_lossy(&o.stdout)).map_or(Ran::Closed, Ran::Picked),
    // 1 = the user closed it; anything else = it couldn't open (no display, broken): try the next
    Ok(o) if o.status.code() == Some(1) => Ran::Closed,
    Ok(_) => Ran::Missing,
  }
}

// zenity, then kdialog. blocks until the dialog closes
pub fn choose() -> Upload {
  let tries: [(&str, &[&str]); 2] = [
    ("zenity", &["--file-selection", "--title=upload a picture or video"]),
    ("kdialog", &["--getopenfilename", ".", "--title", "upload a picture or video"]),
  ];
  for (p, a) in tries {
    match run_chooser(p, a) {
      Ran::Picked(path) => return Upload::Url(path.to_string_lossy().into_owned()),
      Ran::Closed => return Upload::Cancelled,
      Ran::Missing => {}
    }
  }
  Upload::NoChooser
}

#[cfg(test)]
mod tests {
  use super::*;

  // a real 2x1 png, as a screenshot tool would put on the clipboard
  fn tiny_png() -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, 2, 1);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&[255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
    out
  }

  #[test]
  fn a_clipboard_picture_is_checked_like_a_file() {
    let (bytes, m) = checked(tiny_png(), false).unwrap();
    assert_eq!(m.mime, "image/png");
    assert_eq!(sniff(&bytes).unwrap().mime, "image/png");
    assert!(checked(b"just some text".to_vec(), false).is_err());
    let mut big = vec![0xff, 0xd8, 0xff];
    big.resize(MAX_IMG as usize + 1, 0);
    assert!(checked(big, false).unwrap_err().starts_with("too big"));
  }

  #[test]
  fn a_clipboard_picture_is_never_a_video() {
    let mp4 = b"\0\0\0\x14ftypisom\0\0\0\0isom".to_vec();
    assert_eq!(checked(mp4.clone(), false).unwrap_err(), "that isn't a picture");
    assert!(checked(mp4, true).unwrap().1.video, "a file can be");
  }

  fn png(extra: &[u8]) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend_from_slice(&[0, 0, 0, 13]);
    v.extend_from_slice(b"IHDR");
    v.extend_from_slice(&[0; 17]);
    v.extend_from_slice(extra);
    v
  }

  #[test]
  fn sniffs_by_bytes() {
    assert_eq!(sniff(b"\xff\xd8\xff\xe0abc").unwrap().mime, "image/jpeg");
    assert_eq!(sniff(b"GIF89a....").unwrap().ext, "gif");
    assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 ").unwrap().mime, "image/webp");
    assert_eq!(sniff(&png(&[0, 0, 0, 0, b'I', b'D', b'A', b'T'])).unwrap().mime, "image/png");
    assert_eq!(sniff(b"\0\0\0\x18ftypavif\0\0").unwrap().mime, "image/avif");
    let mp4 = sniff(b"\0\0\0\x18ftypisom\0\0\0\0").unwrap();
    assert!(mp4.video && mp4.mime == "video/mp4");
    assert_eq!(sniff(b"\0\0\0\x14ftypqt  \0\0\0\0").unwrap().mime, "video/quicktime");
    assert_eq!(sniff(b"\x1a\x45\xdf\xa3\x01\0\0\0\x42\x82webm").unwrap().mime, "video/webm");
  }

  #[test]
  fn refuses_the_rest() {
    assert!(sniff(b"<svg xmlns=...>").is_err());
    assert!(sniff(b"#!/bin/sh\necho hi").is_err());
    assert!(sniff(b"").is_err());
    assert!(sniff(b"\0\0\0\x18ftypheic\0\0").is_err());
    assert!(sniff(b"\x1a\x45\xdf\xa3matroska").is_err());
    let apng = png(&[0, 0, 0, 8, b'a', b'c', b'T', b'L', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(sniff(&apng).unwrap_err().contains("animated"));
  }

  #[test]
  fn size_caps_follow_the_kind() {
    let (i, v) = (img("image/png", "png"), vid("video/mp4", "mp4"));
    assert!(too_big(&i, MAX_IMG).is_none());
    assert!(too_big(&i, MAX_IMG + 1).unwrap().contains("5MB"));
    assert!(too_big(&v, MAX_IMG + 1).is_none());
    assert!(too_big(&v, MAX_VID + 1).unwrap().contains("45MB"));
  }

  #[test]
  fn paths_are_cleaned() {
    assert_eq!(expand("  ~/a b.png ", Some("/home/x")), PathBuf::from("/home/x/a b.png"));
    assert_eq!(expand("'/tmp/a.png'", None), PathBuf::from("/tmp/a.png"));
    assert_eq!(expand("file:///tmp/a.png", None), PathBuf::from("/tmp/a.png"));
    assert_eq!(expand("~/a", None), PathBuf::from("~/a"));
  }

  #[test]
  fn urls_stay_on_our_host() {
    assert_eq!(absolute("/uploads/a.webp").as_deref(), Some("https://heatsync.org/uploads/a.webp"));
    assert!(absolute("//evil.com/x").is_none());
    assert!(absolute("https://evil.com/uploads/a.webp").is_none());
    assert!(absolute("http://heatsync.org/uploads/a").is_none());
    assert!(absolute("/uploads/a b").is_none());
    assert!(absolute("javascript:alert(1)").is_none());
  }

  #[test]
  fn body_shape() {
    let m = img("image/png", "png");
    let b = multipart("BND", &m, b"DATA");
    let s = String::from_utf8(b).unwrap();
    assert!(s.starts_with("--BND\r\n"));
    assert!(s.contains("filename=\"upload.png\""));
    assert!(s.contains("Content-Type: image/png\r\n\r\nDATA\r\n--BND--\r\n"));
  }

  #[test]
  fn dialog_output() {
    assert_eq!(parse_pick("/tmp/a.png\n"), Some(PathBuf::from("/tmp/a.png")));
    assert_eq!(parse_pick("rel.png\n"), None);
    assert_eq!(parse_pick(""), None);
  }

  #[test]
  fn load_checks_bytes_not_names() {
    let d = std::env::temp_dir().join(format!("hsc-up-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let fake = d.join("cat.png");
    std::fs::write(&fake, b"not a picture at all").unwrap();
    assert!(load(&fake).is_err());
    let real = d.join("x.bin");
    // a one-pixel gif with a comment in it: the comment does not leave
    let mut gif = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff\x21\xfe\x07SECRET!\0".to_vec();
    gif.extend_from_slice(b"\x21\xf9\x04\x01\0\0\0\0\x2c\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0\x3b");
    std::fs::write(&real, &gif).unwrap();
    let (sent, kind) = load(&real).unwrap();
    assert_eq!(kind.mime, "image/gif");
    assert!(!sent.windows(6).any(|w| w == b"SECRET") && sent.len() < gif.len());
    assert!(crate::images::decode::decode(&sent, 8).is_ok());
    // a picture that doesn't parse is not sent as is
    std::fs::write(&real, b"GIF89a\x01\0\x01\0").unwrap();
    assert!(load(&real).unwrap_err().contains("damaged"));
    assert!(load(&d).unwrap_err().contains("not a file"));
    std::fs::write(d.join("e"), b"").unwrap();
    assert!(load(&d.join("e")).unwrap_err().contains("empty"));
    let _ = std::fs::remove_dir_all(&d);
  }

  // the folder is read on a blocking thread and the answer comes back as an event
  #[tokio::test]
  async fn a_completion_comes_back_as_an_event_with_its_ask_number() {
    let d = std::env::temp_dir().join(format!("hsc-cmp-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("cat1.png"), b"x").unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let input = format!("{}/ca", d.display());
    complete(7, input.clone(), None, Out::Pipe(tx));
    let ev =
      tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(5))).await.unwrap().expect("an answer");
    let _ = std::fs::remove_dir_all(&d);
    match ev {
      NetEvent::Upload(Upload::Completed { seq: 7, input: i, text, many }) => {
        assert_eq!((i, text), (input.clone(), format!("{}/cat1.png", d.display())));
        assert!(many.is_empty());
      }
      _ => panic!("not a completion"),
    }
  }
}
