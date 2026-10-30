// sender inventories from `/api/users/emotes/batch`: ≤15 ids per request, one request in flight,
// a short gather window so a burst of new chatters costs one round trip
use std::collections::VecDeque;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

use super::{NetEvent, ORIGIN, Out};
use crate::emotes::providers::fetch;
use crate::heatsync::senders::{Set, parse_set};

const BATCH: usize = 15;
const GATHER: Duration = Duration::from_secs(2);
// bounded backlog: a flood of first-sight chatters drops the oldest asks (they re-ask on next sight)
const MAX_QUEUE: usize = 600;

// (twitch user id, opaque cache-bust version from a ws invalidation)
pub type Ask = (u64, Option<Box<str>>);

pub async fn run(http: reqwest::Client, mut rx: UnboundedReceiver<Ask>, out: Out) {
  let mut queue: VecDeque<Ask> = VecDeque::new();
  let add = |q: &mut VecDeque<Ask>, a: Ask| {
    if let Some(x) = q.iter_mut().find(|x| x.0 == a.0) {
      if a.1.is_some() {
        x.1 = a.1;
      }
    } else {
      if q.len() >= MAX_QUEUE {
        q.pop_front();
      }
      q.push_back(a);
    }
  };
  loop {
    if queue.is_empty() {
      let Some(a) = rx.recv().await else { return };
      add(&mut queue, a);
    }
    let gather = tokio::time::sleep(GATHER);
    tokio::pin!(gather);
    while queue.len() < BATCH {
      tokio::select! {
        a = rx.recv() => match a {
          Some(a) => add(&mut queue, a),
          None => return,
        },
        _ = &mut gather => break,
      }
    }
    let n = queue.len().min(BATCH);
    let asks: Vec<Ask> = queue.drain(..n).collect();
    if let Some(sets) = batch(&http, &asks).await
      && !out.send(NetEvent::Senders(sets))
    {
      return;
    }
  }
}

// None on failure: the ui's in-flight timer retries later; an error is not "owns nothing"
async fn batch(http: &reqwest::Client, asks: &[Ask]) -> Option<Vec<(u64, Set)>> {
  let ids: Vec<String> = asks.iter().map(|(id, _)| format!("twitch:{id}")).collect();
  let mut url = format!("{ORIGIN}/api/users/emotes/batch?ids={}", ids.join(","));
  if let Some(v) = asks.iter().rev().find_map(|a| a.1.as_deref()) {
    url.push_str("&v=");
    url.push_str(v);
  }
  let v = fetch(http, &url).await.ok()?;
  let sets = v.get("sets")?;
  Some(asks.iter().zip(&ids).map(|((id, _), key)| (*id, sets.get(key).map_or_else(Set::new, parse_set))).collect())
}

// another user's inventory changed (`emote:added|removed` with a `username`):
// their `senderKeys` → twitch ids, plus the server's cache-bust token
pub fn stale(v: &Value) -> (Vec<u64>, Option<Box<str>>) {
  let Some(keys) = v.get("senderKeys").and_then(Value::as_array) else { return (Vec::new(), None) };
  let ids = keys.iter().take(30).filter_map(|k| k.as_str()?.strip_prefix("twitch:")?.parse().ok()).collect();
  let ver = v
    .get("ver")
    .and_then(|x| match x {
      Value::String(s) => Some(s.clone()),
      Value::Number(n) => Some(n.to_string()),
      _ => None,
    })
    .map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')).take(32).collect::<String>())
    .filter(|s| !s.is_empty())
    .map(Into::into);
  (ids, ver)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn stale_keys() {
    let v = json!({"senderKeys": ["twitch:12", "kick:foo", "twitch:x", 5, "twitch:7"], "ver": "a&b=1"});
    assert_eq!(stale(&v), (vec![12, 7], Some("ab1".into())));
    assert!(stale(&json!({})).0.is_empty());
  }
}
