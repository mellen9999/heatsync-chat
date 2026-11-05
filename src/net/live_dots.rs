// the red dot on a live tab (ext updateLiveStatus): heatsync's public live-status for the open twitch and
// kick channels, BATCH of each a call (the server drops the rest), all calls at once, no login
use std::time::Duration;

use super::{NetEvent, ORIGIN, Out, social};
use crate::model::live_dots::{self, BATCH};

pub async fn ask(http: reqwest::Client, twitch: Vec<Box<str>>, kick: Vec<Box<str>>, out: Out) {
  let calls = twitch.len().div_ceil(BATCH).max(kick.len().div_ceil(BATCH));
  let mut jobs = tokio::task::JoinSet::new();
  for i in 0..calls {
    let part = |all: &[Box<str>]| all.iter().skip(i * BATCH).take(BATCH).cloned().collect::<Vec<_>>();
    let (tw, kk) = (part(&twitch), part(&kick));
    // names were checked by live_dots::askable ([a-z0-9_-]), so they need no escaping
    let url = format!("{ORIGIN}/api/platform/live-status?channels={}&kick_channels={}", tw.join(","), kk.join(","));
    let req = http.get(url).timeout(Duration::from_secs(8));
    jobs.spawn(async move {
      let (status, v) = social::call(req).await;
      let (live, kick_live) = if status == 200 { live_dots::parse(&v) } else { (None, None) };
      NetEvent::LiveDots { twitch: (tw, live), kick: (kk, kick_live) }
    });
  }
  while let Some(Ok(ev)) = jobs.join_next().await {
    out.send(ev);
  }
}
