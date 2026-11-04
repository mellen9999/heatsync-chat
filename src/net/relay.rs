// kick + youtube lines as the heatsync server relays them (live ws frames and /api/recent rows).
// every field is hostile: bounded, typed, urls allowlisted
use serde_json::Value;

use crate::heatsync::content::emote_hidden;
use crate::heatsync::valid_name;
use crate::images::url::allowed;
use crate::model::color;
use crate::model::message::{HsEmote, Kind, Message, Placed, Platform, REPLY_TEXT};
use crate::util::clamp;
use crate::util::time::{now_ms, parse_iso};

const MAX_TEXT: usize = 2000;
const MAX_NAME: usize = 100;
pub const MAX_BATCH: usize = 250;
const MAX_HS: usize = 50;
// emote pictures per line (any platform): past it the words stay as plain text
pub const MAX_EMOTES: usize = 50;

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
  v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn ts(v: &Value) -> i64 {
  match v.get("timestamp").or_else(|| v.get("ts")) {
    Some(Value::Number(n)) => n.as_i64().unwrap_or_else(now_ms),
    Some(Value::String(s)) => parse_iso(s).unwrap_or_else(now_ms),
    _ => now_ms(),
  }
}

// heatsync's per-sender emote refs, server-resolved from the sender's inventory
pub(crate) fn hs_emotes(v: Option<&Value>) -> Box<[HsEmote]> {
  let Some(map) = v.and_then(Value::as_object) else { return Box::new([]) };
  map
    .iter()
    .filter_map(|(word, e)| {
      let url = str_of(e, "url")?;
      (valid_name(word) && !emote_hidden(e) && allowed(url))
        .then(|| (word.as_str().into(), url.into(), e.get("zeroWidth").and_then(Value::as_bool) == Some(true)))
    })
    .take(MAX_HS)
    .collect()
}

// refs that arrived apart from their line, added by name: a name already on the line keeps its ref (the
// ext's merge: additive, never overwriting). true when any landed
pub fn merge_hs(into: &mut Box<[HsEmote]>, add: &[HsEmote]) -> bool {
  let fresh: Vec<&HsEmote> = add.iter().filter(|a| !into.iter().any(|h| h.0 == a.0)).collect();
  let room = MAX_HS.saturating_sub(into.len());
  if fresh.is_empty() || room == 0 {
    return false;
  }
  let mut v = std::mem::take(into).into_vec();
  v.extend(fresh.into_iter().take(room).cloned());
  *into = v.into();
  true
}

fn base(platform: Platform, id: &str, login: &str, nick: &str, color: Option<&str>, body: String, ts: i64) -> Message {
  let login: Box<str> = clamp(login, MAX_NAME).to_lowercase().into();
  let c = color.and_then(color::parse_hex).unwrap_or_else(|| color::default_for(&login));
  Message {
    id: clamp(id, 100).into(),
    ts,
    platform,
    kind: Kind::Chat,
    flags: 0,
    user_id: "".into(),
    nick: clamp(nick, MAX_NAME).into(),
    login,
    color: c,
    body: body.into(),
    emotes: Box::new([]),
    hs: Box::new([]),
    badges: Box::new([]),
    reply_to: None,
    reply_text: None,
    system: None,
    x: None,
  }
}

// kick text carries `[emote:<id>:<name>]` tokens: swap in the name, remember the image by range
fn kick_text(raw: &str) -> (String, Vec<Placed>) {
  let raw = clamp(raw, MAX_TEXT);
  let mut out = String::with_capacity(raw.len());
  let mut emotes = Vec::new();
  let mut rest = raw;
  while let Some(i) = rest.find("[emote:") {
    out.push_str(&rest[..i]);
    let tail = &rest[i + 7..];
    let parsed = tail.find(']').and_then(|end| {
      let (id, name) = tail[..end].split_once(':')?;
      (id.bytes().all(|b| b.is_ascii_digit()) && !id.is_empty() && !name.is_empty() && !name.contains(' '))
        .then_some((id, name, end))
    });
    match parsed {
      Some((id, name, end)) => {
        let a = out.len() as u32;
        out.push_str(name);
        if emotes.len() < MAX_EMOTES {
          emotes.push((a..out.len() as u32, format!("https://files.kick.com/emotes/{id}/fullsize").into()));
        }
        rest = &tail[end + 1..];
      }
      None => {
        out.push_str("[emote:");
        rest = tail;
      }
    }
  }
  out.push_str(rest);
  (out, emotes)
}

// live `kick-chat-message.data` / backfill rows / `/api/recent/kick` rows
pub fn kick(v: &Value) -> Option<Message> {
  let login = str_of(v, "username").or_else(|| str_of(v, "sender"))?;
  let nick = str_of(v, "displayName").or_else(|| str_of(v, "display_name")).unwrap_or(login);
  let raw = str_of(v, "content").or_else(|| str_of(v, "text")).unwrap_or("");
  let (body, emotes) = kick_text(raw);
  let mut m = base(Platform::Kick, str_of(v, "id").unwrap_or(""), login, nick, str_of(v, "color"), body, ts(v));
  m.emotes = emotes.into();
  // the sender's kick id (frames from before it was sent carry none: plain name)
  m.user_id =
    v.get("senderId").and_then(Value::as_u64).filter(|n| *n > 0).map(|n| n.to_string().into()).unwrap_or_default();
  m.hs = hs_emotes(v.get("hsEmotes"));
  m.reply_to = v.get("replyTo").and_then(|r| str_of(r, "username")).map(|u| clamp(u, MAX_NAME).into());
  m.reply_text = v.get("replyTo").and_then(|r| str_of(r, "content")).map(|t| clamp(t, REPLY_TEXT).into());
  Some(m)
}

// youtube emoji urls come at odd sizes; ask google's resizer for 48px
fn yt_url(url: &str) -> Option<Box<str>> {
  if !allowed(url) {
    return None;
  }
  let Some(i) = url.find("=w") else { return Some(url.into()) };
  let tail = &url[i + 2..];
  let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
  let w = digits(tail);
  let ok = w > 0 && tail[w..].starts_with("-h") && digits(&tail[w + 2..]) > 0;
  if !ok {
    return Some(url.into());
  }
  let after = &tail[w + 2 + digits(&tail[w + 2..])..];
  Some(format!("{}=w48-h48{after}", &url[..i]).into())
}

// live `youtube:chat.messages[]` / `/api/recent/youtube` rows
pub fn youtube(v: &Value) -> Option<Message> {
  let nick = str_of(v, "user").or_else(|| str_of(v, "display_name")).or_else(|| str_of(v, "sender"))?;
  let mut body = clamp(str_of(v, "text").or_else(|| str_of(v, "message")).unwrap_or(""), MAX_TEXT).to_owned();
  if let Some(a) = str_of(v, "amount") {
    body = format!("[{}] {body}", clamp(a, 32));
  }
  let mut m = base(Platform::Youtube, str_of(v, "id").unwrap_or(""), nick, nick, str_of(v, "color"), body, ts(v));
  m.user_id = str_of(v, "authorChannelId").map(|c| clamp(c, 64).into()).unwrap_or_default();
  m.hs = hs_emotes(v.get("hsEmotes"));
  let mut emotes = Vec::new();
  for e in v.get("emotes").and_then(Value::as_array).into_iter().flatten().take(50) {
    let (Some(alt), Some(url)) = (str_of(e, "alt"), str_of(e, "url").and_then(yt_url)) else { continue };
    let mut from = 0;
    while emotes.len() < MAX_EMOTES
      && let Some(i) = m.body[from..].find(alt)
    {
      let a = from + i;
      emotes.push((a as u32..(a + alt.len()) as u32, url.clone()));
      from = a + alt.len();
    }
  }
  emotes.sort_by_key(|(r, _)| r.start);
  m.emotes = emotes.into();
  if m.body.is_empty() {
    let sys = str_of(v, "systemMsg")?;
    m.kind = Kind::System;
    m.body = clamp(sys, MAX_TEXT).into();
  } else if let Some(sys) = str_of(v, "systemMsg") {
    m.system = Some(clamp(sys, MAX_TEXT).into());
  }
  Some(m)
}

// bounded list → messages, keeping the newest when oversized
pub fn batch(list: Option<&Value>, parse: fn(&Value) -> Option<Message>) -> Vec<Message> {
  let Some(a) = list.and_then(Value::as_array) else { return Vec::new() };
  a[a.len().saturating_sub(MAX_BATCH)..].iter().filter_map(parse).collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn kick_tokens_and_hs() {
    let v = json!({"id": "x", "username": "Sov", "displayName": "SOV", "content": "hi [emote:37225:KEKLEO] [emote:bad] gg",
      "color": "#FF2C56", "timestamp": 5,
      "hsEmotes": {"catJAM": {"url": "https://cdn.heatsync.org/uploads/a.webp"}, "nope": {"url": "https://cdn.heatsync.org/b.webp", "nsfw": true},
        "gore": {"url": "https://cdn.heatsync.org/c.webp", "cw_cats": ["gore"]}, "evil": {"url": "https://evil.example/x"}},
      "replyTo": {"username": "bob", "content": "first"}});
    let m = kick(&v).unwrap();
    assert_eq!(&*m.body, "hi KEKLEO [emote:bad] gg");
    let (r, url) = &m.emotes[0];
    assert_eq!(&m.body[r.start as usize..r.end as usize], "KEKLEO");
    assert!(url.ends_with("/emotes/37225/fullsize"));
    assert_eq!(m.hs.len(), 1);
    assert_eq!(&*m.hs[0].0, "catJAM");
    assert_eq!((m.reply_to.as_deref(), m.reply_text.as_deref()), (Some("bob"), Some("first")));
    assert_eq!((&*m.login, m.ts), ("sov", 5));
    assert!(m.user_id.is_empty(), "no sender id on the frame: plain name");
    let with = kick(&json!({"username": "sov", "senderId": 4242, "content": "x"})).unwrap();
    assert_eq!(&*with.user_id, "4242");
    for bad in [json!(0), json!(-3), json!("7"), json!(1.5)] {
      assert!(kick(&json!({"username": "sov", "senderId": bad, "content": "x"})).unwrap().user_id.is_empty());
    }
  }

  #[test]
  fn youtube_emoji_superchat_system() {
    let v = json!({"id": "y", "user": "Al", "text": "yo :smile: :smile:", "amount": "$5.00", "timestamp": 9,
      "emotes": [{"alt": ":smile:", "url": "https://yt3.ggpht.com/abc=w24-h24-c-k-nd"}]});
    let m = youtube(&v).unwrap();
    assert!(m.body.starts_with("[$5.00] "));
    assert_eq!(m.emotes.len(), 2);
    assert!(m.emotes[0].1.contains("=w48-h48-c-k-nd"));
    let s = youtube(&json!({"user": "Bo", "text": "", "systemMsg": "Bo became a member"})).unwrap();
    assert_eq!(s.kind, Kind::System);
    assert!(youtube(&json!({"user": "Bo", "text": ""})).is_none());
  }

  #[test]
  fn a_line_shows_at_most_fifty_emote_pictures() {
    let kick_body = "[emote:1:a] ".repeat(160);
    let m = kick(&json!({"username": "sov", "content": kick_body})).unwrap();
    assert_eq!(m.emotes.len(), MAX_EMOTES);
    assert_eq!(m.body.matches('a').count(), 160, "the words stay, only the pictures stop");
    let yt_body = ":s: ".repeat(400);
    let m = youtube(
      &json!({"user": "Al", "text": yt_body, "emotes": [{"alt": ":s:", "url": "https://yt3.ggpht.com/abc=w24-h24"}]}),
    )
    .unwrap();
    assert_eq!(m.emotes.len(), MAX_EMOTES);
    assert!(m.emotes.windows(2).all(|w| w[0].0.start < w[1].0.start), "the first fifty, in order");
  }

  #[test]
  fn hostile_rows() {
    for v in [json!(null), json!(5), json!({}), json!({"username": 5}), json!({"user": ["x"]})] {
      assert!(kick(&v).is_none());
      assert!(youtube(&v).is_none());
    }
    let long = "x".repeat(10_000);
    assert!(kick(&json!({"username": "a", "content": long})).unwrap().body.len() <= MAX_TEXT);
  }
}
