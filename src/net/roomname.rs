// a shared-chat room's channel name by its number (ext resolveTwitchLoginById): twitch's public web gql,
// no sign-in. two asks in the air at most, each 3s at most, each answer small and checked
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::sync::mpsc::UnboundedReceiver;

use super::http::capped_json;
use super::{NetEvent, Out};

const IN_FLIGHT: usize = 2;
const TIMEOUT: Duration = Duration::from_secs(3);
// the answer is a few dozen bytes
const MAX_BODY: usize = 16 << 10;

pub async fn run(http: reqwest::Client, mut rx: UnboundedReceiver<Box<str>>, out: Out) {
  let gate = Arc::new(Semaphore::new(IN_FLIGHT));
  while let Some(id) = rx.recv().await {
    // the id goes into a request: numbers only (the cache asked for it, but nothing here trusts that)
    if !crate::model::rooms::valid(&id) {
      continue;
    }
    let (http, out, gate) = (http.clone(), out.clone(), gate.clone());
    tokio::spawn(async move {
      let Ok(_slot) = gate.acquire().await else { return };
      let login = resolve(&http, &id).await;
      out.send(NetEvent::RoomLogin(id, login));
    });
  }
}

async fn resolve(http: &reqwest::Client, id: &str) -> Option<Box<str>> {
  let body = json!({ "query": "query($id:ID!){user(id:$id){login}}", "variables": { "id": id } });
  let req = http
    .post(crate::badges::fetch::GQL)
    .header("Client-Id", crate::badges::fetch::GQL_CLIENT_ID)
    .timeout(TIMEOUT)
    .body(body.to_string());
  let v = capped_json(req.send().await, MAX_BODY).await.ok()?;
  login_of(&v)
}

// `{"data":{"user":{"login":"name"}}}`; anything else is none
fn login_of(v: &Value) -> Option<Box<str>> {
  v.pointer("/data/user/login")
    .and_then(Value::as_str)
    .filter(|l| crate::model::ident::chatter_login(l))
    .map(Into::into)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_a_plain_login_is_taken_from_the_answer() {
    assert_eq!(login_of(&json!({"data": {"user": {"login": "partner_1"}}})).as_deref(), Some("partner_1"));
    for bad in [
      json!({"data": {"user": null}}),
      json!({"data": null}),
      json!({"data": {"user": {"login": 7}}}),
      json!({"data": {"user": {"login": "has space"}}}),
      json!({"data": {"user": {"login": "x".repeat(60)}}}),
      json!([1, 2]),
    ] {
      assert_eq!(login_of(&bad), None, "{bad}");
    }
  }
}
