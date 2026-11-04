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
      let req = http.get(format!("https://kick.com/api/v2/channels/{login}")).timeout(TIMEOUT);
      let v = json_of(req.send().await).await;
      v.pointer("/user/profile_pic").and_then(Value::as_str).and_then(crate::model::card::avatar_url)
    }
    Platform::Youtube => None,
  }
}
