// pin / unpin a line for everyone (ext pin-bar.js, BG `pin`): POST /api/mod/pin {platform, channel, message_id}
// and DELETE /api/mod/pin {platform, channel}, bearer. the server holds the rules (channel owner, or a
// twitch mod; kick mods are refused) and the line itself, from its own archive. the reply is capped
// and its pin goes through the same bounds as any pin frame
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out, pin as pins, social};
use crate::model::ident;
use crate::model::message::Platform;
use crate::model::pin::{self, Pin};

pub enum Op {
  Set { platform: Platform, chan: Box<str>, msg_id: Box<str> },
  Clear { platform: Platform, chan: Box<str> },
}

impl Op {
  fn room(&self) -> (Platform, &str) {
    match self {
      Op::Set { platform, chan, .. } | Op::Clear { platform, chan } => (*platform, chan),
    }
  }
}

// what the ui is told when it settles: the room, and the pin the server made (a set), never one we invented
pub struct Done {
  pub platform: Platform,
  pub chan: Box<str>,
  pub set: bool,
  pub res: Result<Option<Pin>, Fail>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
  // logged out, or the login was turned down
  Auth,
  // the twitch login lacks the `mod` pack (the moderated-channels lookup)
  Relink,
  // not the owner / a mod here
  Denied,
  // the line isn't in heatsync's archive yet
  NotArchived,
  Other,
}

pub async fn run(http: reqwest::Client, token: Option<String>, op: Op, out: Out) {
  let set = matches!(op, Op::Set { .. });
  let (platform, chan) = {
    let (p, c) = op.room();
    (p, Box::<str>::from(c))
  };
  let res = match token {
    None => Err(Fail::Auth),
    Some(t) => ask(&http, &t, &op).await,
  };
  out.send(NetEvent::PinDone(Box::new(Done { platform, chan, set, res })));
}

// the body, or None for a room/id that could bend it
fn body(op: &Op) -> Option<Value> {
  let (p, chan) = op.room();
  let plat = pins::name(p)?;
  let chan = chan.to_ascii_lowercase();
  if !ident::room_key(&chan) {
    return None;
  }
  Some(match op {
    Op::Set { msg_id, .. } if (1..=128).contains(&msg_id.len()) => {
      json!({ "platform": plat, "channel": chan, "message_id": &**msg_id })
    }
    Op::Set { .. } => return None,
    Op::Clear { .. } => json!({ "platform": plat, "channel": chan }),
  })
}

async fn ask(http: &reqwest::Client, token: &str, op: &Op) -> Result<Option<Pin>, Fail> {
  let b = body(op).ok_or(Fail::Other)?;
  let url = format!("{ORIGIN}/api/mod/pin");
  let req = match op {
    Op::Set { .. } => http.post(url),
    Op::Clear { .. } => http.delete(url),
  };
  let req = req.bearer_auth(token).header("content-type", "application/json").body(b.to_string());
  let (status, v) = social::call(req).await;
  classify(status, &v)
}

// 200 {pin} / {pin:null}; the ext's vocabulary for the rest (401 relink | auth, 403, 409, anything else)
pub fn classify(status: u16, v: &Value) -> Result<Option<Pin>, Fail> {
  match status {
    200..=299 => Ok(v.get("pin").filter(|p| !p.is_null()).and_then(pin::parse)),
    401
      if v.get("error").and_then(Value::as_str) == Some("relink_required")
        || v.get("relink_required").and_then(Value::as_bool) == Some(true) =>
    {
      Err(Fail::Relink)
    }
    401 => Err(Fail::Auth),
    403 => Err(Fail::Denied),
    409 => Err(Fail::NotArchived),
    _ => Err(Fail::Other),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn set(chan: &str, id: &str) -> Op {
    Op::Set { platform: Platform::Kick, chan: chan.into(), msg_id: id.into() }
  }

  #[test]
  fn bodies_name_the_room_and_only_a_set_names_a_line() {
    let b = body(&set("Some-One", "abc")).unwrap();
    assert_eq!(b, json!({ "platform": "kick", "channel": "some-one", "message_id": "abc" }));
    let c = body(&Op::Clear { platform: Platform::Twitch, chan: "forsen".into() }).unwrap();
    assert_eq!(c, json!({ "platform": "twitch", "channel": "forsen" }));
    // nothing that could bend the request
    assert!(body(&set("bad name", "abc")).is_none());
    assert!(body(&set("forsen", "")).is_none());
    assert!(body(&set("forsen", &"x".repeat(129))).is_none());
    assert!(body(&Op::Clear { platform: Platform::Youtube, chan: "x".into() }).is_none());
  }

  #[test]
  fn answers_in_the_exts_words() {
    let ok = json!({ "pin": { "id": "p1", "message_id": "m1", "content": "hi", "username": "bob" } });
    assert_eq!(&*classify(200, &ok).unwrap().unwrap().id, "p1");
    assert_eq!(classify(200, &json!({ "pin": null })), Ok(None));
    // a pin the bounds refuse is no pin
    assert_eq!(classify(200, &json!({ "pin": { "id": "" } })), Ok(None));
    assert_eq!(classify(401, &json!({ "error": "relink_required" })), Err(Fail::Relink));
    assert_eq!(classify(401, &json!({ "relink_required": true })), Err(Fail::Relink));
    assert_eq!(classify(401, &json!({ "error": "x" })), Err(Fail::Auth));
    assert_eq!(classify(403, &Value::Null), Err(Fail::Denied));
    assert_eq!(classify(409, &Value::Null), Err(Fail::NotArchived));
    assert_eq!(classify(429, &Value::Null), Err(Fail::Other));
    assert_eq!(classify(0, &Value::Null), Err(Fail::Other));
  }
}
