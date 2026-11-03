// twitch's mod suite through heatsync (ext background.js mod_suite): POST /api/mod/warn, /shoutout and
// /shield, GET /api/mod/shield. needs the twitch `modsuite` permission pack; the server's helix call is
// the moderator check. the room is named by login
use serde_json::{Value, json};

use super::blocked::{Fail, classify};
use super::{NetEvent, ORIGIN, Out};
use crate::model::modsuite::Cmd;

pub async fn run(http: reqwest::Client, token: Option<String>, chan: Box<str>, cmd: Cmd, out: Out) {
  let res = match token {
    None => Err(Fail::Auth),
    Some(_) if crate::model::modsuite::login(&chan).as_deref() != Some(&*chan) => Err(Fail::Other),
    Some(t) => ask(&http, &t, &chan, &cmd).await,
  };
  out.send(NetEvent::Suite { chan, cmd, res });
}

// Ok(Some(on)) = shield mode is now `on`
async fn ask(http: &reqwest::Client, token: &str, chan: &str, cmd: &Cmd) -> Result<Option<bool>, Fail> {
  let post = |path: &str, body: Value| {
    http
      .post(format!("{ORIGIN}/api/mod/{path}"))
      .bearer_auth(token)
      .header("content-type", "application/json")
      .body(body.to_string())
  };
  let (status, v) = match cmd {
    Cmd::Warn { user, reason } => {
      super::social::call(post("warn", json!({ "channel": chan, "user": &**user, "reason": reason }))).await
    }
    Cmd::Shoutout(user) => super::social::call(post("shoutout", json!({ "channel": chan, "to_user": &**user }))).await,
    Cmd::Shield(want) => {
      let on = match want {
        Some(on) => *on,
        None => {
          let get = http.get(format!("{ORIGIN}/api/mod/shield?channel={chan}")).bearer_auth(token);
          let (status, v) = super::social::call(get).await;
          classify(status, &v)?;
          !v.get("is_active").and_then(Value::as_bool).ok_or(Fail::Other)?
        }
      };
      let (status, v) = super::social::call(post("shield", json!({ "channel": chan, "is_active": on }))).await;
      classify(status, &v)?;
      return Ok(Some(v.get("is_active").and_then(Value::as_bool).unwrap_or(on)));
    }
  };
  classify(status, &v).map(|()| None)
}
