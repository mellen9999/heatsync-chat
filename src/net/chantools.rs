// the channel commands through heatsync (ext input.js): POST /api/mod/announce, POST|DELETE
// /api/mod/vip and /moderator, PUT /api/chat/color, and /api/mod/polls + /predictions. each needs its
// twitch permission pack (announce, channeltools, chatcolor); the server's helix call is the real check
// (announce: mods, roles / polls / predictions: the broadcaster). the room is named by login
use reqwest::Method;
use serde_json::{Value, json};

use super::blocked::{Fail, classify};
use super::{NetEvent, ORIGIN, Out};
use crate::model::chantools::{Cmd, MAX_OUTCOMES, id_ok, tidy};
use crate::model::modsuite::login;

// the rows read from a list: the newest few are all a live poll or prediction can be
const SCAN: usize = 20;

// what went wrong, and (for a 400) the server's own words, tamed
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Oops(pub Fail, pub Box<str>);

impl From<Fail> for Oops {
  fn from(f: Fail) -> Self {
    Oops(f, "".into())
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
  Plain,
  // your color as the server now has it
  Color(Box<str>),
  Poll(Box<str>),
  Ended(Box<str>),
  Predicted(Box<str>),
  Locked(Box<str>),
  Resolved(Box<str>),
  Canceled(Box<str>),
}

pub async fn run(http: reqwest::Client, token: Option<String>, chan: Box<str>, cmd: Cmd, out: Out) {
  let res = match token {
    None => Err(Fail::Auth.into()),
    Some(_) if !cmd.roomless() && login(&chan).as_deref() != Some(&*chan) => Err(Fail::Other.into()),
    Some(t) => ask(&http, &t, &chan, &cmd).await,
  };
  out.send(NetEvent::Chan { chan, cmd, res });
}

pub struct Pred {
  pub id: Box<str>,
  pub title: Box<str>,
  pub outcomes: Vec<(Box<str>, Box<str>)>,
}

// the newest poll still running: (id, title)
pub fn active_poll(v: &Value) -> Option<(Box<str>, Box<str>)> {
  let rows = v.get("polls")?.as_array()?;
  rows.iter().take(SCAN).find_map(|p| {
    let id = p.get("id")?.as_str().filter(|s| id_ok(s))?;
    (p.get("status")?.as_str()? == "ACTIVE")
      .then(|| (id.into(), tidy(p.get("title").and_then(Value::as_str).unwrap_or(""), 60)))
  })
}

// the newest prediction still open (or, with `locked_ok`, locked and waiting for a winner)
pub fn open_prediction(v: &Value, locked_ok: bool) -> Option<Pred> {
  let rows = v.get("predictions")?.as_array()?;
  rows.iter().take(SCAN).find_map(|p| {
    let id = p.get("id")?.as_str().filter(|s| id_ok(s))?;
    let status = p.get("status")?.as_str()?;
    if !(status == "ACTIVE" || status == "LOCKED" && locked_ok) {
      return None;
    }
    let outcomes = p
      .get("outcomes")?
      .as_array()?
      .iter()
      .take(MAX_OUTCOMES)
      .filter_map(|o| {
        let oid = o.get("id")?.as_str().filter(|s| id_ok(s))?;
        Some((oid.into(), tidy(o.get("title").and_then(Value::as_str).unwrap_or(""), 25)))
      })
      .collect();
    Some(Pred { id: id.into(), title: tidy(p.get("title").and_then(Value::as_str).unwrap_or(""), 45), outcomes })
  })
}

async fn call(req: reqwest::RequestBuilder) -> Result<Value, Oops> {
  let (status, v) = super::social::call(req).await;
  match classify(status, &v) {
    Ok(()) => Ok(v),
    // a 400 carries a reason worth showing (twitch's own for a color)
    Err(f) if status == 400 => {
      Err(Oops(f, super::http::server_words(v.get("error").and_then(Value::as_str).unwrap_or("")).into()))
    }
    Err(f) => Err(f.into()),
  }
}

async fn ask(http: &reqwest::Client, token: &str, chan: &str, cmd: &Cmd) -> Result<Done, Oops> {
  let send = |m: Method, path: &str, body: Value| {
    call(
      http
        .request(m, format!("{ORIGIN}{path}"))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(body.to_string()),
    )
  };
  let read = |path: &str| call(http.get(format!("{ORIGIN}/api/mod/{path}?channel={chan}")).bearer_auth(token));
  match cmd {
    Cmd::Announce { color, text } => {
      send(Method::POST, "/api/mod/announce", json!({ "channel": chan, "message": text, "color": color })).await?;
      Ok(Done::Plain)
    }
    Cmd::Vip(user, add) | Cmd::Mod(user, add) => {
      let path = if matches!(cmd, Cmd::Vip(..)) { "/api/mod/vip" } else { "/api/mod/moderator" };
      let m = if *add { Method::POST } else { Method::DELETE };
      send(m, path, json!({ "channel": chan, "user": &**user })).await?;
      Ok(Done::Plain)
    }
    Cmd::Color(want) => {
      let v = send(Method::PUT, "/api/chat/color", json!({ "color": want })).await?;
      let now = tidy(v.get("color").and_then(Value::as_str).unwrap_or(want), 20);
      Ok(Done::Color(now))
    }
    Cmd::Poll { title, choices, secs } => {
      let body = json!({ "channel": chan, "title": title, "choices": choices, "duration": secs });
      let v = send(Method::POST, "/api/mod/polls", body).await?;
      let shown = v
        .get("poll")
        .and_then(|p| p.get("title"))
        .and_then(Value::as_str)
        .map_or_else(|| tidy(title, 60), |t| tidy(t, 60));
      Ok(Done::Poll(shown))
    }
    Cmd::EndPoll => {
      let (id, title) = active_poll(&read("polls").await?).ok_or(Fail::Gone)?;
      let body = json!({ "channel": chan, "id": &*id, "status": "TERMINATED" });
      send(Method::PATCH, "/api/mod/polls", body).await?;
      Ok(Done::Ended(title))
    }
    Cmd::Prediction { title, outcomes, secs } => {
      let body = json!({ "channel": chan, "title": title, "outcomes": outcomes, "window": secs });
      let v = send(Method::POST, "/api/mod/predictions", body).await?;
      let shown = v
        .get("prediction")
        .and_then(|p| p.get("title"))
        .and_then(Value::as_str)
        .map_or_else(|| tidy(title, 45), |t| tidy(t, 45));
      Ok(Done::Predicted(shown))
    }
    Cmd::Lock | Cmd::Cancel | Cmd::Resolve(_) => {
      let p = open_prediction(&read("predictions").await?, !matches!(cmd, Cmd::Lock)).ok_or(Fail::Gone)?;
      let mut body = json!({ "channel": chan, "id": &*p.id });
      let done = match cmd {
        Cmd::Lock => {
          body["status"] = "LOCKED".into();
          Done::Locked(p.title.clone())
        }
        Cmd::Cancel => {
          body["status"] = "CANCELED".into();
          Done::Canceled(p.title.clone())
        }
        Cmd::Resolve(n) => {
          let Some((oid, name)) = p.outcomes.get(n - 1) else {
            return Err(Oops(Fail::Other, format!("no outcome {n}: it has {}", p.outcomes.len()).into()));
          };
          body["status"] = "RESOLVED".into();
          body["winning_outcome_id"] = (&**oid).into();
          Done::Resolved(name.clone())
        }
        _ => return Err(Fail::Other.into()),
      };
      send(Method::PATCH, "/api/mod/predictions", body).await?;
      Ok(done)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn finds_the_live_poll_and_prediction() {
    let polls = json!({"polls": [
      {"id": "p3", "status": "COMPLETED", "title": "old"},
      {"id": "bad id", "status": "ACTIVE", "title": "x"},
      {"id": "p1", "status": "ACTIVE", "title": "best\nemote", "choices": [{"id": "c"}]},
    ]});
    assert_eq!(active_poll(&polls), Some(("p1".into(), "best emote".into())));
    assert_eq!(active_poll(&json!({"polls": [{"id": "p", "status": "COMPLETED"}]})), None);
    assert_eq!(active_poll(&json!("junk")), None);
    let preds = json!({"predictions": [
      {"id": "d", "status": "RESOLVED", "outcomes": []},
      {"id": "a", "status": "LOCKED", "title": "win?", "outcomes": [{"id": "o1", "title": "yes"}, {"id": "o 2", "title": "x"}, {"id": "o3", "title": "no"}]},
    ]});
    assert!(open_prediction(&preds, false).is_none());
    let p = open_prediction(&preds, true).unwrap();
    assert_eq!((&*p.id, &*p.title), ("a", "win?"));
    assert_eq!(p.outcomes.len(), 2, "a bad outcome id is dropped");
    assert_eq!(&*p.outcomes[1].0, "o3");
  }

  #[test]
  fn bounds_what_the_server_sends() {
    let many: Vec<Value> = (0..40).map(|i| json!({"id": format!("o{i}"), "title": "t".repeat(500)})).collect();
    let v = json!({"predictions": [{"id": "a", "status": "ACTIVE", "title": "z".repeat(900), "outcomes": many}]});
    let p = open_prediction(&v, false).unwrap();
    assert_eq!(p.outcomes.len(), MAX_OUTCOMES);
    assert_eq!(p.title.chars().count(), 45);
    assert!(p.outcomes.iter().all(|(_, t)| t.chars().count() == 25));
    // only the newest SCAN rows are looked at
    let mut rows: Vec<Value> = (0..SCAN).map(|i| json!({"id": format!("r{i}"), "status": "COMPLETED"})).collect();
    rows.push(json!({"id": "late", "status": "ACTIVE", "title": "t"}));
    assert_eq!(active_poll(&json!({ "polls": rows })), None);
  }

  #[test]
  fn failures_map_like_the_rest() {
    let pack = json!({"relink_required": true, "scope_pack": "channeltools"});
    assert_eq!(classify(401, &pack), Err(Fail::Relink));
    assert_eq!(classify(403, &json!({"error": "only the broadcaster can do this"})), Err(Fail::NotMod));
    assert_eq!(classify(429, &Value::Null), Err(Fail::Limited));
  }
}
