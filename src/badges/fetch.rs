// badge sources. twitch via the public web gql (no auth), ffz + chatterino via their id lists
use serde_json::{Value, json};

use super::{Badge, TwitchSet, UserBadges, key};
use crate::images::url::allowed;
use crate::model::color::parse_hex;

// twitch's own public web client id; anonymous reads only
const GQL_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const GQL: &str = "https://gql.twitch.tv/gql";
const MAX_USERS: usize = 100_000;
const MAX_BADGES: usize = 2000;

fn twitch_set(list: Option<&Value>) -> TwitchSet {
  let mut out = TwitchSet::new();
  for b in list.and_then(Value::as_array).into_iter().flatten().take(MAX_BADGES) {
    let (Some(url), Some(set), Some(ver)) = (
      b.get("imageURL").and_then(Value::as_str),
      b.get("setID").and_then(Value::as_str),
      b.get("version").and_then(Value::as_str),
    ) else {
      continue;
    };
    if !allowed(url) {
      continue;
    }
    let title = b.get("title").and_then(Value::as_str).unwrap_or(set);
    out.insert(key(set, ver), Badge { url: url.into(), title: title.into(), bg: None });
  }
  out
}

async fn gql(http: &reqwest::Client, query: &str) -> Result<Value, String> {
  let r = http
    .post(GQL)
    .header("Client-Id", GQL_CLIENT_ID)
    .body(json!({ "query": query }).to_string())
    .send()
    .await
    .map_err(|e| e.to_string())?;
  if !r.status().is_success() {
    return Err(format!("http {}", r.status().as_u16()));
  }
  let b = r.bytes().await.map_err(|e| e.to_string())?;
  let v: Value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
  Ok(v.get("data").cloned().unwrap_or(Value::Null))
}

pub async fn twitch_global(http: &reqwest::Client) -> Result<TwitchSet, String> {
  let d = gql(http, "{ badges { imageURL(size: NORMAL) setID version title } }").await?;
  Ok(twitch_set(d.get("badges")))
}

pub async fn twitch_channel(http: &reqwest::Client, login: &str) -> Result<TwitchSet, String> {
  // login is validated by the caller ([a-z0-9_]); never interpolate anything else
  let q =
    format!("{{ user(login: \"{login}\") {{ broadcastBadges {{ imageURL(size: NORMAL) setID version title }} }} }}");
  let d = gql(http, &q).await?;
  Ok(twitch_set(d.get("user").and_then(|u| u.get("broadcastBadges"))))
}

pub fn ffz(v: &Value) -> UserBadges {
  let mut out = UserBadges::default();
  let mut index = std::collections::HashMap::new();
  for b in v.get("badges").and_then(Value::as_array).into_iter().flatten().take(MAX_BADGES) {
    let (Some(id), Some(url)) =
      (b.get("id").and_then(Value::as_u64), b.get("urls").and_then(|u| u.get("1")).and_then(Value::as_str))
    else {
      continue;
    };
    if !allowed(url) {
      continue;
    }
    let title = b.get("title").and_then(Value::as_str).unwrap_or("ffz");
    let bg = b.get("color").and_then(Value::as_str).and_then(parse_hex);
    index.insert(id.to_string(), out.list.len() as u16);
    out.list.push(Badge { url: url.into(), title: title.into(), bg });
  }
  let mut n = 0;
  for (id, users) in v.get("users").and_then(Value::as_object).into_iter().flatten() {
    let Some(&i) = index.get(id) else { continue };
    for u in users.as_array().into_iter().flatten().filter_map(Value::as_u64) {
      if n >= MAX_USERS {
        return out;
      }
      out.users.entry(u).or_default().push(i);
      n += 1;
    }
  }
  out
}

pub fn chatterino(v: &Value) -> UserBadges {
  let mut out = UserBadges::default();
  let mut n = 0;
  for b in v.get("badges").and_then(Value::as_array).into_iter().flatten().take(200) {
    let Some(url) = b.get("image1").and_then(Value::as_str).filter(|u| allowed(u)) else { continue };
    let i = out.list.len() as u16;
    let title = b.get("tooltip").and_then(Value::as_str).unwrap_or("chatterino");
    out.list.push(Badge { url: url.into(), title: title.into(), bg: None });
    for u in b.get("users").and_then(Value::as_array).into_iter().flatten() {
      let id = u.as_u64().or_else(|| u.as_str().and_then(|s| s.parse().ok()));
      if let Some(id) = id
        && n < MAX_USERS
      {
        out.users.entry(id).or_default().push(i);
        n += 1;
      }
    }
  }
  out
}

pub const FFZ_IDS: &str = "https://api.frankerfacez.com/v1/badges/ids";
pub const CHATTERINO: &str = "https://heatsync.org/api/chatterino-badges";

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_ffz_and_chatterino() {
    let v = json!({"badges": [{"id": 3, "title": "FFZ Supporter", "color": "#755000", "urls": {"1": "https://cdn.frankerfacez.com/badge/3/1"}}, {"id": 9, "urls": {"1": "https://evil.example/x"}}],
      "users": {"3": [1, 2], "9": [3], "77": [4]}});
    let u = ffz(&v);
    assert_eq!(u.list.len(), 1);
    assert_eq!(u.list[0].bg, Some(0x755000));
    assert_eq!(u.users.get(&1), Some(&vec![0]));
    assert!(!u.users.contains_key(&3));
    let c = chatterino(
      &json!({"badges": [{"tooltip": "top", "image1": "https://fourtf.com/chatterino/badges/topd.png", "users": ["11", 12, "x"]}]}),
    );
    assert_eq!(c.users.len(), 2);
  }

  #[test]
  fn twitch_set_filters() {
    let s = twitch_set(Some(
      &json!([{"imageURL": "https://static-cdn.jtvnw.net/badges/v1/a/1", "setID": "moderator", "version": "1", "title": "Moderator"}, {"imageURL": "https://x.example/b", "setID": "s", "version": "1"}]),
    ));
    assert_eq!(s.len(), 1);
    assert_eq!(&*s[&key("moderator", "1")].title, "Moderator");
  }
}
