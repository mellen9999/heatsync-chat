// sending to a live youtube chat. heatsync brokers the official data api (POST /api/youtube/chat/send):
// the line comes back through the heatsync relay like everyone else's, so there is no local echo
use serde_json::{Value, json};

use super::{ORIGIN, http};

// the reply to one youtube send
const MAX_REPLY: usize = 64 << 10;

// youtube's own cap on a chat message
pub const MAX: usize = crate::model::message::Platform::Youtube.max_chars();

// the plain line shown when the user has to allow posting first
pub const NEEDS_PERMISSION: &str = "youtube needs permission to post as you — press ctrl-l → allow youtube posting";

// the request body: a handle key as the tabs hold it (`@name`), and the text as typed
pub fn body(chan: &str, text: &str) -> Value {
  json!({ "channel": chan, "text": text })
}

// what to tell the user for a failure, from the server's `error_code` (or the bare http status)
pub fn explain(code: Option<&str>, status: u16) -> String {
  match code {
    Some("not_linked") => "link youtube first — press ctrl-l → link youtube".into(),
    Some("needs_post_permission") => NEEDS_PERMISSION.into(),
    Some("not_live") => "that youtube channel isn't live right now".into(),
    Some("chat_disabled") => "chat is turned off on this youtube stream".into(),
    Some("quota_exhausted") => "youtube posting is paused for today, try again tomorrow".into(),
    Some("rate_limited") => "slow down".into(),
    Some("rejected") => "youtube refused the message (slow mode, members only, or you're banned here)".into(),
    Some("upstream") => "youtube isn't answering, try again".into(),
    _ => format!("couldn't send: {}", http::plain_status(status)),
  }
}

// client-side check before a round trip: None = fine to send
pub fn too_long(text: &str) -> Option<String> {
  (text.chars().count() > MAX).then(|| format!("youtube messages are {MAX} characters at most"))
}

pub async fn send(http: &reqwest::Client, token: &str, chan: &str, text: &str) -> Result<(), String> {
  if let Some(why) = too_long(text) {
    return Err(why);
  }
  let r = http
    .post(format!("{ORIGIN}/api/youtube/chat/send"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(body(chan, text).to_string())
    .send()
    .await
    .map_err(|e| http::offline("heatsync", e))?;
  if r.status().is_success() {
    return Ok(());
  }
  let status = r.status().as_u16();
  let v: Value = http::read_json(r, MAX_REPLY).await.unwrap_or(Value::Null);
  Err(explain(v.get("error_code").and_then(Value::as_str), status))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn request_body_is_channel_and_text_only() {
    assert_eq!(body("@somebody", "gm"), json!({ "channel": "@somebody", "text": "gm" }));
  }

  #[test]
  fn every_server_code_has_plain_lowercase_copy() {
    for code in [
      "not_linked",
      "needs_post_permission",
      "not_live",
      "chat_disabled",
      "quota_exhausted",
      "rate_limited",
      "rejected",
      "upstream",
    ] {
      let s = explain(Some(code), 400);
      assert!(!s.starts_with("couldn't send"), "{code} fell through");
      assert_eq!(s, s.to_lowercase(), "{code}: lowercase");
      assert!(!s.ends_with('.'), "{code}: no trailing period");
    }
  }

  #[test]
  fn missing_permission_points_at_the_account_menu() {
    assert_eq!(
      explain(Some("needs_post_permission"), 403),
      "youtube needs permission to post as you — press ctrl-l → allow youtube posting"
    );
  }

  #[test]
  fn unknown_failures_stay_plain() {
    assert_eq!(explain(None, 401), "couldn't send: you're logged out, log in again (ctrl-l)");
    assert_eq!(explain(Some("???"), 500), "couldn't send: heatsync is having trouble, try again soon");
    assert_eq!(explain(None, 502), "couldn't send: heatsync is having trouble, try again soon");
  }

  #[test]
  fn length_is_counted_in_characters() {
    assert!(too_long(&"é".repeat(MAX)).is_none());
    assert!(too_long(&"é".repeat(MAX + 1)).is_some());
  }
}
