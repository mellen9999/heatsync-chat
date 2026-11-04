// a twitch room's chat modes through heatsync (ext chat-modes.js): POST /api/mod/chat-settings with
// helix's names. the server's helix call is the moderator check; twitch echoes the change back as a
// ROOMSTATE, which is what the status screen shows
use crate::model::ident;
use serde_json::json;

use super::modact::{Fail, classify};
use super::{NetEvent, ORIGIN, Out};
use crate::model::modes::Set;

pub async fn run(http: reqwest::Client, token: Option<String>, chan: Box<str>, set: Set, out: Out) {
  let res = match token {
    None => Err(Fail::Auth),
    // a twitch login: nothing that could bend the body
    Some(_) if !ident::twitch_login(&chan) => Err(Fail::Other("not a twitch channel".into())),
    Some(t) => ask(&http, &t, &chan, set).await,
  };
  out.send(NetEvent::ModesDone { chan, set, res });
}

async fn ask(http: &reqwest::Client, token: &str, chan: &str, set: Set) -> Result<(), Fail> {
  let body = json!({ "platform": "twitch", "channel": chan, "settings": set.body() });
  let req = http
    .post(format!("{ORIGIN}/api/mod/chat-settings"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(body.to_string());
  let (status, v) = super::social::call(req).await;
  classify(status, &v)
}

// /status's stream half: twitch's public web gql, no auth, 3s at most
pub async fn info(http: reqwest::Client, chan: Box<str>, out: Out) {
  let req = http
    .post(crate::badges::fetch::GQL)
    .header("Client-Id", crate::badges::fetch::GQL_CLIENT_ID)
    .timeout(std::time::Duration::from_secs(3))
    .body(crate::model::status::query(&chan).to_string());
  let (status, v) = super::social::call(req).await;
  let info = if status == 200 { crate::model::status::parse(&v) } else { None };
  out.send(NetEvent::StreamInfo { chan, info });
}
