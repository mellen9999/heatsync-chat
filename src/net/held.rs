// automod holds through heatsync (ext background.js automod_watch / automod_action): POST
// /api/mod/automod-watch asks the server to watch a room you moderate (and hands back what twitch still
// holds there); POST /api/mod/automod-action allows or denies one. both need the twitch `automod`
// permission pack; the server says `relink_required` when it is missing
use crate::i18n::t;
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out};
use crate::model::held;
use crate::util::time::now_ms;

// 100 holds of 2000 chars, with room for the json around them
const MAX_BODY: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fail {
  // logged out, or the heatsync session lapsed
  Auth,
  // the twitch login lacks the automod permission: re-link twitch with `automod`
  Relink,
  NotMod,
  // twitch no longer holds it (settled elsewhere, or lapsed)
  Gone,
  Other(Box<str>),
}

pub enum Cmd {
  // (room login, twitch room id)
  Watch(Box<str>, Box<str>),
  // (room login, message id, allow)
  Act(Box<str>, Box<str>, bool),
}

pub async fn run(http: reqwest::Client, token: Option<String>, c: Cmd, out: Out) {
  match c {
    Cmd::Watch(chan, room) => {
      let res = match token {
        None => Err(Fail::Auth),
        Some(t) => post(&http, &t, "automod-watch", json!({ "broadcaster_id": &*room }))
          .await
          .map(|v| held::pending(&v, now_ms())),
      };
      out.send(NetEvent::HeldWatched { chan, res });
    }
    Cmd::Act(chan, id, allow) => {
      let res = match token {
        None => Err(Fail::Auth),
        Some(t) => {
          let action = if allow { "allow" } else { "deny" };
          post(&http, &t, "automod-action", json!({ "msg_id": &*id, "action": action })).await.map(drop)
        }
      };
      out.send(NetEvent::HeldActed { chan, id, allow, res });
    }
  }
}

async fn post(http: &reqwest::Client, token: &str, route: &str, body: Value) -> Result<Value, Fail> {
  let mut r = http
    .post(format!("{ORIGIN}/api/mod/{route}"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(body.to_string())
    .send()
    .await
    .map_err(|_| Fail::Other(t("net2.http.no_answer").into()))?;
  let status = r.status().as_u16();
  let mut buf = Vec::new();
  if r.content_length().is_none_or(|n| n <= MAX_BODY as u64) {
    while let Ok(Some(c)) = r.chunk().await {
      if buf.len() + c.len() > MAX_BODY {
        buf.clear();
        break;
      }
      buf.extend_from_slice(&c);
    }
  }
  let v: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
  classify(status, v)
}

pub fn classify(status: u16, v: Value) -> Result<Value, Fail> {
  if (200..300).contains(&status) {
    return Ok(v);
  }
  let err = v.get("error").and_then(Value::as_str).unwrap_or("");
  if err == "relink_required" || v.get("relink_required").and_then(Value::as_bool) == Some(true) {
    return Err(Fail::Relink);
  }
  Err(match status {
    401 => Fail::Auth,
    // an action on a hold twitch no longer has
    400 | 404 => Fail::Gone,
    403 => Fail::NotMod,
    429 => Fail::Other(t("net2.modact.slow").into()),
    _ => Fail::Other(super::http::plain_status(status).into()),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn answers_like_the_server_gives_them() {
    let ok = classify(200, json!({"ok": true, "pending": []}));
    assert_eq!(ok, Ok(json!({"ok": true, "pending": []})));
    let pack = json!({"error": "relink_required", "relink_pack": "automod", "platform": "twitch"});
    assert_eq!(classify(401, pack), Err(Fail::Relink));
    let unlinked = json!({"error": "platform account not linked", "relink_required": true});
    assert_eq!(classify(401, unlinked), Err(Fail::Relink));
    // a bare 401 is our own session, not twitch: relinking twitch can't fix it
    assert_eq!(classify(401, json!({"error": "unauthorized"})), Err(Fail::Auth));
    assert_eq!(classify(404, json!({"error": "gone"})), Err(Fail::Gone));
    assert_eq!(classify(403, json!({"error": "not_moderator"})), Err(Fail::NotMod));
    assert_eq!(classify(502, Value::Null), Err(Fail::Other("heatsync is having trouble, try again soon".into())));
  }
}
