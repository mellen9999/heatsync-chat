// a twitch clip of a live channel, from the user card's `clip` row (ext profile-card.js pcDoClip):
// POST /api/twitch/clip {channel} with the bearer. the server holds the twitch token and makes the
// clip; we get back where to watch it and where to edit it. both links are hostile: only https twitch
// pages are ever copied or opened
use crate::i18n::t;
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out, social};
use crate::model::ident;

// what a made clip links to; either may be missing, or refused as not a twitch page
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Made {
  pub clip_url: Option<Box<str>>,
  pub edit_url: Option<Box<str>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fail {
  // logged out
  Auth,
  // the twitch login lacks the `clips` permission pack
  Relink,
  // heatsync's own words (rate limit, channel not found, …), cleaned and cut
  Other(String),
}

const MAX_URL: usize = 2048;
const MAX_WORDS: usize = 100;

pub async fn run(http: reqwest::Client, token: Option<String>, login: Box<str>, chan: Box<str>, out: Out) {
  let res = match token {
    None => Err(Fail::Auth),
    // the server only takes a twitch login here: nothing that could bend the body
    Some(_) if !ident::twitch_login(&chan) => Err(Fail::Other(t("net2.modact.not_twitch").into())),
    Some(t) => ask(&http, &t, &chan).await,
  };
  out.send(NetEvent::Clip { login, res });
}

async fn ask(http: &reqwest::Client, token: &str, chan: &str) -> Result<Made, Fail> {
  let req = http
    .post(format!("{ORIGIN}/api/twitch/clip"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(json!({ "channel": chan }).to_string());
  let (status, v) = social::call(req).await;
  classify(status, &v)
}

// 200 {success, clip_url, edit_url}; 401 {relink_required, scope_pack:"clips"} → relink; the rest says its own words
pub fn classify(status: u16, v: &Value) -> Result<Made, Fail> {
  let relink = v.get("relink_required").and_then(Value::as_bool) == Some(true)
    && v.get("scope_pack").and_then(Value::as_str) == Some("clips");
  if status == 401 && relink {
    return Err(Fail::Relink);
  }
  if (200..300).contains(&status) && v.get("success").and_then(Value::as_bool) != Some(false) {
    let url = |k: &str| v.get(k).and_then(Value::as_str).and_then(twitch_page);
    return Ok(Made { clip_url: url("clip_url"), edit_url: url("edit_url") });
  }
  let words = super::http::server_words(v.get("error").and_then(Value::as_str).unwrap_or(""));
  Err(Fail::Other(match (status, words.as_str()) {
    (0, _) => t("net2.http.no_answer").into(),
    (_, "") => super::http::plain_status(status),
    (_, w) => w.chars().take(MAX_WORDS).collect(),
  }))
}

// an https page on twitch.tv or clips.twitch.tv (www. and the like too), no login in it, no odd port
pub fn twitch_page(u: &str) -> Option<Box<str>> {
  if u.len() > MAX_URL || u.chars().any(char::is_control) {
    return None;
  }
  let url = reqwest::Url::parse(u).ok()?;
  let host = url.host_str()?;
  let ours = host == "twitch.tv" || host.ends_with(".twitch.tv");
  (url.scheme() == "https" && ours && url.username().is_empty() && url.password().is_none() && url.port().is_none())
    .then(|| url.as_str().into())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_https_twitch_pages_pass() {
    for ok in [
      "https://clips.twitch.tv/AbCd-123",
      "https://www.twitch.tv/forsen/clip/AbCd/edit",
      "https://twitch.tv/x",
      "https://clips.twitch.tv/x/edit",
    ] {
      assert!(twitch_page(ok).is_some(), "{ok}");
    }
    for bad in [
      "http://clips.twitch.tv/x",
      "https://evil.com/clips.twitch.tv",
      "https://clips.twitch.tv.evil.com/x",
      "https://nottwitch.tv/x",
      "https://user:pw@clips.twitch.tv/x",
      "https://clips.twitch.tv@evil.com/x",
      "https://clips.twitch.tv:8443/x",
      "javascript:alert(1)",
      "https://clips.twitch.tv/x\n",
      "https://clips.twitch.tv/\u{7}x",
      "clips.twitch.tv/x",
      "",
    ] {
      assert!(twitch_page(bad).is_none(), "{bad:?}");
    }
    assert!(twitch_page(&format!("https://clips.twitch.tv/{}", "a".repeat(MAX_URL))).is_none());
  }

  #[test]
  fn a_made_clip_keeps_only_links_it_trusts() {
    let v = json!({ "success": true, "clip_id": "x", "clip_url": "https://clips.twitch.tv/x",
      "edit_url": "https://evil.com/edit" });
    let m = classify(200, &v).unwrap();
    assert_eq!(m.clip_url.as_deref(), Some("https://clips.twitch.tv/x"));
    assert_eq!(m.edit_url, None);
    // a success with no links is still a clip
    assert_eq!(classify(200, &json!({ "success": true })), Ok(Made::default()));
    assert_eq!(classify(200, &json!({ "success": true, "clip_url": 5 })), Ok(Made::default()));
  }

  #[test]
  fn answers() {
    let relink = json!({ "error": "twitch clip permission not enabled", "relink_required": true,
      "platform": "twitch", "scope_pack": "clips" });
    assert_eq!(classify(401, &relink), Err(Fail::Relink));
    // a different pack is no business of the clip button
    assert_eq!(
      classify(401, &json!({ "error": "x", "relink_required": true, "scope_pack": "mod" })),
      Err(Fail::Other("x".into()))
    );
    assert_eq!(
      classify(401, &json!({ "error": "Not authenticated with Twitch" })),
      Err(Fail::Other("you're logged out, log in again (ctrl-l)".into()))
    );
    assert_eq!(
      classify(429, &json!({ "error": "Clip rate limit \u{2014} try again in a minute" })),
      Err(Fail::Other("slow down, try again in a minute".into()))
    );
    assert_eq!(classify(200, &json!({ "success": false, "error": "nope" })), Err(Fail::Other("nope".into())));
    assert_eq!(classify(0, &Value::Null), Err(Fail::Other("no answer from heatsync".into())));
    assert_eq!(classify(500, &Value::Null), Err(Fail::Other(super::super::http::plain_status(500))));
    // the words are cleaned and cut
    let long = "x\n".repeat(200);
    let Err(Fail::Other(w)) = classify(400, &json!({ "error": long })) else { panic!("fails") };
    assert!(w.chars().count() <= MAX_WORDS && !w.contains('\n'));
  }
}
