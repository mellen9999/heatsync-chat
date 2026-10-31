// fetch + parse 7tv / bttv / ffz sets. every response is hostile: bounded, urls allowlisted
use serde::Deserialize;
use serde_json::Value;

use super::{Emote, MAX_SET, Provider, Set, valid_name};
use crate::images::url::allowed;

// bttv's overlay globals carry no zero-width flag in the api; every client hardcodes them
const BTTV_ZERO_WIDTH: [&str; 8] =
  ["SoSnowy", "IceCold", "SantaHat", "TopHat", "ReinDeer", "CandyCane", "cvMask", "cvHazmat"];

fn put(set: &mut Set, name: &str, e: Emote) {
  if set.len() < MAX_SET && valid_name(name) && allowed(&e.url) {
    set.entry(name.into()).or_insert(e);
  }
}

pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<Value, String> {
  let r = http.get(url).send().await.map_err(|e| e.to_string())?;
  if r.status() == reqwest::StatusCode::NOT_FOUND {
    // channel simply doesn't use this provider
    return Ok(Value::Null);
  }
  if !r.status().is_success() {
    return Err(format!("http {}", r.status().as_u16()));
  }
  let b = r.bytes().await.map_err(|e| e.to_string())?;
  serde_json::from_slice(&b).map_err(|e| e.to_string())
}

// ---- 7tv: v3 emote set { emotes: [{ name, flags, data: { host: { url, files } } }] }

#[derive(Deserialize)]
struct StvEmote {
  name: String,
  #[serde(default)]
  flags: u32,
  data: Option<StvData>,
}

#[derive(Deserialize)]
struct StvData {
  host: StvHost,
  #[serde(default)]
  flags: u32,
}

#[derive(Deserialize)]
struct StvHost {
  url: String,
  #[serde(default)]
  files: Vec<StvFile>,
}

#[derive(Deserialize)]
struct StvFile {
  name: String,
  #[serde(default)]
  width: u16,
  #[serde(default)]
  height: u16,
}

pub fn seventv(set: &Value) -> Set {
  let mut out = Set::new();
  let Some(list) = set.get("emotes").and_then(Value::as_array) else {
    return out;
  };
  for v in list.iter().take(MAX_SET) {
    let Ok(e) = StvEmote::deserialize(v) else { continue };
    let Some(d) = e.data else { continue };
    // 1x.webp: always present, decodes everywhere (avif doesn't)
    let file = d.host.files.iter().find(|f| f.name == "1x.webp");
    let (w, h) = file.map_or((0, 0), |f| (f.width, f.height));
    let base = d.host.url.strip_prefix("//").unwrap_or(&d.host.url);
    // zero-width: active-emote flag bit 0 or the emote's own flag bit 8 (ext checks flags & 257)
    let zero_width = e.flags & 1 != 0 || d.flags & 256 != 0;
    put(&mut out, &e.name, Emote { url: format!("https://{base}/1x.webp").into(), w, h, zero_width });
  }
  out
}

// the id of the channel's active 7tv set (inline, or named when 7tv left the inline copy null)
pub fn seventv_set_id(user: &Value) -> Option<Box<str>> {
  let id = user.get("emote_set").and_then(|s| s.get("id")).or_else(|| user.get("emote_set_id"))?.as_str()?;
  super::live::valid_id(id).then(|| id.into())
}

// the broadcaster's 7tv user id
pub fn seventv_user_id(user: &Value) -> Option<Box<str>> {
  let id = user.get("user")?.get("id")?.as_str()?;
  super::live::valid_id(id).then(|| id.into())
}

pub fn seventv_user(user: &Value) -> Set {
  user.get("emote_set").map(seventv).unwrap_or_default()
}

// ---- bttv: [{ id, code }] (global) / { channelEmotes, sharedEmotes } (channel)

fn bttv_list(list: Option<&Value>, out: &mut Set) {
  let Some(list) = list.and_then(Value::as_array) else {
    return;
  };
  for v in list {
    let (Some(id), Some(code)) = (v.get("id").and_then(Value::as_str), v.get("code").and_then(Value::as_str)) else {
      continue;
    };
    if !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
      continue;
    }
    let zero_width = BTTV_ZERO_WIDTH.contains(&code);
    let e = Emote { url: format!("https://cdn.betterttv.net/emote/{id}/1x.webp").into(), w: 0, h: 0, zero_width };
    put(out, code, e);
  }
}

pub fn bttv_global(v: &Value) -> Set {
  let mut out = Set::new();
  bttv_list(Some(v), &mut out);
  out
}

pub fn bttv_channel(v: &Value) -> Set {
  let mut out = Set::new();
  bttv_list(v.get("channelEmotes"), &mut out);
  bttv_list(v.get("sharedEmotes"), &mut out);
  out
}

// ---- ffz: { sets: { "<id>": { emoticons: [{ name, width, height, urls, animated, modifier }] } } }

pub fn ffz(v: &Value, only: Option<&[u64]>) -> Set {
  let mut out = Set::new();
  let Some(sets) = v.get("sets").and_then(Value::as_object) else {
    return out;
  };
  for (id, set) in sets {
    if let Some(only) = only
      && !id.parse().is_ok_and(|n: u64| only.contains(&n))
    {
      continue;
    }
    let Some(list) = set.get("emoticons").and_then(Value::as_array) else {
      continue;
    };
    for e in list {
      // modifier emotes (ffzX…) are tokens, not images
      if e.get("modifier").and_then(Value::as_bool) == Some(true) {
        continue;
      }
      let Some(name) = e.get("name").and_then(Value::as_str) else {
        continue;
      };
      let url = e
        .get("animated")
        .and_then(|a| a.get("1"))
        .or_else(|| e.get("urls").and_then(|u| u.get("1")))
        .and_then(Value::as_str);
      let Some(url) = url else { continue };
      let url = if let Some(r) = url.strip_prefix("//") { format!("https://{r}") } else { url.to_owned() };
      let dim = |k| e.get(k).and_then(Value::as_u64).unwrap_or(0).min(u16::MAX as u64) as u16;
      put(&mut out, name, Emote { url: url.into(), w: dim("width"), h: dim("height"), zero_width: false });
    }
  }
  out
}

// heatsync's global catalog (`/api/emotes`): only its twitch rows, the globals + unlocks twitch
// itself serves no anonymous api for
pub fn twitch_global(v: &Value) -> Set {
  let mut out = Set::new();
  // the catalog answers `{"emotes": [...]}`; a bare array is accepted too
  let Some(rows) = v.get("emotes").unwrap_or(v).as_array() else { return out };
  for e in rows.iter().take(5000).filter(|e| e.get("provider").and_then(Value::as_str) == Some("twitch")) {
    let (Some(name), Some(url)) = (e.get("name").and_then(Value::as_str), e.get("url").and_then(Value::as_str)) else {
      continue;
    };
    // 1.0 is the 28px size; the catalog lists 3.0
    let url = url.strip_suffix("/3.0").map_or_else(|| url.to_owned(), |b| format!("{b}/1.0"));
    // the catalog lists the light-theme art; this window is dark
    let url = url.replace("/light/", "/dark/");
    put(&mut out, name, Emote { url: url.into(), w: 0, h: 0, zero_width: false });
  }
  out
}

pub fn ffz_global(v: &Value) -> Set {
  let defaults: Vec<u64> = v
    .get("default_sets")
    .and_then(Value::as_array)
    .map(|a| a.iter().filter_map(Value::as_u64).collect())
    .unwrap_or_default();
  ffz(v, Some(&defaults))
}

pub const HEATSYNC_GLOBAL: &str = "https://heatsync.org/api/emotes";
pub const SEVENTV_GLOBAL: &str = "https://7tv.io/v3/emote-sets/global";
pub const BTTV_GLOBAL: &str = "https://api.betterttv.net/3/cached/emotes/global";
pub const FFZ_GLOBAL: &str = "https://api.frankerfacez.com/v1/set/global";

pub fn channel_urls(room_id: &str) -> [(Provider, String); 3] {
  [
    (Provider::SevenTv, format!("https://7tv.io/v3/users/twitch/{room_id}")),
    (Provider::Bttv, format!("https://api.betterttv.net/3/cached/users/twitch/{room_id}")),
    (Provider::Ffz, format!("https://api.frankerfacez.com/v1/room/id/{room_id}")),
  ]
}

pub fn parse_channel(p: Provider, v: &Value) -> Set {
  match p {
    Provider::SevenTv => seventv_user(v),
    Provider::Bttv => bttv_channel(v),
    Provider::Ffz => ffz(v, None),
    // twitch channel emotes come positioned in the irc tag, never as a set
    Provider::Twitch => Set::new(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn heatsync_catalog_twitch_rows_only() {
    let v = json!([
      {"name": "Kappa", "provider": "twitch", "url": "https://static-cdn.jtvnw.net/emoticons/v2/25/static/dark/3.0"},
      {"name": "OMEGALUL", "provider": "7tv", "url": "https://cdn.7tv.app/emote/x/1x.webp"},
      {"name": "evil", "provider": "twitch", "url": "https://evil.example/x"},
    ]);
    let s = twitch_global(&v);
    assert_eq!(s.len(), 1);
    assert!(s["Kappa"].url.ends_with("/dark/1.0"));
    // the live catalog wraps the rows: `{"emotes": [...]}`
    let wrapped = json!({"emotes": [{"name": "Kappa", "provider": "twitch", "url": "https://static-cdn.jtvnw.net/emoticons/v2/25/static/light/1.0"}]});
    assert!(twitch_global(&wrapped)["Kappa"].url.ends_with("/dark/1.0"));
  }

  #[test]
  fn seventv_parses_and_flags_zero_width() {
    let v = json!({"emotes": [
      {"name": "catJAM", "flags": 0, "data": {"flags": 0, "host": {"url": "//cdn.7tv.app/emote/abc", "files": [{"name": "1x.webp", "width": 32, "height": 32}]}}},
      {"name": "RainTime", "flags": 1, "data": {"flags": 0, "host": {"url": "//cdn.7tv.app/emote/zw", "files": []}}},
      {"name": "evil", "data": {"host": {"url": "//evil.example/x", "files": []}}},
      {"name": "has space", "data": {"host": {"url": "//cdn.7tv.app/emote/y", "files": []}}},
    ]});
    let s = seventv(&v);
    assert_eq!(&*s["catJAM"].url, "https://cdn.7tv.app/emote/abc/1x.webp");
    assert_eq!((s["catJAM"].w, s["catJAM"].h), (32, 32));
    assert!(s["RainTime"].zero_width);
    assert!(!s.contains_key("evil"));
    assert!(!s.contains_key("has space"));
  }

  #[test]
  fn bttv_channel_and_zero_width() {
    let v = json!({"channelEmotes": [{"id": "5f1b0186cf6d2144653d2970", "code": "catJAM"}], "sharedEmotes": [{"id": "567b5b520e984428652809b6", "code": "SoSnowy"}, {"id": "../x", "code": "bad"}]});
    let s = bttv_channel(&v);
    assert!(s.contains_key("catJAM"));
    assert!(s["SoSnowy"].zero_width);
    assert!(!s.contains_key("bad"));
  }

  #[test]
  fn ffz_skips_modifiers_and_filters_global_sets() {
    let v = json!({"default_sets": [3], "sets": {
      "3": {"emoticons": [{"name": "LilZ", "width": 28, "height": 28, "urls": {"1": "https://cdn.frankerfacez.com/emote/1/1"}}, {"name": "ffzX", "modifier": true, "urls": {"1": "https://cdn.frankerfacez.com/emote/2/1"}}]},
      "4": {"emoticons": [{"name": "Hidden", "urls": {"1": "https://cdn.frankerfacez.com/emote/3/1"}}]}
    }});
    let s = ffz_global(&v);
    assert!(s.contains_key("LilZ"));
    assert!(!s.contains_key("ffzX"));
    assert!(!s.contains_key("Hidden"));
  }

  #[test]
  fn junk_is_empty_not_panic() {
    for v in [json!(null), json!([]), json!({"emotes": 5}), json!({"sets": {"1": {"emoticons": [1, null]}}})] {
      assert!(seventv(&v).is_empty());
      assert!(bttv_channel(&v).is_empty());
      assert!(ffz(&v, None).is_empty());
    }
  }
}
