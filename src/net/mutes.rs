// your mute and block lists on heatsync (ext background.js: fetchServerMutes, fetchServerBlocks,
// syncMuteToServer). mutes sync both ways while you are logged in; blocks are only read from the
// server (the ext never writes them there), so a block made here stays on this machine
use crate::i18n::f;
use crate::model::ident;
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out, http};
use crate::util::time::parse_iso;

const MAX_BODY: usize = 2 << 20;
// the server accepts [A-Za-z0-9_-] up to 32; anything else is kept locally only
pub fn syncable(name: &str) -> bool {
  ident::slug(name, 32)
}

#[derive(Clone, Debug, PartialEq)]
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

// why one try did not land: `Retry` is worth another go (offline, busy, a server hiccup), `Give` is not
#[derive(Debug, PartialEq)]
pub enum Fail {
  Retry(String),
  Give(String),
}

// tries per change before the user is told
const TRIES: u32 = 4;

// what the server's answer means for a change: done, or why not. unmuting someone it never had (404) is as
// good as done
fn judge(op: &Op, status: u16) -> Result<(), Fail> {
  match status {
    200..=299 => Ok(()),
    404 if matches!(op, Op::Remove(_)) => Ok(()),
    408 | 425 | 429 | 500.. => Err(Fail::Retry(http::plain_status(status))),
    _ => Err(Fail::Give(http::plain_status(status))),
  }
}

async fn send_one(http: &reqwest::Client, token: &str, op: &Op) -> Result<(), Fail> {
  let r = match op {
    Op::Add { name, until } => {
      let body = json!({
        "username": &**name,
        "platform": null,
        "expires_at": until.map(iso),
        "reason": null,
      });
      bearer(http, token, reqwest::Method::POST, "/api/mutes")
        .header("content-type", "application/json")
        .body(body.to_string())
    }
    Op::Remove(name) => bearer(http, token, reqwest::Method::DELETE, &format!("/api/mutes/{name}")),
  };
  match r.send().await {
    Ok(r) => judge(op, r.status().as_u16()),
    Err(e) => Err(Fail::Retry(http::offline("heatsync", e))),
  }
}

fn gave_up(op: &Op, why: &str) -> String {
  match op {
    Op::Add { name, .. } => f("net2.mutes.mute_failed", &[("name", name), ("why", why)]),
    Op::Remove(name) => f("net2.mutes.unmute_failed", &[("name", name), ("why", why)]),
  }
}

pub type Queued = (Option<String>, Op);

// your mute changes, one at a time in the order you made them (an unmute racing the mute before it would
// leave the server wrong, and the next login would mute them again). a change that keeps failing is said
// out loud; the ones behind it still go
pub async fn drain<F, Fut>(
  mut rx: tokio::sync::mpsc::UnboundedReceiver<Queued>,
  out: Out,
  wait_for: impl Fn(u32) -> std::time::Duration,
  mut one: F,
) where
  F: FnMut(String, Op) -> Fut,
  Fut: std::future::Future<Output = Result<(), Fail>>,
{
  while let Some((token, op)) = rx.recv().await {
    let Some(token) = token else { continue };
    let mut tries = 0;
    loop {
      match one(token.clone(), op.clone()).await {
        Ok(()) => break,
        Err(Fail::Give(why)) => {
          out.status(gave_up(&op, &why));
          break;
        }
        Err(Fail::Retry(why)) => {
          tries += 1;
          if tries >= TRIES {
            out.status(gave_up(&op, &why));
            break;
          }
          tokio::time::sleep(wait_for(tries)).await;
        }
      }
    }
  }
}

pub async fn run(http: reqwest::Client, rx: tokio::sync::mpsc::UnboundedReceiver<Queued>, out: Out) {
  drain(rx, out, super::backoff, move |t, op| {
    let http = http.clone();
    async move { send_one(&http, &t, &op).await }
  })
  .await
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

  fn said(rx: &std::sync::mpsc::Receiver<NetEvent>) -> Vec<String> {
    rx.try_iter().filter_map(|e| if let NetEvent::Status(s) = e { Some(s) } else { None }).collect()
  }

  // the server's answer decides: only a busy or broken server is worth another try
  #[test]
  fn statuses_decide_retry_or_done() {
    let add = Op::Add { name: "bob".into(), until: None };
    let del = Op::Remove("bob".into());
    assert_eq!(judge(&add, 200), Ok(()));
    assert_eq!(judge(&del, 404), Ok(()), "already unmuted there");
    assert!(matches!(judge(&add, 404), Err(Fail::Give(_))));
    assert!(matches!(judge(&del, 503), Err(Fail::Retry(_))));
    assert!(matches!(judge(&del, 429), Err(Fail::Retry(_))));
    assert!(matches!(judge(&del, 401), Err(Fail::Give(w)) if w.contains("log in")));
    assert!(matches!(judge(&add, 403), Err(Fail::Give(_))));
  }

  // mute then unmute go out in the order made, even when the first needs retries
  #[tokio::test]
  async fn changes_go_out_in_order_and_transient_failures_retry() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (otx, orx) = std::sync::mpsc::channel();
    let log = Arc::new(Mutex::new(Vec::<&str>::new()));
    let fails = Arc::new(AtomicU32::new(0));
    tx.send((Some("t".into()), Op::Add { name: "bob".into(), until: None })).unwrap();
    tx.send((Some("t".into()), Op::Remove("bob".into()))).unwrap();
    drop(tx);
    let (l, f) = (log.clone(), fails.clone());
    drain(
      rx,
      Out::Pipe(otx),
      |_| std::time::Duration::from_millis(1),
      move |_, op| {
        let (l, f) = (l.clone(), f.clone());
        async move {
          l.lock().unwrap().push(if matches!(op, Op::Add { .. }) { "add" } else { "remove" });
          if matches!(op, Op::Add { .. }) && f.fetch_add(1, Ordering::Relaxed) < 2 {
            return Err(Fail::Retry("busy".into()));
          }
          Ok(())
        }
      },
    )
    .await;
    assert_eq!(*log.lock().unwrap(), ["add", "add", "add", "remove"]);
    assert!(said(&orx).is_empty(), "everything landed, nothing to tell");
  }

  // a change that never lands is said in plain words, and the next one still goes
  #[tokio::test]
  async fn a_change_that_keeps_failing_is_told_and_the_queue_goes_on() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (otx, orx) = std::sync::mpsc::channel();
    tx.send((Some("t".into()), Op::Remove("bob".into()))).unwrap();
    tx.send((Some("t".into()), Op::Add { name: "al".into(), until: None })).unwrap();
    drop(tx);
    let calls = Arc::new(AtomicU32::new(0));
    let c = calls.clone();
    drain(
      rx,
      Out::Pipe(otx),
      |_| std::time::Duration::from_millis(1),
      move |_, op| {
        let c = c.clone();
        async move {
          c.fetch_add(1, Ordering::Relaxed);
          match op {
            Op::Remove(_) => Err(Fail::Retry("heatsync is having trouble, try again soon".into())),
            Op::Add { .. } => Ok(()),
          }
        }
      },
    )
    .await;
    assert_eq!(calls.load(Ordering::Relaxed), TRIES + 1, "tries on bob, then al once");
    let said = said(&orx);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains("unmute of bob") && said[0].contains("next login"), "{}", said[0]);
  }

  #[test]
  fn only_plain_names_sync() {
    assert!(syncable("bob_1-x"));
    assert!(!syncable("@yt.handle"));
    assert!(!syncable(&"a".repeat(33)));
  }
}
