// chatters' 7tv paints + badges from `/api/cosmetics/batch`: ≤25 ids per request, one in flight,
// a short gather window so a burst of new chatters costs one round trip
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;

use super::{NetEvent, ORIGIN, Out};
use crate::cosmetics;
use crate::emotes::providers::fetch;

const BATCH: usize = 25;
const GATHER: Duration = Duration::from_millis(300);
// a failed batch (rate limit, outage) waits before the next one so a bad spell can't become a storm
const COOLDOWN: Duration = Duration::from_secs(5);
const MAX_QUEUE: usize = 600;

pub async fn run(http: reqwest::Client, mut rx: UnboundedReceiver<u64>, out: Out) {
  let mut queue: VecDeque<u64> = VecDeque::new();
  let mut paints = HashMap::new();
  let add = |q: &mut VecDeque<u64>, id: u64| {
    if !q.contains(&id) {
      if q.len() >= MAX_QUEUE {
        q.pop_front();
      }
      q.push_back(id);
    }
  };
  loop {
    if queue.is_empty() {
      let Some(id) = rx.recv().await else { return };
      add(&mut queue, id);
    }
    let gather = tokio::time::sleep(GATHER);
    tokio::pin!(gather);
    while queue.len() < BATCH {
      tokio::select! {
        id = rx.recv() => match id {
          Some(id) => add(&mut queue, id),
          None => return,
        },
        _ = &mut gather => break,
      }
    }
    // newest first: the viewport shows the latest chatters
    let n = queue.len().min(BATCH);
    let mut ids: Vec<u64> = queue.drain(queue.len() - n..).collect();
    ids.sort_unstable();
    match batch(&http, &ids, &mut paints).await {
      Some(got) => {
        if !out.send(NetEvent::Cosmetics(got)) {
          return;
        }
      }
      None => tokio::time::sleep(COOLDOWN).await,
    }
  }
}

// None on failure: the ui's in-flight timer retries; an error is not "wears nothing"
async fn batch(
  http: &reqwest::Client,
  ids: &[u64],
  paints: &mut HashMap<Box<str>, std::sync::Arc<cosmetics::paint::Paint>>,
) -> Option<Vec<(u64, Option<cosmetics::Cosmetic>)>> {
  let list: Vec<String> = ids.iter().map(u64::to_string).collect();
  let list = list.join(",");
  // 7tv's cosmetics and heatsync's own paints are two routes; either failing retries the batch
  let (u_cos, u_own) = (format!("{ORIGIN}/api/cosmetics/batch?ids={list}"), format!("{ORIGIN}/api/paints?ids={list}"));
  let (cos, own) = tokio::join!(fetch(http, &u_cos), fetch(http, &u_own));
  let (cos, own) = (cos.ok()?, own.ok()?);
  let map = cos.get("cosmetics")?.as_object()?;
  let own = own.get("paints").and_then(|p| p.as_object());
  Some(
    ids
      .iter()
      .map(|id| {
        let k = id.to_string();
        (*id, cosmetics::parse(map.get(&k).unwrap_or(&serde_json::Value::Null), own.and_then(|o| o.get(&k)), paints))
      })
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  // real network: ennortix (twitch 37005319) wears a heatsync-native paint and no 7tv one
  #[tokio::test]
  #[ignore]
  async fn live_batch_returns_ennortix_paint() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = reqwest::Client::new();
    let mut cache = HashMap::new();
    let got = batch(&http, &[37005319], &mut cache).await.expect("batch failed");
    eprintln!(
      "{:?}",
      got.iter().map(|(i, c)| (i, c.as_ref().map(|c| c.paint.as_ref().map(|p| p.id.clone())))).collect::<Vec<_>>()
    );
    assert!(got[0].1.as_ref().and_then(|c| c.paint.as_ref()).is_some());
  }
}
