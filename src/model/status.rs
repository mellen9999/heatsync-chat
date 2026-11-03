// /status's stream half (ext buildChatStatusPanel): live or offline, uptime, viewers, title, game and
// language, from twitch's public web gql. every field is bounded; a junk answer reads as nothing
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
  pub name: Box<str>,
  // live: (viewers, started ms)
  pub live: Option<(u64, i64)>,
  pub title: Box<str>,
  pub game: Box<str>,
  pub lang: Box<str>,
}

pub fn query(login: &str) -> Value {
  serde_json::json!({
    "query": "query($login:String!){user(login:$login){displayName stream{type viewersCount createdAt game{name}} broadcastSettings{title language}}}",
    "variables": { "login": login },
  })
}

fn text(v: Option<&Value>, max: usize) -> Box<str> {
  let s = v.and_then(Value::as_str).unwrap_or("");
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().into()
}

// None = twitch knows no such login
pub fn parse(v: &Value) -> Option<Info> {
  let u = v.pointer("/data/user").filter(|u| u.is_object())?;
  let s = u.get("stream").filter(|s| s.get("type").and_then(Value::as_str) == Some("live"));
  let live = s.map(|s| {
    let n = s.get("viewersCount").and_then(Value::as_u64).unwrap_or(0);
    let at = s.get("createdAt").and_then(Value::as_str).and_then(crate::util::time::parse_iso).unwrap_or(0);
    (n, at)
  });
  let bs = u.get("broadcastSettings");
  Some(Info {
    name: text(u.get("displayName"), 40),
    live,
    title: text(bs.and_then(|b| b.get("title")), 200),
    game: text(s.and_then(|s| s.pointer("/game/name")), 80),
    lang: text(bs.and_then(|b| b.get("language")), 8),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn reads_twitchs_answer() {
    let v = json!({"data": {"user": {"displayName": "forsen", "stream": {"type": "live", "viewersCount": 12345,
      "createdAt": "2026-11-03T10:00:00Z", "game": {"name": "Just\nChatting"}},
      "broadcastSettings": {"title": "hi", "language": "en"}}}});
    let i = parse(&v).unwrap();
    assert_eq!((&*i.name, &*i.title, &*i.game, &*i.lang), ("forsen", "hi", "Just Chatting", "en"));
    assert_eq!(i.live.map(|l| l.0), Some(12345));
    let off = json!({"data": {"user": {"displayName": "forsen", "stream": null, "broadcastSettings": {"title": "x"}}}});
    assert_eq!(parse(&off).unwrap().live, None);
    assert!(parse(&json!({"data": {"user": null}})).is_none());
    assert!(parse(&json!("junk")).is_none());
  }
}
