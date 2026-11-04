// recent twitch history on join, via heatsync's robotty proxy (raw irc lines)
use crate::model::ident;
use std::collections::HashMap;

use serde::Deserialize;

use tokio::time::sleep;

use super::http;
use super::irc::{self, Chat};
use super::retry::{self, Watch};
use super::{NetEvent, ORIGIN, Out};
use crate::model::message::{Message, Platform, REPLY_TEXT};
use crate::util::clamp;

pub const LIMIT: usize = 800;
// ~800 raw irc lines
const MAX_BODY: usize = 2 << 20;
const CATCH_TRIES: u32 = 3;

#[derive(Deserialize)]
struct Recent {
  #[serde(default)]
  messages: Vec<String>,
}

// one ask for a channel's last `limit` lines; Err is the technical reason (stderr, not the screen)
async fn get(http: &reqwest::Client, ch: &str, limit: usize) -> Result<Vec<Chat>, String> {
  // the name goes into the url: only a real channel name does
  if !ident::twitch_room(ch) {
    return Ok(Vec::new());
  }
  let url = format!(
    "{ORIGIN}/api/recent-messages/{ch}?limit={limit}&hide_moderation_messages=false&hide_moderated_messages=false&clearchatToNotice=true"
  );
  let r = http.get(url).send().await.map_err(|e| e.to_string())?;
  if !r.status().is_success() {
    return Err(format!("http {}", r.status().as_u16()));
  }
  let body = http::read(r, MAX_BODY).await?;
  let recent = serde_json::from_slice::<Recent>(&body).map_err(|e| e.to_string())?;
  let start = recent.messages.len().saturating_sub(LIMIT);
  Ok(events(&recent.messages[start..], ch))
}

// the history a tab opens with, asked again until it comes (or the tab closes)
pub async fn fetch(http: reqwest::Client, ch: Box<str>, tab: Watch, out: Out) {
  let what = format!("{ch}'s recent chat");
  if let Some(evs) = retry::until(&out, &what, || tab.alive(), || get(&http, &ch, LIMIT)).await {
    out.send(NetEvent::Backfill(ch, evs));
  }
}

// twitch came back after a drop: what each channel said meanwhile, for the app to slot in by time. a
// few tries each, one channel at a time (a hundred tabs shouldn't be a hundred requests at once)
pub async fn catch_up(http: reqwest::Client, chans: Vec<Box<str>>, limit: usize, out: Out) {
  for ch in chans {
    for attempt in 1..=CATCH_TRIES {
      match get(&http, &ch, limit).await {
        Ok(evs) => {
          let msgs = settled(evs);
          if !msgs.is_empty() {
            out.send(NetEvent::CatchUp { platform: Platform::Twitch, key: ch, msgs });
          }
          break;
        }
        Err(e) => {
          eprintln!("catching up on {ch}: {e}");
          if attempt == CATCH_TRIES {
            out.status(format!("couldn't fetch what you missed in {ch}"));
          } else {
            sleep(super::backoff(attempt)).await;
          }
        }
      }
    }
  }
}

// only lines a catch-up can place safely: the ones with an id (the id is how a line we already have
// is told from one we missed). plain notices and mod events have none and are left out
fn settled(evs: Vec<Chat>) -> Vec<Message> {
  evs
    .into_iter()
    .filter_map(|e| match e {
      Chat::Msg(m) if !m.id.is_empty() => Some(m),
      _ => None,
    })
    .collect()
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
