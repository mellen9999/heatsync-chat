// chatters' pictures for the lines on screen (ext resolve_avatar_url): twitch's public web gql, kick's
// public channel read. at most five asks in the air, each 3s at most, each answer small and checked
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::sync::mpsc::UnboundedReceiver;

use super::http::capped;
use super::{NetEvent, Out};
use crate::model::message::Platform;

// the ext's MAX_AVATAR_FETCHES
const IN_FLIGHT: usize = 5;
const TIMEOUT: Duration = Duration::from_secs(3);
// a profile answer is a few KB; a kick channel a few dozen
const MAX_BODY: usize = 256 << 10;

pub async fn run(http: reqwest::Client, mut rx: UnboundedReceiver<(Platform, Box<str>)>, out: Out) {
  let gate = Arc::new(Semaphore::new(IN_FLIGHT));
  while let Some((p, login)) = rx.recv().await {
    let (http, out, gate) = (http.clone(), out.clone(), gate.clone());
    tokio::spawn(async move {
      let Ok(_slot) = gate.acquire().await else { return };
      let url = resolve(&http, p, &login).await;
      out.send(NetEvent::Avatar(p, login, url));
    });
  }
}

async fn json_of(res: Result<reqwest::Response, reqwest::Error>) -> Value {
  match capped(res, MAX_BODY).await {
    Ok(b) => serde_json::from_slice(&b).unwrap_or(Value::Null),
    Err(_) => Value::Null,
  }
}

// kick keys a channel by its slug, and the slug is the name with `_` written `-`: a chatter called a_b is
// `a-b` there and a 404 as typed. a few older channels kept the underscore, so it is the
// second ask, not a replacement. a fifth of the kick lines in a 15 minute capture (801 of 3778) carry one
fn kick_slugs(login: &str) -> Vec<String> {
  if login.contains('_') { vec![login.replace('_', "-"), login.into()] } else { vec![login.into()] }
}

async fn resolve(http: &reqwest::Client, p: Platform, login: &str) -> Option<Box<str>> {
  if !crate::avatars::valid(login) {
    return None;
  }
  match p {
    Platform::Twitch => {
      let body = json!({
        "query": "query($login:String!){user(login:$login){profileImageURL(width:70)}}",
        "variables": { "login": login },
      });
      let req = http
        .post(crate::badges::fetch::GQL)
        .header("Client-Id", crate::badges::fetch::GQL_CLIENT_ID)
        .timeout(TIMEOUT)
        .body(body.to_string());
      crate::model::card::twitch_avatar(&json_of(req.send().await).await)
    }
    Platform::Kick => {
      for slug in kick_slugs(login) {
        let req = http.get(format!("https://kick.com/api/v2/channels/{slug}")).timeout(TIMEOUT);
        let v = json_of(req.send().await).await;
        let pic = v.pointer("/user/profile_pic").and_then(Value::as_str).and_then(crate::model::card::avatar_url);
        if pic.is_some() {
          return pic;
        }
      }
      None
    }
    Platform::Youtube => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // a fifth of the kick lines in a real capture carry an underscore name, and kick answers 404 for `a_b` where
  // the channel is keyed `a-b` (names redacted, shape kept)
  #[test]
  fn a_kick_name_with_an_underscore_asks_for_its_dashed_slug_first() {
    assert_eq!(kick_slugs("redacted_one"), ["redacted-one", "redacted_one"]);
    assert_eq!(kick_slugs("re_da_cted"), ["re-da-cted", "re_da_cted"]);
    // nothing to translate: one ask, as typed
    assert_eq!(kick_slugs("redacted1"), ["redacted1"]);
  }
}
