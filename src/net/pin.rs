// pinned lines: `GET /api/pin?platform=&channel=` for each joined twitch/kick room (on join and after a
// reconnect, which may have missed a `pin:*` frame). one request at a time; the server caches 5s
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

use super::{NetEvent, ORIGIN, Out};
use crate::emotes::providers::fetch;
use crate::model::message::Platform;
use crate::model::pin;

pub type Ask = (Platform, Box<str>);

const MAX_QUEUE: usize = 64;
const MAX_SEEN: usize = 256;
// asking again sooner than this can't learn anything: the server caches that long
const MIN_GAP: Duration = Duration::from_secs(5);

pub fn name(p: Platform) -> Option<&'static str> {
  match p {
    Platform::Twitch => Some("twitch"),
    Platform::Kick => Some("kick"),
    Platform::Youtube => None,
  }
}

pub fn valid_channel(c: &str) -> bool {
  (1..=50).contains(&c.len())
    && c.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

pub async fn run(http: reqwest::Client, mut rx: UnboundedReceiver<Ask>, out: Out) {
  let mut queue: VecDeque<Ask> = VecDeque::new();
  let mut last: HashMap<(u8, Box<str>), Instant> = HashMap::new();
  loop {
    if queue.is_empty() {
      let Some(a) = rx.recv().await else { return };
      queue.push_back(a);
    }
    while let Ok(a) = rx.try_recv() {
      if !queue.contains(&a) {
        if queue.len() >= MAX_QUEUE {
          queue.pop_front();
        }
        queue.push_back(a);
      }
    }
    let Some((p, ch)) = queue.pop_front() else { continue };
    let Some(plat) = name(p) else { continue };
    if !valid_channel(&ch) || last.get(&(p as u8, ch.clone())).is_some_and(|t| t.elapsed() < MIN_GAP) {
      continue;
    }
    if last.len() >= MAX_SEEN {
      last.retain(|_, t| t.elapsed() < MIN_GAP);
    }
    last.insert((p as u8, ch.clone()), Instant::now());
    // a failed lookup says nothing: keep whatever pin we hold
    if let Ok(v) = fetch(&http, &format!("{ORIGIN}/api/pin?platform={plat}&channel={ch}")).await {
      let pin = v.get("pin").filter(|x| !x.is_null()).and_then(pin::parse);
      if !out.send(NetEvent::Pin { platform: p, key: ch, pin }) {
        return;
      }
    }
  }
}

// ws `pin:set {platform, channel, pin}` / `pin:clear {platform, channel}`
pub fn frame(v: &Value, set: bool) -> Option<NetEvent> {
  let platform = match v.get("platform").and_then(Value::as_str)? {
    "twitch" => Platform::Twitch,
    "kick" => Platform::Kick,
    _ => return None,
  };
  let key: Box<str> = v.get("channel").and_then(Value::as_str)?.to_ascii_lowercase().into();
  if !valid_channel(&key) {
    return None;
  }
  let pin = if set { Some(pin::parse(v.get("pin")?)?) } else { None };
  Some(NetEvent::Pin { platform, key, pin })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn frames_are_validated() {
    let set = json!({"platform": "kick", "channel": "XQC", "pin": {"id": "1", "content": "hello"}});
    assert!(matches!(frame(&set, true), Some(NetEvent::Pin { platform: Platform::Kick, pin: Some(_), .. })));
    assert!(matches!(frame(&set, false), Some(NetEvent::Pin { pin: None, .. })));
    assert!(frame(&json!({"platform": "youtube", "channel": "x"}), false).is_none());
    assert!(frame(&json!({"platform": "twitch", "channel": "bad name"}), false).is_none());
    assert!(frame(&json!({"platform": "twitch", "channel": "x"}), true).is_none());
    assert!(!valid_channel(&"a".repeat(51)));
  }
}
