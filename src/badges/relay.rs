// the badges a kick or youtube frame carries, as (name, version) pairs like twitch's. the server sends
// kick `[{name, version:"1", count?}]` (count = months for subscriber), the raw kick shape is
// `{type, count}`, and archive rows may hold the joined string `name/version,…`. all of it is hostile:
// few, short, lowercase `[a-z0-9_-]`, deduped, junk dropped
use serde_json::Value;

use crate::images::url::allowed;

type Pairs = Box<[(Box<str>, Box<str>)]>;

const MAX_BADGES: usize = 16;
const MAX_NAME: usize = 32;
// a believable count: 100 years of months (the server's own bound)
const MAX_COUNT: u64 = 1200;

fn name_of(s: &str) -> Option<Box<str>> {
  let s = s.trim();
  let ok = (1..=MAX_NAME).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
  ok.then(|| s.to_ascii_lowercase().into())
}

fn count_of(v: Option<&Value>) -> Option<u64> {
  v.and_then(Value::as_u64).filter(|n| (1..=MAX_COUNT).contains(n))
}

fn push(out: &mut Vec<(Box<str>, Box<str>)>, name: Box<str>, version: String) {
  if out.len() < MAX_BADGES && !out.iter().any(|b| b.0 == name) {
    out.push((name, version.into()));
  }
}

fn field(b: &Value) -> Option<&str> {
  ["name", "type"].iter().find_map(|k| b.get(k).and_then(Value::as_str))
}

// kick: a subscriber's version is their months (what the channel's tier art is picked by), anything
// else is "1". a line with none gets the empty slice, which costs nothing
pub fn kick(v: Option<&Value>) -> Pairs {
  let mut out = Vec::new();
  match v {
    Some(Value::Array(list)) => {
      for b in list.iter().take(MAX_BADGES * 2) {
        let Some(name) = field(b).and_then(name_of) else { continue };
        let version = count_of(b.get("count")).map_or_else(|| "1".to_owned(), |n| n.to_string());
        push(&mut out, name, version);
      }
    }
    Some(Value::String(s)) => {
      for part in s.split(',').take(MAX_BADGES * 2) {
        let (name, ver) = part.rsplit_once('/').unwrap_or((part, "1"));
        let Some(name) = name_of(name) else { continue };
        let ver = ver.trim();
        let ok = (1..=4).contains(&ver.len()) && ver.bytes().all(|b| b.is_ascii_digit());
        push(&mut out, name, if ok { ver.to_owned() } else { "1".to_owned() });
      }
    }
    _ => {}
  }
  out.into()
}

// youtube: roles only (broadcaster, moderator, verified, member). a member badge may carry the channel's own
// art (`url`, an image host we allow): it rides in the version slot, which youtube has no other use for, and
// is what the line draws. without one (or with a url off the allowlist) the version stays "1"
pub fn youtube(v: Option<&Value>) -> Pairs {
  let mut out = Vec::new();
  for b in v.and_then(Value::as_array).into_iter().flatten().take(MAX_BADGES * 2) {
    if let Some(name) = b.as_str().or_else(|| field(b)).and_then(name_of) {
      let art = b.get("url").and_then(Value::as_str).filter(|u| &*name == "member" && allowed(u));
      push(&mut out, name, art.unwrap_or("1").to_owned());
    }
  }
  out.into()
}

// the art url a youtube badge's version slot holds, if it holds one
pub fn art(version: &str) -> Option<&str> {
  allowed(version).then_some(version)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn names(p: &Pairs) -> Vec<(&str, &str)> {
    p.iter().map(|(a, b)| (&**a, &**b)).collect()
  }

  #[test]
  fn kick_server_shape_and_months() {
    let v = json!([{"name": "Moderator", "version": "1"}, {"name": "subscriber", "version": "1", "count": 7}, {"type": "og"}]);
    assert_eq!(names(&kick(Some(&v))), [("moderator", "1"), ("subscriber", "7"), ("og", "1")]);
  }

  #[test]
  fn kick_joined_string_from_archive_rows() {
    let v = json!("subscriber/12,Verified/1,bad name/1,vip/zzz");
    assert_eq!(names(&kick(Some(&v))), [("subscriber", "12"), ("verified", "1"), ("vip", "1")]);
  }

  #[test]
  fn hostile_badges_are_bounded_or_dropped() {
    for v in [json!(null), json!(5), json!({}), json!([null, 3, [], {"name": 4}, {"name": ""}, {"name": "a b"}])] {
      assert!(kick(Some(&v)).is_empty() && youtube(Some(&v)).is_empty());
    }
    let long = json!([{"name": "x".repeat(500)}]);
    assert!(kick(Some(&long)).is_empty());
    let many: Vec<Value> = (0..500).map(|i| json!({"name": format!("b{i}")})).collect();
    assert_eq!(kick(Some(&json!(many))).len(), MAX_BADGES);
    assert_eq!(youtube(Some(&json!(many))).len(), MAX_BADGES);
    // a count off the believable range is a plain "1", and a name shows once
    let v = json!([{"name": "subscriber", "count": 99999999}, {"name": "subscriber", "count": 3}]);
    assert_eq!(names(&kick(Some(&v))), [("subscriber", "1")]);
    let v = json!([{"name": "subscriber", "count": -2}, {"name": "subscriber", "count": 1.5}]);
    assert_eq!(names(&kick(Some(&v))), [("subscriber", "1")]);
    assert!(kick(None).is_empty());
  }

  #[test]
  fn youtube_roles() {
    let v = json!([{"name": "broadcaster", "version": "1"}, "moderator", {"type": "member"}]);
    assert_eq!(names(&youtube(Some(&v))), [("broadcaster", "1"), ("moderator", "1"), ("member", "1")]);
  }

  #[test]
  fn youtube_member_art_is_kept_only_from_an_allowed_host() {
    let art = "https://yt3.ggpht.com/abc=s16-c-k";
    let v = json!([{"name": "member", "version": "1", "url": art, "title": "Member (2 months)"}]);
    assert_eq!(names(&youtube(Some(&v))), [("member", art)]);
    assert_eq!(super::art(art), Some(art));
    assert_eq!(super::art("1"), None);
    for bad in
      ["http://yt3.ggpht.com/a", "https://evil.example/a.png", "https://yt3.ggpht.com.evil.io/a", "javascript:1"]
    {
      let v = json!([{"name": "member", "url": bad}]);
      assert_eq!(names(&youtube(Some(&v))), [("member", "1")], "{bad}");
    }
    // only the member badge takes art; a url on a role is ignored
    let v = json!([{"name": "moderator", "url": art}]);
    assert_eq!(names(&youtube(Some(&v))), [("moderator", "1")]);
  }
}
