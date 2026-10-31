// heatsync's account-bound calls the whispers and feed tabs make. each command is one task; the
// session hands it the current token (None = logged out). answers go back as NetEvents
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out};
use crate::model::whisper::{self, Fail, Who};
use crate::ui::theme;

const MAX_BODY: usize = 4 << 20;
// conversations opened when the whispers tab first shows, and messages read from each
const CONVERSATIONS: usize = 8;
const PER_CONVERSATION: usize = 40;

pub enum Cmd {
  WhisperHistory { me: u64 },
  // `id` is the twitch user id when we already know it, else it is looked up from the login
  WhisperSend { key: u64, login: Box<str>, id: Option<Box<str>>, text: String },
}

pub async fn run(http: reqwest::Client, token: Option<String>, cmd: Cmd, out: Out) {
  match cmd {
    Cmd::WhisperHistory { me } => {
      if let Some(t) = token {
        history(&http, &t, me, &out).await;
      }
    }
    Cmd::WhisperSend { key, login, id, text } => {
      let res = match token {
        Some(t) => send(&http, &t, &login, id, &text).await,
        None => Err(Fail::Auth),
      };
      out.send(NetEvent::WhisperSent { key, res });
    }
  }
}

// status + json body, the body capped. a network failure is status 0
async fn call(req: reqwest::RequestBuilder) -> (u16, Value) {
  let Ok(mut r) = req.send().await else { return (0, Value::Null) };
  let status = r.status().as_u16();
  if r.content_length().is_some_and(|n| n > MAX_BODY as u64) {
    return (status, Value::Null);
  }
  let mut buf = Vec::new();
  while let Ok(Some(c)) = r.chunk().await {
    if buf.len() + c.len() > MAX_BODY {
      return (status, Value::Null);
    }
    buf.extend_from_slice(&c);
  }
  (status, serde_json::from_slice(&buf).unwrap_or(Value::Null))
}

fn get(http: &reqwest::Client, token: &str, path: &str) -> reqwest::RequestBuilder {
  http.get(format!("{ORIGIN}{path}")).bearer_auth(token)
}

async fn history(http: &reqwest::Client, token: &str, me: u64, out: &Out) {
  let (status, list) = call(get(http, token, "/api/whispers")).await;
  if status != 200 {
    return out.status(format!("whispers: {}", fail_text(&classify(status, &list))));
  }
  let mut all = Vec::new();
  let convos = list.get("conversations").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  for c in convos.iter().take(CONVERSATIONS) {
    let Some(id) =
      c.get("other_twitch_id").and_then(|v| v.as_str().map(String::from).or_else(|| v.as_u64().map(|n| n.to_string())))
    else {
      continue;
    };
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
      continue;
    }
    let login: Box<str> = c
      .get("other_twitch_login")
      .and_then(Value::as_str)
      .unwrap_or("")
      .to_lowercase()
      .chars()
      .take(25)
      .collect::<String>()
      .into();
    let name: Box<str> = c
      .get("other_twitch_display")
      .and_then(Value::as_str)
      .filter(|s| !s.is_empty())
      .unwrap_or(&login)
      .chars()
      .take(100)
      .collect::<String>()
      .into();
    let color = c
      .get("other_color")
      .and_then(Value::as_str)
      .and_then(crate::model::color::parse_hex)
      .map_or(theme::WHITE, crate::model::color::readable);
    let who = Who { id: id.clone().into(), login, name, color };
    let (status, thread) = call(get(http, token, &format!("/api/whispers/{id}?limit={PER_CONVERSATION}"))).await;
    if status != 200 {
      continue;
    }
    let rows = thread.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    all.extend(rows.iter().take(PER_CONVERSATION).filter_map(|m| whisper::parse(m, me, Some(&who))));
  }
  all.sort_by_key(|w| w.ts);
  out.send(NetEvent::Whispers { items: all, live: false });
}

async fn send(http: &reqwest::Client, token: &str, login: &str, id: Option<Box<str>>, text: &str) -> Result<Who, Fail> {
  let who = match id {
    Some(id) => Who { id, login: login.into(), name: login.into(), color: theme::WHITE },
    None => resolve(http, login).await?,
  };
  let req = http
    .post(format!("{ORIGIN}/api/twitch/whisper"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(json!({ "toUserId": &*who.id, "message": text }).to_string());
  let (status, v) = call(req).await;
  if (200..300).contains(&status) && v.get("error").is_none() { Ok(who) } else { Err(classify(status, &v)) }
}

async fn resolve(http: &reqwest::Client, login: &str) -> Result<Who, Fail> {
  let (status, v) = call(http.get(format!("{ORIGIN}/api/resolve/twitch/{login}"))).await;
  if status == 404 {
    return Err(Fail::Other(format!("twitch user \"{login}\" not found").into()));
  }
  let id = v.get("id").and_then(|i| i.as_str().map(String::from).or_else(|| i.as_u64().map(|n| n.to_string())));
  match id.filter(|i| !i.is_empty() && i.len() <= 20 && i.bytes().all(|b| b.is_ascii_digit())) {
    Some(id) if status == 200 => {
      let name = v.get("displayName").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(login);
      Ok(Who {
        id: id.into(),
        login: login.into(),
        name: name.chars().take(100).collect::<String>().into(),
        color: theme::WHITE,
      })
    }
    _ => Err(Fail::Other("failed to resolve twitch user".into())),
  }
}

fn fail_text(f: &Fail) -> String {
  whisper::fail_text(f)
}

// why a send failed, by the ext's rules (whispers.js sendTwitchWhisper): a missing whispers permission
// (the server flags it, or says re-link) beats a plain logged-out answer; 5xx or no answer is the server
pub fn classify(status: u16, v: &Value) -> Fail {
  let err = v.get("error").and_then(Value::as_str).unwrap_or("");
  let low = err.to_ascii_lowercase();
  let flagged = status == 401
    && (v.get("relink_required").and_then(Value::as_bool) == Some(true)
      || v.get("scope_pack").and_then(Value::as_str) == Some("whispers"));
  if flagged || low.contains("relink") || low.contains("re-link") {
    Fail::Relink
  } else if status == 401 && ["no twitch token", "not authenticated", "re-login"].iter().any(|k| low.contains(k)) {
    Fail::Auth
  } else if status == 0 || status >= 500 {
    Fail::Server
  } else if err.is_empty() {
    Fail::Other(format!("twitch error {status}").into())
  } else {
    Fail::Other(err.chars().take(100).collect::<String>().into())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn failures_are_classified_like_the_ext() {
    let relink = json!({"error": "No Twitch token — re-link on heatsync.org", "relink_required": true});
    assert_eq!(classify(401, &relink), Fail::Relink);
    assert_eq!(classify(403, &json!({"error": "Please re-link twitch"})), Fail::Relink);
    assert_eq!(classify(401, &json!({"error": "Not authenticated with Twitch"})), Fail::Auth);
    assert_eq!(classify(0, &Value::Null), Fail::Server);
    assert_eq!(classify(502, &json!({})), Fail::Server);
    assert_eq!(classify(400, &json!({"error": "Message too long"})), Fail::Other("Message too long".into()));
    assert_eq!(classify(429, &Value::Null), Fail::Other("twitch error 429".into()));
    // a bare 401 with an unknown reason is shown as the server said it
    assert_eq!(classify(401, &json!({"error": "nope"})), Fail::Other("nope".into()));
  }
}
