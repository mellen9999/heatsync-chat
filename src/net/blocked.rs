// a twitch room's blocked terms through heatsync (ext background.js blocked_terms): GET / POST / DELETE
// /api/mod/blocked-terms. needs the twitch `blockedterms` permission pack; the server's helix call is the
// moderator check
use reqwest::Method;
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out};
use crate::model::blocked::{self, Term};

// 10000 terms of up to 500 chars, with room for the json around them
const MAX_BODY: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fail {
  // logged out, or the heatsync session lapsed
  Auth,
  // the twitch login lacks the blocked terms permission
  Relink,
  NotMod,
  // the term (or the room) is gone already
  Gone,
  Limited,
  Other,
}

pub enum Op {
  List,
  Add(String),
  Remove(Box<str>),
}

pub enum Got {
  List(Vec<Term>, bool),
  Added(Term),
  Removed(Box<str>),
}

pub async fn run(http: reqwest::Client, token: Option<String>, room: Box<str>, op: Op, out: Out) {
  let res = match token {
    None => Err(Fail::Auth),
    // a twitch room id (ROOMSTATE): digits, nothing that could bend the url
    Some(_) if room.is_empty() || room.len() > 20 || !room.bytes().all(|b| b.is_ascii_digit()) => Err(Fail::Other),
    Some(t) => ask(&http, &t, &room, op).await,
  };
  out.send(NetEvent::Blocked { room, res });
}

async fn ask(http: &reqwest::Client, token: &str, room: &str, op: Op) -> Result<Got, Fail> {
  let url = format!("{ORIGIN}/api/mod/blocked-terms");
  let req = match &op {
    Op::List => http.get(format!("{url}?broadcaster_id={room}")),
    Op::Add(text) => body(http, Method::POST, &url, json!({ "broadcaster_id": room, "text": text })),
    Op::Remove(id) => body(http, Method::DELETE, &url, json!({ "broadcaster_id": room, "id": &**id })),
  };
  let mut r = req.bearer_auth(token).send().await.map_err(|_| Fail::Other)?;
  let status = r.status().as_u16();
  let mut buf = Vec::new();
  if r.content_length().is_none_or(|n| n <= MAX_BODY as u64) {
    while let Ok(Some(c)) = r.chunk().await {
      if buf.len() + c.len() > MAX_BODY {
        return Err(Fail::Other);
      }
      buf.extend_from_slice(&c);
    }
  }
  let v: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
  classify(status, &v)?;
  match op {
    Op::List => {
      let (terms, more) = blocked::list(&v);
      Ok(Got::List(terms, more))
    }
    Op::Add(_) => v.get("term").and_then(blocked::term).map(Got::Added).ok_or(Fail::Other),
    Op::Remove(id) => Ok(Got::Removed(id)),
  }
}

fn body(http: &reqwest::Client, m: Method, url: &str, b: Value) -> reqwest::RequestBuilder {
  http.request(m, url).header("content-type", "application/json").body(b.to_string())
}

pub fn classify(status: u16, v: &Value) -> Result<(), Fail> {
  if (200..300).contains(&status) {
    return Ok(());
  }
  let relink = v.get("relink_required").and_then(Value::as_bool) == Some(true)
    || v.get("error").and_then(Value::as_str) == Some("relink_required");
  Err(match status {
    401 if relink => Fail::Relink,
    401 => Fail::Auth,
    403 => Fail::NotMod,
    404 => Fail::Gone,
    429 => Fail::Limited,
    _ => Fail::Other,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn answers_like_the_server_gives_them() {
    assert_eq!(classify(200, &json!({"terms": []})), Ok(()));
    let pack = json!({"error": "twitch blocked-terms scopes not granted", "relink_required": true, "scope_pack": "blockedterms"});
    assert_eq!(classify(401, &pack), Err(Fail::Relink));
    // a bare 401 is our own session: relinking twitch can't fix it
    assert_eq!(classify(401, &json!({"error": "unauthorized"})), Err(Fail::Auth));
    assert_eq!(classify(403, &json!({"error": "not a moderator of this channel"})), Err(Fail::NotMod));
    assert_eq!(classify(404, &Value::Null), Err(Fail::Gone));
    assert_eq!(classify(429, &Value::Null), Err(Fail::Limited));
    assert_eq!(classify(502, &Value::Null), Err(Fail::Other));
  }
}
