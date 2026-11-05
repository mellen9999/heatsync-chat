// the two lookups behind the marks before a twitch name, batched like the ext's: heat by login
// (`POST /api/users/heat`, ≤25 names) and sub flair by twitch id (`GET /api/users/flair?ids=`, ≤50).
// a short gather window so a burst of new chatters costs one round trip each, one of each in flight
use std::collections::VecDeque;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;

use super::{NetEvent, ORIGIN, Out};
use crate::heatsync::{flair, heat};
use crate::net::http;

const HEAT_BATCH: usize = 25;
const FLAIR_BATCH: usize = 50;
const GATHER: Duration = Duration::from_millis(500);
const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_BODY: usize = 256 * 1024;
// bounded backlog: a flood of first-sight chatters drops the oldest asks (they ask again on next sight)
const MAX_QUEUE: usize = 600;
// after a failed round the next waits this long, doubling to the cap, so a bad spell can't become a storm
const COOLDOWN: Duration = Duration::from_secs(5);
const COOLDOWN_MAX: Duration = Duration::from_secs(60);

pub enum Ask {
  Heat(Box<str>),
  Flair(u64),
}

fn add<T: PartialEq>(q: &mut VecDeque<T>, x: T) {
  if !q.contains(&x) {
    if q.len() >= MAX_QUEUE {
      q.pop_front();
    }
    q.push_back(x);
  }
}

pub async fn run(http: reqwest::Client, mut rx: UnboundedReceiver<Ask>, out: Out) {
  let (mut heats, mut flairs): (VecDeque<Box<str>>, VecDeque<u64>) = (VecDeque::new(), VecDeque::new());
  let mut cooldown = COOLDOWN;
  let take = |a: Ask, h: &mut VecDeque<Box<str>>, f: &mut VecDeque<u64>| match a {
    Ask::Heat(l) => add(h, l),
    Ask::Flair(i) => add(f, i),
  };
  loop {
    if heats.is_empty() && flairs.is_empty() {
      let Some(a) = rx.recv().await else { return };
      take(a, &mut heats, &mut flairs);
    }
    let gather = tokio::time::sleep(GATHER);
    tokio::pin!(gather);
    while heats.len() < HEAT_BATCH && flairs.len() < FLAIR_BATCH {
      tokio::select! {
        a = rx.recv() => match a {
          Some(a) => take(a, &mut heats, &mut flairs),
          None => return,
        },
        _ = &mut gather => break,
      }
    }
    // newest first: the viewport shows the latest chatters
    let (nh, nf) = (heats.len().min(HEAT_BATCH), flairs.len().min(FLAIR_BATCH));
    let hs: Vec<Box<str>> = heats.drain(heats.len() - nh..).collect();
    let fs: Vec<u64> = flairs.drain(flairs.len() - nf..).collect();
    let (h, f) = tokio::join!(heat_batch(&http, &hs), flair_batch(&http, &fs));
    let mut failed = false;
    match h {
      Some(got) => {
        if !got.is_empty() && !out.send(NetEvent::Heat(got)) {
          return;
        }
      }
      None => {
        failed = true;
        hs.into_iter().for_each(|l| add(&mut heats, l));
      }
    }
    match f {
      Some(got) => {
        if !got.is_empty() && !out.send(NetEvent::Flair(got)) {
          return;
        }
      }
      None => {
        failed = true;
        fs.into_iter().for_each(|i| add(&mut flairs, i));
      }
    }
    if failed {
      tokio::time::sleep(cooldown).await;
      cooldown = (cooldown * 2).min(COOLDOWN_MAX);
    } else {
      cooldown = COOLDOWN;
    }
  }
}

// None on failure: `run` asks again after a cooldown; an error is not "has no heat"
async fn heat_batch(http: &reqwest::Client, logins: &[Box<str>]) -> Option<Vec<(Box<str>, u32)>> {
  if logins.is_empty() {
    return Some(Vec::new());
  }
  let body = serde_json::json!({ "usernames": logins }).to_string();
  let r = http
    .post(format!("{ORIGIN}/api/users/heat"))
    .header("content-type", "application/json")
    .timeout(TIMEOUT)
    .body(body)
    .send()
    .await;
  heat::parse(&http::capped_json(r, MAX_BODY).await.ok()?, logins)
}

async fn flair_batch(http: &reqwest::Client, ids: &[u64]) -> Option<Vec<(u64, Option<crate::badges::Badge>)>> {
  if ids.is_empty() {
    return Some(Vec::new());
  }
  let list: Vec<String> = ids.iter().map(u64::to_string).collect();
  let r = http.get(format!("{ORIGIN}/api/users/flair?ids={}", list.join(","))).timeout(TIMEOUT).send().await;
  flair::parse(&http::capped_json(r, MAX_BODY).await.ok()?, ids)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_backlog_is_bounded_and_deduped() {
    let mut q: VecDeque<u64> = VecDeque::new();
    for i in 0..MAX_QUEUE as u64 + 50 {
      add(&mut q, i);
    }
    add(&mut q, 700);
    add(&mut q, 700);
    assert_eq!(q.len(), MAX_QUEUE);
    assert_eq!(q.front(), Some(&51), "the oldest asks went");
    assert_eq!(q.iter().filter(|&&x| x == 700).count(), 1);
  }
}
