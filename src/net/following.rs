// who you follow that is live, from heatsync (the ext's live-following poll, background.js
// pollFollowedLive): GET /api/live/following, signed in. the ws snapshot and `follow:stream:*` keep it
// current between asks, this is the full answer. the body is capped and every row checked in model::following
use serde_json::Value;

use super::{NetEvent, ORIGIN, Out, social};
use crate::model::following::{self, Live};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
  // logged out, or the heatsync session lapsed
  Auth,
  Limited,
  Other,
}

pub async fn run(http: reqwest::Client, token: Option<String>, out: Out) {
  let res = match token {
    None => Err(Fail::Auth),
    Some(t) => {
      let req =
        http.get(format!("{ORIGIN}/api/live/following")).bearer_auth(t).timeout(std::time::Duration::from_secs(15));
      let (status, v) = social::call(req).await;
      read(status, &v)
    }
  };
  out.send(NetEvent::Following(res));
}

// the server answers 200 with an `error` and no streams when its own lookup failed: that is no answer,
// not "nobody is live"
fn read(status: u16, v: &Value) -> Result<Vec<Live>, Fail> {
  match status {
    200 if v.get("error").is_none() && v.get("streams").is_some_and(Value::is_array) => {
      Ok(following::parse(v.get("streams")))
    }
    401 => Err(Fail::Auth),
    429 => Err(Fail::Limited),
    _ => Err(Fail::Other),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn only_a_real_answer_replaces_the_list() {
    let ok = json!({"streams": [{"platform": "twitch", "username": "a", "viewerCount": 3}]});
    assert_eq!(read(200, &ok).unwrap().len(), 1);
    assert_eq!(read(200, &json!({"streams": []})).unwrap().len(), 0, "nobody live is an answer");
    for (s, v) in [
      (200, json!({"streams": [], "error": "Failed to fetch streams"})),
      (200, Value::Null),
      (200, json!({"streams": "x"})),
      (500, ok.clone()),
      (304, Value::Null),
    ] {
      assert_eq!(read(s, &v).unwrap_err(), Fail::Other, "{s} {v}");
    }
    assert_eq!(read(401, &json!({"streams": [], "error": "Login required"})).unwrap_err(), Fail::Auth);
    assert_eq!(read(429, &Value::Null).unwrap_err(), Fail::Limited);
  }
}
