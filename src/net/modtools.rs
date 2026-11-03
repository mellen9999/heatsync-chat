// the mod tools screen through heatsync (ext background.js mod_suite): shield mode, unban requests,
// automod levels and chatters under /api/mod/*. needs the twitch `modsuite` pack; the room is a login
use serde_json::{Value, json};

use super::blocked::{Fail, classify};
use super::{NetEvent, ORIGIN, Out};
use crate::model::modtools::{self, LEVELS, Levels, MAX_LEVEL, Unban};

pub enum Op {
  // all four reads
  Load,
  Shield(bool),
  // (request id, approve)
  Resolve(Box<str>, bool),
  // (place in LEVELS, level)
  Level(usize, u8),
}

// a read that failed on its own reads None; the screen says so in its section
pub struct Loaded {
  pub shield: Option<bool>,
  pub levels: Option<Levels>,
  pub unbans: Option<Vec<Unban>>,
  pub chatters: Option<(u64, Vec<Box<str>>)>,
}

pub enum Got {
  Loaded(Box<Loaded>),
  Shield(bool),
  Resolved(Box<str>, bool),
  Levels(Levels),
}

pub async fn run(http: reqwest::Client, token: Option<String>, chan: Box<str>, op: Op, out: Out) {
  let res = match token {
    None => Err(Fail::Auth),
    Some(_) if crate::model::modsuite::login(&chan).as_deref() != Some(&*chan) => Err(Fail::Other),
    Some(t) => ask(&http, &t, &chan, op).await,
  };
  out.send(NetEvent::Tools { chan, res });
}

async fn ask(http: &reqwest::Client, token: &str, chan: &str, op: Op) -> Result<Got, Fail> {
  let get = |path: &'static str| async move {
    let req = http.get(format!("{ORIGIN}/api/mod/{path}channel={chan}")).bearer_auth(token);
    let (status, v) = super::social::call(req).await;
    classify(status, &v).map(|()| v)
  };
  let send = |m: reqwest::Method, path: &'static str, body: Value| async move {
    let req = http
      .request(m, format!("{ORIGIN}/api/mod/{path}"))
      .bearer_auth(token)
      .header("content-type", "application/json")
      .body(body.to_string());
    let (status, v) = super::social::call(req).await;
    classify(status, &v).map(|()| v)
  };
  match op {
    Op::Load => {
      let (s, l, u, c) =
        tokio::join!(get("shield?"), get("automod-settings?"), get("unban-requests?status=pending&"), get("chatters?"));
      // the gate (signed in, permission, a mod here) is the same for all four: its refusal is the answer
      for r in [&s, &l, &u, &c] {
        if let Err(f @ (Fail::Auth | Fail::Relink | Fail::NotMod)) = r {
          return Err(f.clone());
        }
      }
      Ok(Got::Loaded(Box::new(Loaded {
        shield: s.ok().and_then(|v| v.get("is_active").and_then(Value::as_bool)),
        levels: l.ok().map(|v| modtools::levels(&v)),
        unbans: u.ok().map(|v| modtools::unbans(&v)),
        chatters: c.ok().map(|v| modtools::chatters(&v)),
      })))
    }
    Op::Shield(on) => {
      let v = send(reqwest::Method::POST, "shield", json!({ "channel": chan, "is_active": on })).await?;
      Ok(Got::Shield(v.get("is_active").and_then(Value::as_bool).unwrap_or(on)))
    }
    Op::Resolve(id, approve) => {
      let status = if approve { "approved" } else { "denied" };
      send(reqwest::Method::POST, "unban-requests/resolve", json!({ "channel": chan, "id": &*id, "status": status }))
        .await?;
      Ok(Got::Resolved(id, approve))
    }
    Op::Level(i, n) => {
      let (key, _) = *LEVELS.get(i).ok_or(Fail::Other)?;
      let mut body = json!({ "channel": chan });
      body[key] = json!(n.min(MAX_LEVEL));
      let v = send(reqwest::Method::PUT, "automod-settings", body).await?;
      Ok(Got::Levels(modtools::levels(v.get("settings").unwrap_or(&Value::Null))))
    }
  }
}
