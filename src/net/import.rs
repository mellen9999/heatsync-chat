// your twitch follows for /import (the ext's get_twitch_followed_channels + fetch_live_status, background.js):
// GET /api/twitch/followed-channels, signed in, then the public GET /api/platform/live-status for who is live
// (it takes 20 names a call). the live check only sorts: a call that fails leaves those rows offline.
// every body is capped and every row checked in model::import
use std::collections::HashSet;
use std::time::Duration;

use serde_json::Value;

use super::{NetEvent, ORIGIN, Out, social};
use crate::model::import::{self, BATCH, Follow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
  // logged out, or the heatsync session lapsed
  Login,
  // twitch took the link back
  Expired,
  // the link never included follows
  Scope,
  // twitch (or heatsync) didn't answer
  Down,
}

pub struct Got {
  pub rows: Vec<Follow>,
  pub truncated: bool,
}

pub async fn run(http: reqwest::Client, token: Option<String>, out: Out) {
  let res = match token {
    None => Err(Fail::Login),
    Some(t) => follows(&http, &t).await,
  };
  out.send(NetEvent::Import(res));
}

async fn follows(http: &reqwest::Client, token: &str) -> Result<Got, Fail> {
  let req =
    http.get(format!("{ORIGIN}/api/twitch/followed-channels")).bearer_auth(token).timeout(Duration::from_secs(25));
  let (status, v) = social::call(req).await;
  let (mut rows, truncated) = read(status, &v)?;
  let live = live(http, &rows).await;
  for r in &mut rows {
    r.live = live.contains(&r.login);
  }
  import::sort(&mut rows);
  Ok(Got { rows, truncated })
}

// the follow list, or why there isn't one. the server's words: 401 `login_required`, 401 `relink_required` with
// reason `scope` (the grant never had follows) or `expired`, 502 `twitch_unavailable`, 429 when asked too fast
fn read(status: u16, v: &Value) -> Result<(Vec<Follow>, bool), Fail> {
  match status {
    200 => import::parse(v).ok_or(Fail::Down),
    401 if v.get("error").and_then(Value::as_str) == Some("relink_required") => {
      Err(if v.get("reason").and_then(Value::as_str) == Some("scope") { Fail::Scope } else { Fail::Expired })
    }
    401 => Err(Fail::Login),
    _ => Err(Fail::Down),
  }
}

// who of `rows` is live, asked 20 at a time all at once (the ext asks in one call, which the server cuts to 20)
async fn live(http: &reqwest::Client, rows: &[Follow]) -> HashSet<Box<str>> {
  let mut jobs = tokio::task::JoinSet::new();
  for chunk in rows.chunks(BATCH) {
    let names = chunk.iter().map(|r| &*r.login).collect::<Vec<_>>().join(",");
    // logins are [a-z0-9_] (checked in model::import), so they need no escaping
    let req = http.get(format!("{ORIGIN}/api/platform/live-status?channels={names}")).timeout(Duration::from_secs(8));
    jobs.spawn(async move {
      let (status, v) = social::call(req).await;
      if status == 200 { import::live_in(&v) } else { Vec::new() }
    });
  }
  let mut live = HashSet::new();
  while let Some(Ok(names)) = jobs.join_next().await {
    live.extend(names);
  }
  live
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn the_servers_answers_become_the_three_sentences_and_a_list() {
    let ok = json!({"channels": [{"login": "a", "displayName": "A"}], "truncated": false, "cached": true});
    assert_eq!(read(200, &ok).unwrap().0.len(), 1);
    assert_eq!(read(200, &json!({"channels": []})).unwrap().0.len(), 0, "following nobody is an answer");
    let relink = |reason| json!({"error": "relink_required", "relink_pack": "base", "reason": reason});
    assert_eq!(read(401, &relink("scope")).unwrap_err(), Fail::Scope);
    assert_eq!(read(401, &relink("expired")).unwrap_err(), Fail::Expired);
    assert_eq!(read(401, &json!({"error": "relink_required"})).unwrap_err(), Fail::Expired);
    assert_eq!(read(401, &json!({"error": "login_required", "platform": "twitch"})).unwrap_err(), Fail::Login);
    for (s, v) in [
      (502, json!({"error": "twitch_unavailable"})),
      (429, json!({"error": "rate limited"})),
      (200, Value::Null),
      (200, json!({"channels": "x"})),
      (500, ok.clone()),
      (0, Value::Null),
    ] {
      assert_eq!(read(s, &v).unwrap_err(), Fail::Down, "{s} {v}");
    }
  }
}
