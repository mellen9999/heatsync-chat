// sending to a live youtube chat. heatsync brokers the official data api (POST /api/youtube/chat/send):
// the line comes back through the heatsync relay like everyone else's, so there is no local echo
use crate::i18n::{f, t};
use serde_json::{Value, json};

use super::{ORIGIN, http};

// the reply to one youtube send
const MAX_REPLY: usize = 64 << 10;

// youtube's own cap on a chat message
pub const MAX: usize = crate::model::message::Platform::Youtube.max_chars();

// the plain line shown when the user has to allow posting first

// the request body: a handle key as the tabs hold it (`@name`), and the text as typed
// the line's send id rides along, as on kick: heatsync files the youtube leg under the same send as the
// others, so every device folds the feeds' copies into one row
pub fn body(chan: &str, text: &str, send: Option<&str>) -> Value {
  let mut b = json!({ "channel": chan, "text": text });
  if let Some(id) = send {
    b["send_id"] = json!(id);
  }
  b
}

// what to tell the user for a failure, from the server's `error_code` (or the bare http status)
pub fn explain(code: Option<&str>, status: u16) -> String {
  match code {
    Some("not_linked") => t("net1.yt.not_linked").into(),
    Some("needs_post_permission") => t("net1.yt.needs_permission").into(),
    Some("not_live") => t("net1.yt.not_live").into(),
    Some("chat_disabled") => t("net1.yt.chat_disabled").into(),
    Some("quota_exhausted") => t("net1.yt.quota").into(),
    Some("rate_limited") => t("net1.slow_down").into(),
    Some("rejected") => t("net1.yt.rejected").into(),
    Some("upstream") => t("net1.yt.upstream").into(),
    _ => f("net1.yt.send_failed", &[("why", &http::plain_status(status))]),
  }
}

// client-side check before a round trip: None = fine to send
pub fn too_long(text: &str) -> Option<String> {
  (text.chars().count() > MAX).then(|| f("net1.yt.too_long", &[("max", &MAX.to_string())]))
}

pub async fn send(
  http: &reqwest::Client,
  token: &str,
  chan: &str,
  text: &str,
  send: Option<&str>,
) -> Result<(), String> {
  if let Some(why) = too_long(text) {
    return Err(why);
  }
  let r = http
    .post(format!("{ORIGIN}/api/youtube/chat/send"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(body(chan, text, send).to_string())
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
  fn request_body_is_channel_text_and_the_send_id() {
    assert_eq!(body("@somebody", "gm", None), json!({ "channel": "@somebody", "text": "gm" }));
    assert_eq!(
      body("@somebody", "gm", Some("hsc-1")),
      json!({ "channel": "@somebody", "text": "gm", "send_id": "hsc-1" })
    );
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
