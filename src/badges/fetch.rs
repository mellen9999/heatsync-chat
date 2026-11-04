// badge sources. twitch via the public web gql (no auth), ffz + chatterino via their id lists
use serde_json::{Value, json};

use super::{Badge, TwitchSet, UserBadges, key};
use crate::images::url::allowed;
use crate::model::color::parse_hex;
use crate::net::http;

// twitch's own public web client id; anonymous reads only
pub(crate) const GQL_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
pub(crate) const GQL: &str = "https://gql.twitch.tv/gql";
// the twitch badge gql answer
const MAX_GQL: usize = 1 << 20;
const MAX_USERS: usize = 100_000;
const MAX_BADGES: usize = 2000;
const MAX_KICK: usize = 1 << 20;
const MAX_KICK_TIERS: usize = 100;
const KICK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

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
  let v = http::capped_json(Ok(r), MAX_GQL).await?;
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

// a channel's own ffz mod and vip art (`/v1/room/id/{id}`), standing in for twitch's moderator/1 and
// vip/1 there like the ext: 2x first, then 1x (mod also `moderator_badge`). glyphs on transparency, so
// each sits on its tile: mod green, vip black
pub fn ffz_room(v: &Value) -> TwitchSet {
  let mut out = TwitchSet::new();
  let Some(room) = v.get("room") else { return out };
  let pick = |k: &str| room.get(k).and_then(|u| u.get("2").or_else(|| u.get("1"))).and_then(Value::as_str);
  let rows = [
    (
      "moderator",
      "moderator",
      pick("mod_urls").or_else(|| room.get("moderator_badge").and_then(Value::as_str)),
      crate::ui::theme::MOD_BADGE,
    ),
    ("vip", "vip", pick("vip_badge"), crate::ui::theme::BLACK),
  ];
  for (set, title, url, bg) in rows {
    let Some(url) = url else { continue };
    let url = url.strip_prefix("//").map_or_else(|| url.to_owned(), |r| format!("https://{r}"));
    if url.len() <= 512 && allowed(&url) {
      out.insert(key(set, "1"), Badge { url: url.into(), title: title.into(), bg: Some(bg) });
    }
  }
  out
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

// a kick channel's subscriber badge art (`subscriber_badges[{months, badge_image:{src}}]` of the public
// channel json, what the ext's kick_channel_badges reads): months → picture, only pictures on the host
// allowlist. a channel with none gives an empty list
pub fn kick_subs(v: &Value) -> Vec<(u32, Box<str>)> {
  let mut out: Vec<(u32, Box<str>)> = Vec::new();
  for b in v.get("subscriber_badges").and_then(Value::as_array).into_iter().flatten().take(MAX_KICK_TIERS) {
    let months = b.get("months").and_then(|m| m.as_u64().or_else(|| m.as_str()?.trim().parse().ok()));
    let src = b.pointer("/badge_image/src").and_then(Value::as_str);
    if let (Some(m), Some(src)) = (months.filter(|m| (1..=1200).contains(m)), src)
      && allowed(src)
      && !out.iter().any(|o| u64::from(o.0) == m)
    {
      out.push((m as u32, src.into()));
    }
  }
  out
}

pub async fn kick_channel(http: &reqwest::Client, slug: &str) -> Result<Vec<(u32, Box<str>)>, String> {
  // slug is checked by the caller (`ident::relay_key`); it goes in a path, so nothing else gets here
  let r = http.get(format!("https://kick.com/api/v2/channels/{slug}")).timeout(KICK_TIMEOUT).send().await;
  Ok(kick_subs(&http::capped_json(r, MAX_KICK).await?))
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

  #[test]
  fn kick_tiers_are_months_to_allowlisted_art() {
    let v = json!({"subscriber_badges": [
      {"months": 1, "badge_image": {"src": "https://files.kick.com/channel_subscriber_badges/1/original"}},
      {"months": "3", "badge_image": {"src": "https://files.kick.com/channel_subscriber_badges/3/original"}},
      {"months": 1, "badge_image": {"src": "https://files.kick.com/dupe"}},
      {"months": 6, "badge_image": {"src": "https://evil.example/x"}},
      {"months": 0, "badge_image": {"src": "https://files.kick.com/zero"}},
      {"months": 9, "badge_image": null}, {"months": 12}, 7, null]});
    let t = kick_subs(&v);
    assert_eq!(t.iter().map(|a| a.0).collect::<Vec<_>>(), [1, 3]);
    assert!(t[0].1.ends_with("/1/original"), "first wins");
    for bad in [json!(null), json!({"subscriber_badges": 4}), json!({"subscriber_badges": {}})] {
      assert!(kick_subs(&bad).is_empty());
    }
    let many: Vec<Value> =
      (1..=500).map(|i| json!({"months": i, "badge_image": {"src": "https://files.kick.com/a"}})).collect();
    assert_eq!(kick_subs(&json!({"subscriber_badges": many})).len(), MAX_KICK_TIERS);
  }

  #[test]
  fn ffz_room_mod_and_vip() {
    let v = json!({"room": {
      "mod_urls": {"1": "https://cdn.frankerfacez.com/room-badge/mod/id/1/v/a/1", "2": "https://cdn.frankerfacez.com/room-badge/mod/id/1/v/a/2"},
      "vip_badge": {"1": "//cdn.frankerfacez.com/room-badge/vip/id/1/v/b/1"}
    }});
    let s = ffz_room(&v);
    assert!(s[&key("moderator", "1")].url.ends_with("/2"), "2x first");
    assert_eq!(s[&key("moderator", "1")].bg, Some(crate::ui::theme::MOD_BADGE));
    assert_eq!(&*s[&key("vip", "1")].url, "https://cdn.frankerfacez.com/room-badge/vip/id/1/v/b/1");
    let only_old =
      json!({"room": {"mod_urls": null, "moderator_badge": "https://cdn.frankerfacez.com/room-badge/mod/id/1/v/a/1"}});
    assert_eq!(ffz_room(&only_old).len(), 1);
    let hostile = json!({"room": {"mod_urls": {"2": "https://evil.example/x"}, "vip_badge": {"1": 7}}});
    assert!(ffz_room(&hostile).is_empty());
    assert!(ffz_room(&json!({"room": null})).is_empty() && ffz_room(&json!(3)).is_empty());
  }
}
