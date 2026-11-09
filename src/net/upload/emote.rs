// upload your own emote (the ext's picker upload): the file goes up as `type=emote` (the server makes the
// emote picture and its smaller copies), then the add puts it in your set under the name you gave it.
// two calls, like the site makes. a name that is taken keeps the upload (`Kept`) so another name is one call
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Media, NetEvent, ORIGIN, Out, Upload, absolute, send_form, sniff, strip, why};
use crate::heatsync::valid_name;

// the server's own image cap (the chat limit is lower)
pub const MAX_EMOTE: u64 = 10 << 20;
// the server's bounds on the size it is told
const MAX_DIM: u64 = 8192;
const MAX_REPLY: usize = 64 << 10;
// the pictures the ext's upload takes
const TYPES: [&str; 5] = ["image/png", "image/gif", "image/webp", "image/jpeg", "image/avif"];
const NOT_EMOTE: &str = "an emote has to be a png, gif, webp, jpg or avif picture";

// an upload the server holds, waiting for a name that works
#[derive(Clone, Debug, PartialEq)]
pub struct Kept {
  pub url: String,
  pub phash: Option<String>,
  pub w: u16,
  pub h: u16,
  // what the scan flagged but still hosted: a content warning, not a block
  pub flagged: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct Done {
  pub name: String,
  pub kept: Kept,
}

#[derive(Debug, PartialEq)]
pub struct Failed {
  pub why: String,
  // the file is up but the name did not work: ask for another name
  pub kept: Option<Kept>,
}

// the default name for a file: its stem (no extension, no `-2x` size tail) cut to letters, digits and `_`
pub fn default_name(path: &Path) -> String {
  let file = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
  let stem = file.rsplit_once('.').map_or(file.as_str(), |(head, _)| head).to_owned();
  let stem = match stem.rsplit_once('-') {
    Some((head, tail)) if matches!(tail.to_ascii_lowercase().as_str(), "1x" | "2x" | "3x" | "4x") => head.to_owned(),
    _ => stem,
  };
  let name: String = stem.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(30).collect();
  if name.is_empty() { "emote".into() } else { name }
}

// why a name won't do, or none. the word you type in chat
pub fn name_problem(name: &str) -> Option<&'static str> {
  if name.is_empty() {
    Some("type a name")
  } else if !valid_name(name) {
    Some("a name is one word: no spaces, and it can't start with / or .")
  } else {
    None
  }
}

// read the file (bounded) off the async threads: a picture by its bytes, never a video, within the server's cap
fn load(path: &Path) -> Result<(Vec<u8>, Media), String> {
  let meta = std::fs::metadata(path).map_err(|e| format!("can't open that file ({})", e.kind()))?;
  if !meta.is_file() {
    return Err("that's not a file".into());
  }
  if meta.len() == 0 {
    return Err("that file is empty".into());
  }
  if meta.len() > MAX_EMOTE {
    return Err(format!("too big ({}MB max)", MAX_EMOTE >> 20));
  }
  let mut buf = Vec::with_capacity(meta.len() as usize);
  let f = std::fs::File::open(path).map_err(|e| format!("can't open that file ({})", e.kind()))?;
  f.take(MAX_EMOTE + 1).read_to_end(&mut buf).map_err(|e| format!("can't read that file ({})", e.kind()))?;
  checked(buf)
}

fn checked(buf: Vec<u8>) -> Result<(Vec<u8>, Media), String> {
  let m = sniff(&buf)?;
  if !TYPES.contains(&m.mime) {
    return Err(NOT_EMOTE.into());
  }
  if buf.len() as u64 > MAX_EMOTE {
    return Err(format!("too big ({}MB max)", MAX_EMOTE >> 20));
  }
  Ok((strip::clean(m.mime, buf)?, m))
}

// the upload's answer, hostile: a url on our host, a short phash, sizes the server would take, a few flags
pub fn kept(v: &Value) -> Option<Kept> {
  let url = absolute(v.get("url")?.as_str()?)?;
  let phash = v.get("phash").and_then(Value::as_str).filter(|p| !p.is_empty() && p.len() <= 512).map(str::to_owned);
  let dim = |k: &str| v.get(k).and_then(Value::as_u64).filter(|n| (1..=MAX_DIM).contains(n)).map_or(0, |n| n as u16);
  let flagged = v
    .pointer("/moderationResult/flaggedCategories")
    .and_then(Value::as_array)
    .map(|a| {
      a.iter()
        .filter_map(Value::as_str)
        .map(|s| s.chars().filter(|c| !c.is_control()).take(32).collect::<String>())
        .filter(|s| !s.is_empty())
        .take(8)
        .collect()
    })
    .unwrap_or_default();
  Some(Kept { url, phash, w: dim("width"), h: dim("height"), flagged })
}

fn add_body(name: &str, k: &Kept) -> Value {
  let mut b = json!({
    "emoteUrl": k.url, "emoteName": name, "customName": name,
    "source": "upload", "sourceId": null, "zeroWidth": false,
  });
  if let Some(p) = &k.phash {
    b["phash"] = Value::String(p.clone());
  }
  if k.w > 0 && k.h > 0 {
    b["width"] = k.w.into();
    b["height"] = k.h.into();
  }
  b
}

// the add's refusal in words: a taken name keeps the upload, so it asks for another
fn add_failed(code: u16, v: &Value, k: Kept) -> Failed {
  let said = v.get("error").and_then(Value::as_str).unwrap_or("").to_lowercase();
  if code == 409 || said.contains("already in inventory") || said.contains("held by your upload") {
    return Failed { why: "you already have an emote with that name, pick another".into(), kept: Some(k) };
  }
  if said.contains("slot_limit") || said.contains("slot limit") {
    return Failed { why: "your emote set is full, remove one first".into(), kept: None };
  }
  Failed { why: why(code, v), kept: None }
}

async fn add(http: &reqwest::Client, token: &str, name: &str, k: Kept) -> Result<Done, Failed> {
  let r = http
    .post(format!("{ORIGIN}/api/user/emotes"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(add_body(name, &k).to_string())
    .send()
    .await
    .map_err(|e| Failed { why: super::super::http::offline("heatsync", e), kept: Some(k.clone()) })?;
  let code = r.status().as_u16();
  let v = super::super::http::read_json(r, MAX_REPLY).await.unwrap_or(Value::Null);
  if (200..300).contains(&code) { Ok(Done { name: name.to_owned(), kept: k }) } else { Err(add_failed(code, &v, k)) }
}

fn finish(res: Result<Done, Failed>, out: &Out) {
  match res {
    Ok(d) => {
      out.send(NetEvent::Upload(Upload::EmoteDone(d)));
      // the sender refetch: your line draws the new emote as yours
      out.send(NetEvent::Inventory { removed: None });
    }
    Err(f) => {
      out.send(NetEvent::Upload(Upload::EmoteFailed(f)));
    }
  }
}

fn bad(why: impl Into<String>) -> Failed {
  Failed { why: why.into(), kept: None }
}

// pick the file, upload it, add it
pub async fn run(http: reqwest::Client, token: String, path: PathBuf, name: String, out: Out) {
  if let Some(p) = name_problem(&name) {
    return finish(Err(bad(p)), &out);
  }
  let loaded = tokio::task::spawn_blocking(move || load(&path)).await;
  let res = async {
    let (bytes, m) = loaded.map_err(|e| bad(e.to_string()))?.map_err(bad)?;
    let v = send_form(&http, &token, bytes, m, true).await.map_err(bad)?;
    let k = kept(&v).ok_or_else(|| bad("heatsync sent back a link i don't trust"))?;
    add(&http, &token, &name, k).await
  }
  .await;
  finish(res, &out);
}

// the file is already up (a name was taken): add it under another
pub async fn run_add(http: reqwest::Client, token: String, k: Kept, name: String, out: Out) {
  if let Some(p) = name_problem(&name) {
    return finish(Err(Failed { why: p.into(), kept: Some(k) }), &out);
  }
  finish(add(&http, &token, &name, k).await, &out);
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_default_name_is_the_files_word() {
    let n = |s: &str| default_name(Path::new(s));
    assert_eq!(n("/x/Pog Champ!.png"), "PogChamp");
    assert_eq!(n("/x/kappa-2x.webp"), "kappa");
    assert_eq!(n("/x/KAPPA-4X.gif"), "KAPPA");
    assert_eq!(n("/x/my-emote.png"), "myemote", "only a size tail goes");
    assert_eq!(n("/x/日本.png"), "emote");
    assert_eq!(n("/x/.png"), "emote");
    assert_eq!(n(&format!("/x/{}.png", "a".repeat(80))).len(), 30);
  }

  #[test]
  fn a_name_is_one_word() {
    assert!(name_problem("PogChamp").is_none());
    assert!(name_problem("kappa_2").is_none());
    assert!(name_problem("").is_some());
    assert!(name_problem("two words").is_some());
    assert!(name_problem("/me").is_some());
    assert!(name_problem(".bot").is_some());
    assert!(name_problem(&"a".repeat(101)).is_some());
    assert!(name_problem(&"a".repeat(100)).is_none());
  }

  #[test]
  fn only_the_ext_s_five_pictures_are_emotes() {
    assert!(checked(b"\0\0\0\x14ftypisom\0\0\0\0isom".to_vec()).is_err(), "a video is no emote");
    assert_eq!(checked(b"BM\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec()).unwrap_err(), NOT_EMOTE);
    assert!(checked(b"<svg/>".to_vec()).is_err());
    let mut big = vec![0xff, 0xd8, 0xff];
    big.resize(MAX_EMOTE as usize + 1, 0);
    assert!(checked(big).unwrap_err().starts_with("too big"));
  }

  #[test]
  fn a_file_is_read_by_its_bytes() {
    let d = std::env::temp_dir().join(format!("hsc-emo-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("cat.png");
    std::fs::write(&f, b"not a picture").unwrap();
    assert!(load(&f).is_err());
    assert!(load(&d).unwrap_err().contains("not a file"));
    std::fs::write(d.join("e"), b"").unwrap();
    assert!(load(&d.join("e")).unwrap_err().contains("empty"));
    assert!(load(&d.join("missing")).is_err());
    let _ = std::fs::remove_dir_all(&d);
  }

  #[test]
  fn the_upload_answer_is_hostile_input() {
    let v = json!({"url": "/uploads/a.webp", "phash": "abc", "width": 128, "height": 96,
      "moderationResult": {"flaggedCategories": ["violence", "x\u{7}y", 5, ""]}});
    let k = kept(&v).unwrap();
    assert_eq!(k.url, "https://heatsync.org/uploads/a.webp");
    assert_eq!((k.w, k.h, k.phash.as_deref()), (128, 96, Some("abc")));
    assert_eq!(k.flagged, ["violence", "xy"]);
    assert!(kept(&json!({"url": "https://evil.com/a.webp"})).is_none());
    assert!(kept(&json!({"url": "//evil.com/a.webp"})).is_none());
    assert!(kept(&json!({})).is_none());
    // sizes the server would refuse are not sent; a long phash is dropped
    let k = kept(&json!({"url": "/uploads/a.webp", "width": 99999, "height": 0, "phash": "p".repeat(513)})).unwrap();
    assert_eq!((k.w, k.h, k.phash), (0, 0, None));
    let flags: Vec<Value> = (0..20).map(|i| json!(format!("c{i}"))).collect();
    let k = kept(&json!({"url": "/uploads/a.webp", "moderationResult": {"flaggedCategories": flags}})).unwrap();
    assert_eq!(k.flagged.len(), 8);
  }

  #[test]
  fn the_add_is_an_upload_with_its_size() {
    let k = Kept {
      url: "https://heatsync.org/uploads/a.webp".into(),
      phash: Some("ab".into()),
      w: 128,
      h: 96,
      flagged: vec![],
    };
    let b = add_body("PogChamp", &k);
    assert_eq!(
      (b["source"].as_str(), b["emoteName"].as_str(), b["customName"].as_str()),
      (Some("upload"), Some("PogChamp"), Some("PogChamp"))
    );
    assert_eq!((b["width"].as_u64(), b["height"].as_u64(), b["phash"].as_str()), (Some(128), Some(96), Some("ab")));
    assert!(b["sourceId"].is_null() && b["zeroWidth"] == false);
    let none = Kept { w: 0, h: 0, phash: None, ..k };
    let b = add_body("x", &none);
    assert!(b.get("width").is_none() && b.get("height").is_none() && b.get("phash").is_none());
  }

  #[test]
  fn a_taken_name_keeps_the_upload_and_the_rest_do_not() {
    let k = Kept { url: "https://heatsync.org/uploads/a.webp".into(), phash: None, w: 1, h: 1, flagged: vec![] };
    let f = add_failed(409, &json!({"error": "emote name already in inventory"}), k.clone());
    assert_eq!(f.kept.as_ref(), Some(&k));
    assert!(f.why.contains("pick another"));
    assert_eq!(add_failed(409, &json!({"error": "name held by your upload"}), k.clone()).kept, Some(k.clone()));
    let f = add_failed(403, &json!({"error": "slot_limit_reached"}), k.clone());
    assert!(f.kept.is_none() && f.why.contains("full"));
    let f = add_failed(429, &json!({}), k.clone());
    assert!(f.kept.is_none() && f.why.contains("slow down"));
    assert!(add_failed(400, &json!({"error": "emote violates content policy"}), k).why.contains("content policy"));
  }
}
