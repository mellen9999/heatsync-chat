// a kick line through heatsync (POST /api/kick/chat/send); it comes back through the relay like everyone's.
// there is no idempotency key, so a try is repeated only when it provably never landed (the ext's rule):
// kick refused it with a 4xx, or the connection never opened. a 5xx or a timeout after connecting may well
// be in chat, and is never posted twice
use crate::i18n::t;
use serde_json::{Value, json};
use std::time::Duration;

use super::ORIGIN;
use super::http;

const MAX_REPLY: usize = 64 << 10;

// the waits before the second and the third try
const BACKOFF: [Duration; 2] = [Duration::from_millis(750), Duration::from_millis(1500)];

pub async fn send(
  http: &reqwest::Client,
  token: &str,
  chan: &str,
  text: &str,
  reply: Option<&str>,
  send: Option<&str>,
) -> Result<(), String> {
  let mut body = json!({ "channel": chan, "text": text });
  if let Some(id) = reply {
    body["reply_to_message_id"] = json!(id);
  }
  // the line's send id, the same on every try: heatsync records this kick leg under the same send as the twitch one
  if let Some(id) = send {
    body["send_id"] = json!(id);
  }
  let body = body.to_string();
  retrying(&BACKOFF, || once(http, token, &body)).await
}

// one go: Err(why, whether a second go could help)
type Go = Result<(), (String, bool)>;

// `again` until it lands, gives up, or may have landed, waiting between tries
async fn retrying<F, Fut>(waits: &[Duration], mut again: F) -> Result<(), String>
where
  F: FnMut() -> Fut,
  Fut: Future<Output = Go>,
{
  let mut tried = 0;
  loop {
    match again().await {
      Ok(()) => return Ok(()),
      Err((_, true)) if tried < waits.len() => {
        tokio::time::sleep(waits[tried]).await;
        tried += 1;
      }
      Err((why, _)) => return Err(why),
    }
  }
}

async fn once(http: &reqwest::Client, token: &str, body: &str) -> Go {
  let r = match http
    .post(format!("{ORIGIN}/api/kick/chat/send"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(body.to_owned())
    .send()
    .await
  {
    Ok(r) => r,
    // never connected: nothing reached heatsync. any other failure (a reset or a timeout mid-way) may have
    Err(e) => {
      let never = e.is_connect();
      return Err((http::offline("heatsync", e), never));
    }
  };
  if r.status().is_success() {
    return Ok(());
  }
  let code = r.status().as_u16();
  let v: Value = http::read_json(r, MAX_REPLY).await.unwrap_or(Value::Null);
  let error_code = v.get("error_code").and_then(Value::as_str);
  let why = match error_code {
    Some("not_linked") => t("net1.session.kick_not_linked").into(),
    Some("scope_missing") => t("net1.session.kick_scope").into(),
    Some("token_expired") => t("net1.session.kick_expired").into(),
    Some("rate_limited") => t("net1.slow_down").into(),
    _ => match http::server_words(v.get("error").and_then(Value::as_str).unwrap_or("")) {
      w if w.is_empty() => http::plain_status(code),
      w => w,
    },
  };
  // a 4xx is kick turning the line down: not in chat. only a refusal that could pass on a second go is tried
  // again; one that needs the user (link, scope, login) will not
  let unsent = (400..500).contains(&code);
  let needs_user = matches!(error_code, Some("not_linked" | "scope_missing" | "token_expired"));
  Err((why, unsent && !needs_user))
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::cell::Cell;

  fn refused(again: bool) -> Go {
    Err(("no".into(), again))
  }

  async fn runs(answers: Vec<Go>) -> (usize, Result<(), String>) {
    let calls = Cell::new(0);
    let mut answers = answers.into_iter();
    let got = retrying(&[Duration::ZERO; 2], || {
      calls.set(calls.get() + 1);
      std::future::ready(answers.next().unwrap())
    })
    .await;
    (calls.get(), got)
  }

  #[tokio::test]
  async fn a_refusal_that_never_landed_is_tried_three_times_then_given_up() {
    let (n, got) = runs(vec![refused(true), refused(true), refused(true)]).await;
    assert_eq!((n, got.is_err()), (3, true));
  }

  #[tokio::test]
  async fn a_second_try_that_lands_ends_it() {
    let (n, got) = runs(vec![refused(true), Ok(())]).await;
    assert_eq!((n, got.is_ok()), (2, true));
  }

  #[tokio::test]
  async fn an_unsure_or_user_bound_failure_is_not_posted_again() {
    // a 5xx or a drop after connecting may be in chat; a missing link or scope no second go fixes
    let (n, got) = runs(vec![refused(false)]).await;
    assert_eq!((n, got.is_err()), (1, true));
  }
}
