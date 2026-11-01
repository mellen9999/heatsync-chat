// auto-multichat: a twitch channel's publicly linked kick/youtube accounts, and their recent history
use serde_json::Value;

use super::{NetEvent, ORIGIN, Out, hs_ws::valid_key, relay};
use crate::emotes::providers::fetch;
use crate::model::{chan, message::Platform};

pub async fn discover(http: reqwest::Client, tab: Box<str>, out: Out) {
  let v = match fetch(&http, &format!("{ORIGIN}/api/profile/{tab}?platform=twitch")).await {
    Ok(v) => v,
    Err(e) => return out.status(format!("links for {tab}: {e}")),
  };
  let p = v.get("profile").unwrap_or(&Value::Null);
  let s = |k: &str| p.get(k).and_then(Value::as_str).filter(|s| valid_key(s)).map(|s| s.to_ascii_lowercase().into());
  let kick = s("kick_username");
  // the same `@handle` key a saved `yt:` tab uses, or the feed is subscribed (and announced) twice
  let handle = s("youtube_username").and_then(|h: Box<str>| chan::yt_key(&h).ok()).filter(|h| !h.is_empty());
  let yt = handle.map(|h| (h.into(), s("youtube_channel_id")));
  if kick.is_some() || yt.is_some() {
    out.send(NetEvent::Links { tab, kick, yt });
  }
}

// /api/recent/<kick|youtube>/<key>: oldest first. kick by slug, youtube by lowercased UC id
pub async fn backfill(http: reqwest::Client, platform: Platform, key: Box<str>, archive: Box<str>, out: Out) {
  let (path, parse): (&str, fn(&Value) -> Option<crate::model::message::Message>) = match platform {
    Platform::Kick => ("kick", relay::kick),
    Platform::Youtube => ("youtube", relay::youtube),
    Platform::Twitch => return,
  };
  if !valid_key(&archive) {
    return;
  }
  match fetch(&http, &format!("{ORIGIN}/api/recent/{path}/{archive}?limit=50")).await {
    Ok(v) => {
      let msgs = relay::batch(v.get("messages"), parse);
      if !msgs.is_empty() {
        out.send(NetEvent::Relay { platform, key, msgs, history: true });
      }
    }
    Err(e) => out.status(format!("{path} history for {key}: {e}")),
  }
}
