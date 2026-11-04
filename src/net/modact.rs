// mod actions through heatsync (POST /api/mod/action). the server holds the twitch / kick mod
// tokens; we only name the room, the person and what to do. twitch wants numeric ids, kick takes
// the channel slug and the login
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out, http, social};
use crate::model::message::Platform;
use crate::model::modcmd::Verb;

// the reply to one mod action
const MAX_REPLY: usize = 64 << 10;

pub struct Req {
  pub platform: Platform,
  // twitch login / kick slug the action happens in
  pub chan: Box<str>,
  // twitch room id when known (ROOMSTATE); looked up from the login otherwise
  pub room: Option<Box<str>>,
  pub verb: Verb,
  // the person (login); for a delete only used in the "did it" line
  pub login: Box<str>,
  pub user_id: Option<Box<str>>,
  pub msg_id: Option<Box<str>>,
  pub reason: Option<String>,
}

// what the ui is told when it settles
pub struct Done {
  pub platform: Platform,
  pub chan: Box<str>,
  pub verb: Verb,
  pub login: Box<str>,
  pub msg_id: Option<Box<str>>,
  pub res: Result<(), Fail>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
  // logged out
  Auth,
  // the twitch login lacks the moderation permission pack: re-link twitch with `mod`
  Relink,
  Other(String),
}

fn body(r: &Req, room: &str, user: Option<&str>) -> Value {
  let mut b =
    json!({ "action": r.verb.action(), "platform": match r.platform { Platform::Kick => "kick", _ => "twitch" } });
  match r.platform {
    Platform::Kick => {
      b["channel"] = json!(&*r.chan);
      if r.verb != Verb::Delete {
        b["target_login"] = json!(&*r.login);
      }
    }
    _ => {
      b["broadcaster_id"] = json!(room);
      if let Some(u) = user {
        b["target_user_id"] = json!(u);
      }
    }
  }
  if let Some(id) = &r.msg_id
    && r.verb == Verb::Delete
  {
    b["message_id"] = json!(&**id);
  }
  if let Some(s) = r.verb.seconds() {
    // kick's shortest timeout is a minute
    b["duration_seconds"] = json!(if r.platform == Platform::Kick { s.max(60) } else { s });
  }
  if let Some(why) = &r.reason {
    b["reason"] = json!(why);
  }
  b
}

pub async fn run(http: reqwest::Client, token: Option<String>, r: Req, out: Out) {
  let res = match token {
    None => Err(Fail::Auth),
    Some(t) => act(&http, &t, &r).await,
  };
  out.send(NetEvent::ModDone(Box::new(Done {
    platform: r.platform,
    chan: r.chan,
    verb: r.verb,
    login: r.login,
    msg_id: r.msg_id,
    res,
  })));
}

async fn id_of(http: &reqwest::Client, login: &str) -> Result<Box<str>, Fail> {
  social::resolve(http, login).await.map(|w| w.id).map_err(|_| Fail::Other(format!("couldn't find {login} on twitch")))
}

async fn act(http: &reqwest::Client, token: &str, r: &Req) -> Result<(), Fail> {
  let (mut room, mut user) = (String::new(), None::<Box<str>>);
  if r.platform != Platform::Kick {
    room = match &r.room {
      Some(id) => id.to_string(),
      None => id_of(http, &r.chan).await?.to_string(),
    };
    if r.verb != Verb::Delete {
      user = Some(match &r.user_id {
        Some(id) => id.clone(),
        None => id_of(http, &r.login).await?,
      });
    }
  }
  let resp = http
    .post(format!("{ORIGIN}/api/mod/action"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(body(r, &room, user.as_deref()).to_string())
    .send()
    .await
    .map_err(|e| Fail::Other(http::offline("heatsync", e)))?;
  let status = resp.status().as_u16();
  let v: Value = http::read_json(resp, MAX_REPLY).await.unwrap_or(Value::Null);
  classify(status, &v)
}

pub fn classify(status: u16, v: &Value) -> Result<(), Fail> {
  if (200..300).contains(&status) && v.get("success").and_then(Value::as_bool) != Some(false) {
    return Ok(());
  }
  let err = v.get("error").and_then(Value::as_str).unwrap_or("");
  if v.get("relink_required").and_then(Value::as_bool) == Some(true) {
    return Err(Fail::Relink);
  }
  Err(match status {
    401 => Fail::Auth,
    429 => Fail::Other("slow down".into()),
    0 => Fail::Other("no answer from heatsync".into()),
    403 => Fail::Other("you're not a mod here".into()),
    _ if err.is_empty() => Fail::Other(http::plain_status(status)),
    _ => Fail::Other(err.chars().take(100).collect()),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn req(platform: Platform, verb: Verb) -> Req {
    Req {
      platform,
      chan: "forsen".into(),
      room: Some("22484632".into()),
      verb,
      login: "bob".into(),
      user_id: Some("77".into()),
      msg_id: Some("abc".into()),
      reason: Some("spam".into()),
    }
  }

  #[test]
  fn twitch_body_names_ids() {
    let b = body(&req(Platform::Twitch, Verb::Timeout(600)), "22484632", Some("77"));
    assert_eq!(b["platform"], "twitch");
    assert_eq!(b["action"], "timeout");
    assert_eq!(b["broadcaster_id"], "22484632");
    assert_eq!(b["target_user_id"], "77");
    assert_eq!(b["duration_seconds"], 600);
    assert_eq!(b["reason"], "spam");
    assert!(b.get("message_id").is_none());
    let d = body(&req(Platform::Twitch, Verb::Delete), "22484632", None);
    assert_eq!(d["action"], "delete_message");
    assert_eq!(d["message_id"], "abc");
  }

  #[test]
  fn kick_body_names_the_slug_and_login() {
    let b = body(&req(Platform::Kick, Verb::Purge), "", None);
    assert_eq!(b["platform"], "kick");
    assert_eq!(b["channel"], "forsen");
    assert_eq!(b["target_login"], "bob");
    // kick's shortest timeout is a minute
    assert_eq!(b["duration_seconds"], 60);
    assert!(b.get("broadcaster_id").is_none());
  }

  #[test]
  fn answers() {
    assert_eq!(classify(200, &json!({ "success": true })), Ok(()));
    assert_eq!(classify(401, &json!({ "relink_required": true, "scope_pack": "mod" })), Err(Fail::Relink));
    assert_eq!(classify(401, &json!({ "error": "x" })), Err(Fail::Auth));
    assert_eq!(classify(403, &json!({})), Err(Fail::Other("you're not a mod here".into())));
    assert_eq!(
      classify(400, &json!({ "error": "could not find kick user" })),
      Err(Fail::Other("could not find kick user".into()))
    );
  }
}
