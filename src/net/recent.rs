// recent twitch history on join, via heatsync's robotty proxy (raw irc lines)
use crate::i18n::f;
use crate::model::ident;
use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use tokio::time::sleep;

use super::http;
use super::irc::{self, Chat};
use super::retry::{self, Watch};
use super::{NetEvent, ORIGIN, Out};
use crate::emotes::live::valid_id;
use crate::heatsync::valid_name;
use crate::images::url::allowed;
use crate::model::message::{HsEmote, Message, Platform, REPLY_TEXT};
use crate::util::clamp;

pub const LIMIT: usize = 800;
// ~800 raw irc lines
const MAX_BODY: usize = 2 << 20;
const CATCH_TRIES: u32 = 3;

#[derive(Deserialize)]
struct Recent {
  #[serde(default)]
  messages: Vec<String>,
  // line id -> { word: ref }: the emotes each sender's app shipped with its line, as the server recorded
  // them. read loosely (a shape we do not know is no refs, never no history)
  #[serde(default)]
  emote_refs: Value,
}

// picks kept per line: past it the words read as any hand-typed word
const MAX_PICKS: usize = 50;

// one recorded ref as this app draws it: `{ p, id, z?, a? }`, an `hs` one with the `url` the server
// resolved, or `{ gone: true }` for one taken down since (an empty url: the reader makes the word text,
// never another emote of the name). 7tv, bttv and ffz are named as their own answers name them (the 1x file)
fn pick(word: &str, r: &Value) -> Option<HsEmote> {
  if !valid_name(word) {
    return None;
  }
  if r.get("gone").and_then(Value::as_bool) == Some(true) {
    return Some((word.into(), "".into(), false));
  }
  let s = |k: &str| r.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
  let id = s("id")?;
  let moving = r.get("a").and_then(Value::as_bool) == Some(true);
  let url = match s("p")? {
    "hs" => {
      let u = s("url")?;
      if u.starts_with('/') && !u.starts_with("//") { format!("{ORIGIN}{u}") } else { u.to_owned() }
    }
    "7tv" if valid_id(id) => format!("https://cdn.7tv.app/emote/{id}/1x.webp"),
    "bttv" if valid_id(id) => format!("https://cdn.betterttv.net/emote/{id}/1x.webp"),
    "ffz" if id.len() <= 12 && id.bytes().all(|b| b.is_ascii_digit()) => {
      format!("https://cdn.frankerfacez.com/emote/{id}/{}1", if moving { "animated/" } else { "" })
    }
    _ => return None,
  };
  allowed(&url).then(|| (word.into(), url.into(), r.get("z").and_then(Value::as_bool) == Some(true)))
}

fn picks(refs: &Value) -> Box<[HsEmote]> {
  let words = refs.as_object().into_iter().flatten();
  words.filter_map(|(w, r)| pick(w, r)).take(MAX_PICKS).collect()
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
  Ok(events(&recent.messages[start..], ch, &recent.emote_refs))
}

// the history a tab opens with, asked again until it comes (or the tab closes)
pub async fn fetch(http: reqwest::Client, ch: Box<str>, tab: Watch, out: Out) {
  let what = f("net2.recent.what", &[("channel", &ch)]);
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
            out.status(f("net2.recent.missed", &[("channel", &ch)]));
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
fn events(raw: &[String], ch: &str, refs: &Value) -> Vec<Chat> {
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
        // what the sender's app shipped with this line: a scrolled-back line draws what it drew live
        let shipped = refs.get(&*m.id).map(picks).unwrap_or_default();
        if !shipped.is_empty() {
          m.extra().picks = shipped;
        }
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
    let ev = events(&raw, "xqc", &Value::Null);
    let reply = |i: usize| match &ev[i] {
      Chat::Msg(m) => (m.reply_to.as_deref(), m.reply_text.as_deref()),
      _ => panic!(),
    };
    assert_eq!(reply(1), (Some("MarwN1"), Some("what set is that")));
    assert_eq!(reply(2), (None, None), "a parent older than the history: plain line");
  }

  // the server's record of what each sender's app shipped rides beside the lines: a scrolled-back line
  // reads the same picks its live copy did, and a pick taken down since is marked (the reader: text)
  #[test]
  fn a_history_line_keeps_the_emotes_its_sender_shipped() {
    let line = |id: &str, text: &str| format!("@id={id};display-name=A :a!a@a.tmi.twitch.tv PRIVMSG #xqc :{text}");
    let raw = [line("m1", "KEKW Mine Old"), line("m2", "KEKW"), line("m3", "Wide Moving Over0"), line("m4", "x")];
    let refs = serde_json::json!({
      "m1": {
        "KEKW": { "p": "7tv", "id": "01ABC" },
        "Mine": { "p": "hs", "id": "h1", "url": "/uploads/emotes/mine.webp", "z": true },
        "Old": { "p": "hs", "id": "h2", "gone": true },
      },
      "m3": {
        "Wide": { "p": "bttv", "id": "5f1b0186cf6d2144653d2970" },
        "Moving": { "p": "ffz", "id": "128054", "a": true },
        "Over0": { "p": "ffz", "id": "9" },
      },
      "m4": {
        "bad name": { "p": "7tv", "id": "01ABC" },
        "Path": { "p": "7tv", "id": "../x" },
        "Ffz": { "p": "ffz", "id": "12a" },
        "Evil": { "p": "hs", "id": "h3", "url": "https://evil.example/x.webp" },
        "NoUrl": { "p": "hs", "id": "h4" },
        "Other": { "p": "twitch", "id": "25" },
        "Junk": 7,
      },
      "nobody": { "KEKW": { "p": "7tv", "id": "01ABD" } },
    });
    let ev = events(&raw, "xqc", &refs);
    let of = |i: usize| match &ev[i] {
      Chat::Msg(m) => {
        let mut p: Vec<(String, String, bool)> =
          m.x.iter().flat_map(|x| x.picks.iter()).map(|(w, u, z)| (w.to_string(), u.to_string(), *z)).collect();
        p.sort();
        p
      }
      _ => panic!(),
    };
    let row = |w: &str, u: &str, z: bool| (w.to_string(), u.to_string(), z);
    assert_eq!(
      of(0),
      [
        row("KEKW", "https://cdn.7tv.app/emote/01ABC/1x.webp", false),
        row("Mine", "https://heatsync.org/uploads/emotes/mine.webp", true),
        row("Old", "", false),
      ],
      "the pick, the sender's own by the address the server resolved, and a taken-down one as a marker"
    );
    assert!(of(1).is_empty(), "a line with no record reads as any line does");
    assert_eq!(
      of(2),
      [
        row("Moving", "https://cdn.frankerfacez.com/emote/128054/animated/1", false),
        row("Over0", "https://cdn.frankerfacez.com/emote/9/1", false),
        row("Wide", "https://cdn.betterttv.net/emote/5f1b0186cf6d2144653d2970/1x.webp", false),
      ]
    );
    assert!(of(3).is_empty(), "nothing malformed, off the allowlist or of an unknown provider is kept: {:?}", of(3));
    for junk in [Value::Null, serde_json::json!([1]), serde_json::json!("x"), serde_json::json!({ "m1": [1] })] {
      let ev = events(&raw, "xqc", &junk);
      assert_eq!(ev.len(), 4, "a record we cannot read costs no history");
      assert!(matches!(&ev[0], Chat::Msg(m) if m.x.as_ref().is_none_or(|x| x.picks.is_empty())));
    }
  }
}
