// recent twitch history on join, via heatsync's robotty proxy (raw irc lines)
use serde::Deserialize;

use super::irc::{self, Chat};
use super::{NetEvent, ORIGIN, Out};

const LIMIT: usize = 800;

#[derive(Deserialize)]
struct Recent {
  #[serde(default)]
  messages: Vec<String>,
}

pub async fn fetch(http: reqwest::Client, ch: Box<str>, out: Out) {
  let url = format!(
    "{ORIGIN}/api/recent-messages/{ch}?limit={LIMIT}&hide_moderation_messages=false&hide_moderated_messages=false&clearchatToNotice=true"
  );
  let res = match http.get(url).send().await {
    Ok(r) if r.status().is_success() => r.bytes().await,
    Ok(r) => return out.status(format!("history for {ch}: http {}", r.status().as_u16())),
    Err(e) => return out.status(format!("history for {ch}: {e}")),
  };
  let Ok(body) = res else {
    return out.status(format!("history for {ch}: read failed"));
  };
  let Ok(recent) = serde_json::from_slice::<Recent>(&body) else {
    return out.status(format!("history for {ch}: bad response"));
  };
  let start = recent.messages.len().saturating_sub(LIMIT);
  let events: Vec<Chat> = recent.messages[start..]
    .iter()
    .filter_map(|raw| irc::parse(raw).and_then(|l| irc::to_chat(&l)))
    .filter(|(c, _)| *c == ch)
    .map(|(_, ev)| ev)
    .collect();
  out.send(NetEvent::Backfill(ch, events));
}
