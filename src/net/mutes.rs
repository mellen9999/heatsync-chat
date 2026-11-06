// your mute and block lists on heatsync (ext background.js: fetchServerMutes, fetchServerBlocks,
// syncMuteToServer). mutes sync both ways while you are logged in; blocks are only read from the
// server (the ext never writes them there), so a block made here stays on this machine
use crate::model::ident;
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out, http};
use crate::util::time::parse_iso;

const MAX_BODY: usize = 2 << 20;
// the server accepts [A-Za-z0-9_-] up to 32; anything else is kept locally only
pub fn syncable(name: &str) -> bool {
  ident::slug(name, 32)
}

pub enum Op {
  Add { name: Box<str>, until: Option<i64> },
  Remove(Box<str>),
}

// the server's lists, to merge into ours
pub struct Lists {
  pub mutes: Vec<(Box<str>, Option<i64>)>,
  pub blocks: Vec<Box<str>>,
  // the block list answered (an empty `blocks` is then the truth, not a failed read)
  pub blocks_known: bool,
}

// a change another device (or heatsync.org) made, pushed over the websocket
pub enum Sync {
  Added(Box<str>, Option<i64>),
  Removed(Box<str>),
  Cleared,
}

fn bearer(http: &reqwest::Client, token: &str, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
  http.request(method, format!("{ORIGIN}{path}")).bearer_auth(token)
}

pub async fn run(http: reqwest::Client, token: Option<String>, op: Op) {
  let Some(token) = token else { return };
  let r = match op {
    Op::Add { name, until } => {
      let body = json!({
        "username": &*name,
        "platform": null,
        "expires_at": until.map(iso),
        "reason": null,
      });
      bearer(&http, &token, reqwest::Method::POST, "/api/mutes")
        .header("content-type", "application/json")
        .body(body.to_string())
    }
    Op::Remove(name) => bearer(&http, &token, reqwest::Method::DELETE, &format!("/api/mutes/{name}")),
  };
  if let Err(e) = r.send().await {
    eprintln!("mutes sync: {e}");
  }
}

async fn json(r: reqwest::RequestBuilder) -> Option<Value> {
  let r = r.send().await.ok().filter(|r| r.status().is_success())?;
  http::read_json(r, MAX_BODY).await.ok()
}

pub async fn load(http: reqwest::Client, token: String, out: Out) {
  let (m, b) = tokio::join!(
    json(bearer(&http, &token, reqwest::Method::GET, "/api/mutes")),
    json(bearer(&http, &token, reqwest::Method::GET, "/api/user/blocks")),
  );
  if m.is_none() && b.is_none() {
    return;
  }
  out.send(NetEvent::Lists(Lists {
    mutes: m.as_ref().map(mutes_of).unwrap_or_default(),
    blocks: b.as_ref().map(blocks_of).unwrap_or_default(),
    blocks_known: b.is_some(),
  }));
}

fn ms(v: &Value) -> Option<i64> {
  v.get("expires_at").and_then(Value::as_str).and_then(parse_iso)
}

pub fn mutes_of(v: &Value) -> Vec<(Box<str>, Option<i64>)> {
  let rows = v.get("mutes").or(Some(v)).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  rows
    .iter()
    .take(5000)
    .filter_map(|e| Some((e.get("username").and_then(Value::as_str).map(crate::model::filter::key)?, ms(e))))
    .filter(|(n, _)| !n.is_empty() && n.len() <= 64)
    .collect()
}

pub fn blocks_of(v: &Value) -> Vec<Box<str>> {
  let rows = v.get("blocked_users").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  rows
    .iter()
    .take(5000)
    .filter_map(|e| e.get("username").and_then(Value::as_str).map(crate::model::filter::key))
    .filter(|n| !n.is_empty() && n.len() <= 64)
    .collect()
}

// ws frames: mute:added / mute:removed / mute:expired / mute:cleared
pub fn frame(v: &Value, ty: &str) -> Option<Sync> {
  if ty == "mute:cleared" {
    return Some(Sync::Cleared);
  }
  let name = crate::model::filter::key(v.get("username").and_then(Value::as_str)?);
  if name.is_empty() || name.len() > 64 {
    return None;
  }
  Some(if ty == "mute:added" { Sync::Added(name, ms(v)) } else { Sync::Removed(name) })
}

fn iso(ms: i64) -> String {
  let (secs, rem) = (ms.div_euclid(1000), ms.rem_euclid(1000));
  let days = secs.div_euclid(86400);
  let t = secs.rem_euclid(86400);
  // civil from days (howard hinnant)
  let z = days + 719468;
  let era = z.div_euclid(146097);
  let doe = z - era * 146097;
  let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  let y = yoe + era * 400 + i64::from(m <= 2);
  format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{rem:03}Z", t / 3600, t % 3600 / 60, t % 60)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn iso_roundtrips() {
    for ms in [0, 1793399211099, 951782400000, 1709164799999] {
      assert_eq!(parse_iso(&iso(ms)), Some(ms));
    }
    assert_eq!(iso(1793399211099), "2026-10-30T22:26:51.099Z");
  }

  #[test]
  fn what_the_server_sends() {
    let m = json!({ "mutes": [{ "username": "Bob", "expires_at": "2026-10-30T22:26:51.099Z" }, { "username": "al", "expires_at": null }, { "x": 1 }], "v": 3 });
    assert_eq!(mutes_of(&m), vec![("bob".into(), Some(1793399211099)), ("al".into(), None)]);
    // a bare array is read too (the ext accepts both)
    assert_eq!(mutes_of(&json!([{ "username": "z" }])).len(), 1);
    let b = json!({ "blocked_users": [{ "username": "Troll", "platform": "twitch" }, { "id": "1" }] });
    assert_eq!(blocks_of(&b), vec![Box::<str>::from("troll")]);
  }

  #[test]
  fn ws_frames() {
    assert!(
      matches!(frame(&json!({ "username": "Bob", "expires_at": null }), "mute:added"), Some(Sync::Added(n, None)) if &*n == "bob")
    );
    assert!(matches!(frame(&json!({ "username": "bob" }), "mute:expired"), Some(Sync::Removed(_))));
    assert!(matches!(frame(&json!({}), "mute:cleared"), Some(Sync::Cleared)));
    assert!(frame(&json!({}), "mute:added").is_none());
  }

  #[test]
  fn only_plain_names_sync() {
    assert!(syncable("bob_1-x"));
    assert!(!syncable("@yt.handle"));
    assert!(!syncable(&"a".repeat(33)));
  }
}
