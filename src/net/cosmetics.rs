// chatters' 7tv paints + badges from `/api/cosmetics/batch`: ≤25 ids per request, one in flight,
// a short gather window so a burst of new chatters costs one round trip
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;

use super::{NetEvent, ORIGIN, Out};
use crate::cosmetics::{self, id::PaintId};
use crate::emotes::providers::fetch;

const BATCH: usize = 25;
const GATHER: Duration = Duration::from_millis(300);
// a failed batch (rate limit, outage) waits before the next one so a bad spell can't become a storm
const COOLDOWN: Duration = Duration::from_secs(5);
const MAX_QUEUE: usize = 600;

pub async fn run(http: reqwest::Client, mut rx: UnboundedReceiver<PaintId>, out: Out) {
  let mut queue: VecDeque<PaintId> = VecDeque::new();
  let mut paints = HashMap::new();
  let add = |q: &mut VecDeque<PaintId>, id: PaintId| {
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
    let ids: Vec<PaintId> = queue.drain(queue.len() - n..).collect();
    match batch(&http, &ids, &mut paints).await {
      Some(got) => {
        if !out.send(NetEvent::Cosmetics(got)) {
          return;
        }
      }
      // a failed batch asks again itself: the rows that wanted it are laid out already and won't ask twice
      None => {
        tokio::time::sleep(COOLDOWN).await;
        ids.into_iter().for_each(|id| add(&mut queue, id));
      }
    }
  }
}

// None on failure: `run` retries after a cooldown; an error is not "wears nothing".
// 7tv's route knows twitch and kick ids only; heatsync's own paints cover every namespace. the answers
// are read back per id, so a native "5" and twitch 5 (one wire string) still get their own verdicts
async fn batch(
  http: &reqwest::Client,
  ids: &[PaintId],
  paints: &mut HashMap<Box<str>, std::sync::Arc<cosmetics::paint::Paint>>,
) -> Option<Vec<(PaintId, Option<cosmetics::Cosmetic>)>> {
  let list = |keep: fn(&PaintId) -> bool| {
    let mut w: Vec<String> = ids.iter().filter(|i| keep(i)).map(PaintId::wire).collect();
    w.sort_unstable();
    w.dedup();
    w.join(",")
  };
  let (seven, all) = (list(PaintId::seven_tv), list(|_| true));
  let u_own = format!("{ORIGIN}/api/paints?ids={all}");
  let u_cos = format!("{ORIGIN}/api/cosmetics/batch?ids={seven}");
  // either failing retries the batch
  let (cos, own) = tokio::join!(
    async { if seven.is_empty() { Ok(serde_json::json!({"cosmetics": {}})) } else { fetch(http, &u_cos).await } },
    fetch(http, &u_own)
  );
  let (cos, own) = (cos.ok()?, own.ok()?);
  let map = cos.get("cosmetics")?.as_object()?;
  let (plus, own) = (own.get("plus").and_then(|p| p.as_object()), own.get("paints").and_then(|p| p.as_object()));
  Some(
    ids
      .iter()
      .map(|id| {
        let k = id.wire();
        let seven = if id.seven_tv() { map.get(&k) } else { None };
        (
          id.clone(),
          cosmetics::parse(
            seven.unwrap_or(&serde_json::Value::Null),
            own.and_then(|o| o.get(&k)),
            plus.and_then(|o| o.get(&k)),
            paints,
          ),
        )
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
    let got = batch(&http, &[PaintId::Twitch(37005319)], &mut cache).await.expect("batch failed");
    eprintln!(
      "{:?}",
      got
        .iter()
        .map(|(i, c)| (i.wire(), c.as_ref().map(|c| c.paint.as_ref().map(|p| p.id.clone()))))
        .collect::<Vec<_>>()
    );
    assert!(got[0].1.as_ref().and_then(|c| c.paint.as_ref()).is_some());
  }
}
