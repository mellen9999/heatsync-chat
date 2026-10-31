// heatsync feed posts (ext social.js buildFeedMessageDiv): `>>id [OP] [T] name  heat° ▲ ↩n: text`.
// pure: parsing the server's json, the heat ladder, the row a post becomes. the server escapes
// post text on store, so it is unescaped here and drawn as plain text
use serde_json::Value;

use super::color;
use super::message::{Extra, HsEmote, Kind, Message, Platform, Seg};
use crate::heatsync::{cw_blocked, valid_name};
use crate::images::url::allowed;
use crate::ui::theme;
use crate::util::time::{now_ms, parse_iso};

const MAX_TEXT: usize = 4000;
const MAX_NAME: usize = 100;
const MAX_REFS: usize = 50;

#[derive(Clone, Debug)]
pub struct Post {
  // base36, zero padded (the server's id)
  pub id: Box<str>,
  pub user: Box<str>,
  pub color: u32,
  pub anon: bool,
  pub platform: Option<Platform>,
  pub text: Box<str>,
  pub ts: i64,
  pub heat: u32,
  pub replies: u32,
  pub reply_to: Option<Box<str>>,
  pub is_op: bool,
  pub voted: bool,
  pub edited: bool,
  // the AUTHOR's own heatsync emotes, server-resolved: the only heatsync emote source for a post
  pub refs: Box<[HsEmote]>,
}

fn clamp(s: &str, max: usize) -> String {
  s.chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).take(max).collect()
}

// &amp; &lt; &gt; &quot; &#39; &#x27;, in one pass so "&amp;lt;" stays "&lt;"
pub fn unescape(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut rest = s;
  while let Some(i) = rest.find('&') {
    out.push_str(&rest[..i]);
    rest = &rest[i..];
    let hit = [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>'), ("&quot;", '"'), ("&#39;", '\''), ("&#x27;", '\'')]
      .into_iter()
      .find(|(e, _)| rest.starts_with(e));
    match hit {
      Some((e, c)) => {
        out.push(c);
        rest = &rest[e.len()..];
      }
      None => {
        out.push('&');
        rest = &rest[1..];
      }
    }
  }
  out.push_str(rest);
  out
}

fn id_ok(s: &str) -> bool {
  (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

// { name: url } or { name: { url, cw, nsfw, cw_cats } }; a gated or non-https ref draws as its name
fn refs_of(v: Option<&Value>) -> Box<[HsEmote]> {
  let Some(map) = v.and_then(Value::as_object) else { return Box::new([]) };
  map
    .iter()
    .filter_map(|(word, e)| {
      let url = e.as_str().or_else(|| e.get("url").and_then(Value::as_str))?;
      let gated =
        e.is_object() && (cw_blocked(e) || e.get("cw").is_some_and(|c| c.as_str().is_some_and(|s| !s.is_empty())));
      let zero = e.get("zeroWidth").and_then(Value::as_bool) == Some(true);
      (valid_name(word) && !gated && allowed(url)).then(|| (word.as_str().into(), url.into(), zero))
    })
    .take(MAX_REFS)
    .collect()
}

// one post from /api/messages, /api/thread or a `new-message` frame
pub fn parse(v: &Value) -> Option<Post> {
  let id = v.get("base36_id").and_then(Value::as_str).filter(|s| id_ok(s))?;
  let text = unescape(v.get("content").and_then(Value::as_str)?);
  let name = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
  let user = name("username").or_else(|| name("display_name")).unwrap_or("anon");
  let anon = user == "Anonymous" || v.get("user_id").is_none_or(Value::is_null);
  let reply_to = name("reply_to").filter(|s| id_ok(s)).map(Into::into);
  let platform = match name("platform") {
    Some("twitch") => Some(Platform::Twitch),
    Some("kick") => Some(Platform::Kick),
    Some("youtube") => Some(Platform::Youtube),
    _ => None,
  };
  let num =
    |k: &str| v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0).map_or(0, |n| n.min(1e9) as u32);
  Some(Post {
    id: id.into(),
    user: clamp(user, MAX_NAME).into(),
    color: name("user_color").and_then(color::parse_hex).map_or(theme::WHITE, color::readable),
    anon,
    platform,
    text: clamp(&text, MAX_TEXT).trim().into(),
    ts: name("created_at").and_then(parse_iso).unwrap_or_else(now_ms),
    heat: num("heat"),
    replies: num("reply_count"),
    is_op: v.get("is_op").and_then(Value::as_bool).unwrap_or(reply_to.is_none()),
    reply_to,
    voted: v.get("user_vote").and_then(Value::as_i64) == Some(1),
    edited: v.get("edited_at").is_some_and(|e| !e.is_null()),
    refs: refs_of(v.get("emote_refs")),
  })
}

// the site's heat ladder (ext heatLadder): white, white bold, yellow, yellow bold, red bold
pub fn tier(heat: u32) -> (u32, bool) {
  match heat {
    1000.. => (theme::RED, true),
    250.. => (theme::YELLOW, true),
    50.. => (theme::YELLOW, false),
    10.. => (theme::WHITE, true),
    _ => (theme::WHITE, false),
  }
}

// 1234 -> 1.2k, 2000 -> 2k
pub fn heat_text(heat: u32) -> String {
  if heat < 1000 {
    return heat.to_string();
  }
  let k = format!("{:.1}", heat as f64 / 1000.0);
  format!("{}k", k.strip_suffix(".0").unwrap_or(&k))
}

// the number as the ext shows it: a degree sign from 10 up, never an emoji
fn heat_seg(heat: u32) -> Seg {
  let (ink, bold) = tier(heat);
  let deg = if heat >= 10 { "°" } else { "" };
  Seg { text: format!(" {}{deg}", heat_text(heat)).into(), ink, bold }
}

pub fn row_id(id: &str) -> Box<str> {
  id.into()
}

// short id the way the site writes it: leading zeros dropped
pub fn short(id: &str) -> &str {
  let s = id.trim_start_matches('0');
  if s.is_empty() { "0" } else { s }
}

// `me` = your heatsync name, `signed_in` = a token exists (the ▲ shows only then, and never on your own post)
pub fn message(p: &Post, me: Option<&str>, signed_in: bool) -> Message {
  let own = me.is_some_and(|m| m.eq_ignore_ascii_case(&p.user));
  let tag = if p.is_op { ("[OP] ", theme::GREEN) } else { ("[RE] ", theme::BLUE) };
  let mut head = vec![
    Seg { text: format!(">>{} ", short(&p.id)).into(), ink: theme::CYAN, bold: false },
    Seg { text: tag.0.into(), ink: tag.1, bold: true },
  ];
  if let Some(pl) = p.platform {
    let (t, ink) = match pl {
      Platform::Twitch => ("[T] ", theme::TWITCH),
      Platform::Kick => ("[K] ", theme::GREEN),
      Platform::Youtube => ("[Y] ", theme::RED),
    };
    head.push(Seg { text: t.into(), ink, bold: true });
  }
  let mut tail = Vec::new();
  if p.heat > 0 {
    tail.push(heat_seg(p.heat));
  }
  if signed_in && !own {
    tail.push(Seg { text: " ▲".into(), ink: if p.voted { theme::HEATSYNC } else { theme::DIM }, bold: p.voted });
  }
  if p.replies > 0 {
    tail.push(Seg { text: format!(" ↩{}", p.replies).into(), ink: theme::DIM, bold: false });
  }
  let mut m = Message::system(p.ts, Platform::Twitch, "");
  m.kind = Kind::Chat;
  m.id = row_id(&p.id);
  m.login = p.user.to_lowercase().into();
  m.nick = if p.anon { "Anonymous".into() } else { p.user.clone() };
  m.color = if p.anon { theme::WHITE } else { p.color };
  m.body = if p.edited { format!("{} (edited)", p.text).into() } else { p.text.clone() };
  m.hs = p.refs.clone();
  m.x = Some(Box::new(Extra {
    chan: "".into(),
    head: head.into(),
    tail: tail.into(),
    edge: if p.heat > 0 { tier(p.heat).0 } else { 0 },
  }));
  m
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn sample() -> Value {
    json!({
      "base36_id": "00abc1", "username": "bob", "user_id": "7", "user_color": "#ff0000",
      "content": "a &gt; b &amp;&amp; &lt;c&gt; &#x27;q&#x27; &amp;lt;",
      "created_at": "2026-01-02T03:04:05Z", "heat": 1500, "reply_count": 3, "user_vote": 1,
      "emote_refs": { "catJAM": "https://cdn.7tv.app/a.webp", "nope": { "url": "https://cdn.7tv.app/b.webp", "nsfw": true },
                      "bad": "javascript:x", "obj": { "url": "https://cdn.7tv.app/c.webp" } },
    })
  }

  #[test]
  fn parses_and_unescapes() {
    let p = parse(&sample()).unwrap();
    assert_eq!(&*p.id, "00abc1");
    assert_eq!(&*p.text, "a > b && <c> 'q' &lt;");
    assert!(p.is_op && p.voted && !p.anon);
    assert_eq!((p.heat, p.replies), (1500, 3));
    let names: Vec<_> = p.refs.iter().map(|r| &*r.0).collect();
    assert!(names.contains(&"catJAM") && names.contains(&"obj"));
    assert!(!names.contains(&"nope") && !names.contains(&"bad"), "gated or non-https refs never become emotes");
  }

  #[test]
  fn rejects_bad_ids_and_missing_text() {
    assert!(parse(&json!({"base36_id": "../x", "content": "hi"})).is_none());
    assert!(parse(&json!({"base36_id": "abc"})).is_none());
  }

  #[test]
  fn anonymous_and_reply_tags() {
    let p = parse(&json!({"base36_id": "1", "content": "x", "username": "Anonymous", "reply_to": "00abc1"})).unwrap();
    assert!(p.anon && !p.is_op);
    let m = message(&p, None, false);
    assert_eq!(&*m.nick, "Anonymous");
    let x = m.x.unwrap();
    assert_eq!(&*x.head[0].text, ">>1 ");
    assert_eq!(&*x.head[1].text, "[RE] ");
  }

  #[test]
  fn heat_ladder_and_text() {
    assert_eq!(tier(0), (theme::WHITE, false));
    assert_eq!(tier(10), (theme::WHITE, true));
    assert_eq!(tier(50), (theme::YELLOW, false));
    assert_eq!(tier(250), (theme::YELLOW, true));
    assert_eq!(tier(5000), (theme::RED, true));
    assert_eq!(heat_text(999), "999");
    assert_eq!(heat_text(1000), "1k");
    assert_eq!(heat_text(1234), "1.2k");
  }

  #[test]
  fn row_shows_heat_vote_and_replies_without_fire() {
    let p = parse(&sample()).unwrap();
    let m = message(&p, Some("alice"), true);
    let x = m.x.as_ref().unwrap();
    let tail: String = x.tail.iter().map(|s| &*s.text).collect();
    assert_eq!(tail, " 1.5k° ▲ ↩3");
    assert!(!tail.contains('🔥'));
    assert_eq!(x.edge, theme::RED);
    // your own post has no ▲; logged out has none either
    let own = message(&p, Some("BOB"), true);
    assert!(!own.x.unwrap().tail.iter().any(|s| s.text.contains('▲')));
    assert!(!message(&p, None, false).x.unwrap().tail.iter().any(|s| s.text.contains('▲')));
  }

  #[test]
  fn emote_refs_ride_the_post_not_the_viewer() {
    let m = message(&parse(&sample()).unwrap(), None, false);
    assert_eq!(m.hs.len(), 2);
    assert_eq!(&*m.user_id, "", "no twitch id, so no sender-inventory lookup can run for a feed author");
  }
}
