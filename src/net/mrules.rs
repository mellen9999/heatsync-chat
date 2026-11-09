// your mention rules through heatsync (ext multichat/mention-rules.js): GET / POST /api/mention-rules and
// PUT / DELETE /api/mention-rules/<id>. the answer is read as hostile json (model::mrules). the heatsync
// session's token is the only credential; it never reaches a log or a screen
use crate::i18n::t;
use reqwest::Method;
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out};
use crate::model::mrules::{self, Body, Rule};

// 100 rules of 200 chars and a few scope lists, with room for the json around them
const MAX_BODY: usize = 2 << 20;

pub enum Op {
  List,
  Add(Body),
  Toggle { id: u64, on: bool },
  Remove(u64),
}

pub enum Got {
  List(Vec<Rule>),
  // added, switched or removed: the list is read again
  Changed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fail {
  // logged out, or the heatsync session lapsed
  Auth,
  // the server's own reason (the cap for your account, a regex it will not take)
  Said(String),
  // the rule is gone already
  Gone,
  Limited,
  Other,
}

pub fn words(f: &Fail) -> String {
  match f {
    Fail::Auth => t("net2.mrules.auth").into(),
    Fail::Said(s) => s.clone(),
    Fail::Gone => t("net2.mrules.gone").into(),
    Fail::Limited => t("net2.mrules.limited").into(),
    Fail::Other => t("net2.mrules.other").into(),
  }
}

pub async fn run(http: reqwest::Client, token: Option<String>, op: Op, out: Out) {
  let res = match token {
    None => Err(Fail::Auth),
    Some(t) => ask(&http, &t, op).await,
  };
  out.send(NetEvent::MentionRules(res));
}

async fn ask(http: &reqwest::Client, token: &str, op: Op) -> Result<Got, Fail> {
  let url = format!("{ORIGIN}/api/mention-rules");
  let one = |id: u64| format!("{url}/{id}");
  let req = match &op {
    Op::List => http.get(&url),
    Op::Add(b) => {
      http.request(Method::POST, &url).header("content-type", "application/json").body(b.json().to_string())
    }
    Op::Toggle { id, on } => http
      .request(Method::PUT, one(*id))
      .header("content-type", "application/json")
      .body(json!({ "enabled": on }).to_string()),
    Op::Remove(id) => http.request(Method::DELETE, one(*id)),
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
    Op::List => mrules::list(&v).map(Got::List).ok_or(Fail::Other),
    _ => Ok(Got::Changed),
  }
}

pub fn classify(status: u16, v: &Value) -> Result<(), Fail> {
  if (200..300).contains(&status) {
    return Ok(());
  }
  Err(match status {
    401 => Fail::Auth,
    404 => Fail::Gone,
    429 => Fail::Limited,
    // the cap (403 plus_required, 400 alert_limit) and a bad rule (400): the server says why
    400 | 403 => mrules::said(v).map_or(Fail::Other, Fail::Said),
    _ => Fail::Other,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn answers_like_the_server_gives_them() {
    assert_eq!(classify(200, &json!({"rules": []})), Ok(()));
    assert_eq!(classify(401, &json!({"error": "unauthorized"})), Err(Fail::Auth));
    assert_eq!(classify(404, &json!({"error": "rule not found"})), Err(Fail::Gone));
    assert_eq!(classify(429, &Value::Null), Err(Fail::Limited));
    let cap = json!({"error": "free accounts run 5 alerts; plus runs 50", "error_code": "plus_required", "limit": 5});
    assert_eq!(classify(403, &cap), Err(Fail::Said("free accounts run 5 alerts; plus runs 50".into())));
    assert_eq!(classify(400, &json!({"error": "invalid regex: nope"})), Err(Fail::Said("invalid regex: nope".into())));
    // a refusal with no reason, or a server that is down, says nothing it could be blamed for
    assert_eq!(classify(400, &json!({"details": {"a": 1}})), Err(Fail::Other));
    assert_eq!(classify(403, &Value::Null), Err(Fail::Other));
    assert_eq!(classify(502, &Value::Null), Err(Fail::Other));
  }

  #[test]
  fn every_failure_has_plain_words() {
    for f in [Fail::Auth, Fail::Gone, Fail::Limited, Fail::Other, Fail::Said("x".into())] {
      assert!(!words(&f).is_empty() && words(&f) == words(&f).to_lowercase());
    }
  }
}
