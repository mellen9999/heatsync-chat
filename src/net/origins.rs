// which lines went through heatsync, for anyone: the seed read when a room opens and the `chat:origin` push
// after it. both end as `NetEvent::HsOrigin` (ids with their send ids), keyed by message id
use std::time::Duration;

use serde_json::Value;

use super::{NetEvent, ORIGIN, Out};
use crate::model::hsorigin::room;
use crate::model::ident;
use crate::model::message::Platform;

const MAX_BODY: usize = 1 << 20;
const MAX_IDS: usize = 500;

fn wire(p: &str) -> Option<Platform> {
  match p {
    "twitch" => Some(Platform::Twitch),
    "kick" => Some(Platform::Kick),
    "youtube" => Some(Platform::Youtube),
    _ => None,
  }
}

fn id_ok(s: &str) -> bool {
  (1..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic())
}

fn sid_of(s: Option<&Value>) -> Box<str> {
  s.and_then(Value::as_str).filter(|s| s.len() <= 128 && s.bytes().all(|b| b.is_ascii_graphic())).unwrap_or("").into()
}

// `{type:'chat:origin', platform, channel, messageId, sendId?, origin:'heatsync'}`
pub fn frame(v: &Value) -> Option<NetEvent> {
  if v.get("origin").and_then(Value::as_str) != Some("heatsync") {
    return None;
  }
  let platform = wire(v.get("platform").and_then(Value::as_str)?)?;
  let key = room(v.get("channel").and_then(Value::as_str).filter(|c| ident::relay_key(c))?);
  let id = v.get("messageId").and_then(Value::as_str).filter(|i| id_ok(i))?;
  Some(NetEvent::HsOrigin { platform, key, ids: vec![(id.into(), sid_of(v.get("sendId")))] })
}

// `{origins: [{message_id, origin, ts, send_id}]}`
fn parse(v: &Value) -> Vec<(Box<str>, Box<str>)> {
  let list = v.get("origins").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  list
    .iter()
    .filter(|o| o.get("origin").and_then(Value::as_str) == Some("heatsync"))
    .filter_map(|o| {
      Some((o.get("message_id").and_then(Value::as_str).filter(|i| id_ok(i))?.into(), sid_of(o.get("send_id"))))
    })
    .take(MAX_IDS)
    .collect()
}

// the room a kick or youtube join opens (twitch's is seeded where its tab joins)
pub fn room_of(c: &super::hs_ws::Cmd) -> Option<(Platform, Box<str>)> {
  use super::hs_ws::Cmd;
  match c {
    Cmd::Join(Platform::Kick, k) => Some((Platform::Kick, k.clone())),
    Cmd::YtSub(h) => Some((Platform::Youtube, h.clone())),
    _ => None,
  }
}

// a room's seed, once. a miss costs only the marks that came before the app looked: pushes still land
pub async fn seed(http: reqwest::Client, platform: Platform, chan: Box<str>, out: Out) {
  if !ident::relay_key(&chan) {
    return;
  }
  let key = room(&chan);
  let url = format!("{ORIGIN}/api/chat/origins?platform={}&channel={key}&limit={MAX_IDS}", platform.name());
  let got = async {
    let r = http.get(url).timeout(Duration::from_secs(10)).send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
      return Err(format!("http {}", r.status().as_u16()));
    }
    let body = super::http::read(r, MAX_BODY).await?;
    serde_json::from_slice::<Value>(&body).map_err(|e| e.to_string())
  };
  match got.await {
    Ok(v) => {
      let ids = parse(&v);
      if !ids.is_empty() {
        out.send(NetEvent::HsOrigin { platform, key: chan, ids });
      }
    }
    Err(e) => eprintln!("origins for {platform:?} {key}: {e}"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn ids(e: Option<NetEvent>) -> Vec<(String, String)> {
    match e {
      Some(NetEvent::HsOrigin { ids, .. }) => ids.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
      _ => Vec::new(),
    }
  }

  #[test]
  fn a_push_names_one_line_and_junk_names_none() {
    let ok = json!({"type": "chat:origin", "platform": "kick", "channel": "Xqc", "messageId": "m1", "sendId": "s9", "origin": "heatsync"});
    assert_eq!(ids(frame(&ok)), [("m1".into(), "s9".into())]);
    let mut bad = ok.clone();
    bad["platform"] = "yt".into();
    assert!(frame(&bad).is_none(), "an alias is not a platform");
    bad = ok.clone();
    bad["origin"] = "twitch".into();
    assert!(frame(&bad).is_none());
    bad = ok.clone();
    bad["messageId"] = "a b\n".into();
    assert!(frame(&bad).is_none());
    let mut bare = ok.clone();
    bare.as_object_mut().unwrap().remove("sendId");
    assert_eq!(ids(frame(&bare)), [("m1".into(), "".into())]);
  }

  #[test]
  fn the_seed_keeps_heatsync_rows() {
    let v = json!({"origins": [
      {"message_id": "a", "origin": "heatsync", "ts": 1, "send_id": "s"},
      {"message_id": "b", "origin": "other", "ts": 1, "send_id": null},
      {"message_id": "c", "origin": "heatsync", "ts": 1},
      {"origin": "heatsync"},
    ]});
    assert_eq!(parse(&v), [("a".into(), "s".into()), ("c".into(), "".into())]);
    assert!(parse(&json!({})).is_empty());
  }
}
