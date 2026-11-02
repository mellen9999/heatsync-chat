// recent twitch history on join, via heatsync's robotty proxy (raw irc lines)
use std::collections::HashMap;

use serde::Deserialize;

use super::irc::{self, Chat};
use super::{NetEvent, ORIGIN, Out};
use crate::model::message::REPLY_TEXT;
use crate::util::clamp;

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
  out.send(NetEvent::Backfill(ch.clone(), events(&recent.messages[start..], &ch)));
}

// the channel's lines, oldest first. heatsync's archive keeps a reply's parent id but not its name or
// words: those come from the parent line, when it is in the same history
fn events(raw: &[String], ch: &str) -> Vec<Chat> {
  let mut seen: HashMap<Box<str>, (Box<str>, Box<str>)> = HashMap::new();
  let mut out = Vec::with_capacity(raw.len());
  for r in raw {
    let Some(l) = irc::parse(r) else { continue };
    let Some((c, mut ev)) = irc::to_chat(&l) else { continue };
    if *c != *ch {
      continue;
    }
    if let Chat::Msg(m) = &mut ev {
      if m.reply_to.is_none()
        && let Some((nick, body)) = l.tag("reply-parent-msg-id").and_then(|id| seen.get(id.as_str()))
      {
        m.reply_to = Some(nick.clone());
        m.reply_text = Some(clamp(body, REPLY_TEXT).into());
      }
      if !m.id.is_empty() {
        seen.insert(m.id.clone(), (m.nick.clone(), m.body.clone()));
      }
    }
    out.push(ev);
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_reply_finds_its_parent_in_the_history() {
    let raw = [
      "@id=p1;display-name=MarwN1 :marwn1!marwn1@marwn1.tmi.twitch.tv PRIVMSG #xqc :what set is that",
      "@id=r1;display-name=oredokzx;reply-parent-msg-id=p1 :oredokzx!o@o.tmi.twitch.tv PRIVMSG #xqc :@MarwN1 30th",
      "@id=r2;display-name=x;reply-parent-msg-id=gone :x!x@x.tmi.twitch.tv PRIVMSG #xqc :@y hi",
    ]
    .map(String::from);
    let ev = events(&raw, "xqc");
    let reply = |i: usize| match &ev[i] {
      Chat::Msg(m) => (m.reply_to.as_deref(), m.reply_text.as_deref()),
      _ => panic!(),
    };
    assert_eq!(reply(1), (Some("MarwN1"), Some("what set is that")));
    assert_eq!(reply(2), (None, None), "a parent older than the history: plain line");
  }
}
