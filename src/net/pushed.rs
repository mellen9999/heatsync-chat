// the server's "this changed" pushes, read as hostile json: a profile colour, a feed post hidden / edited /
// updated / moderated, a bookmark, the unread count, plus. the server's emits are flat (`{type, ...fields}`);
// the typed protocol wraps some in `data`, so both are read. every id is checked, every list bounded
use serde_json::Value;

use crate::heatsync::content::Cat;

use crate::model::{color, ident, post};

// usernames a colour frame may name (the server sends the base name and each linked alias)
const MAX_NAMES: usize = 10;
const MAX_CATS: usize = 8;
// a post count or heat is a number of people: anything past this is a lie
const MAX_COUNT: f64 = 1e9;
// the biggest unread count shown: the server counts its 200 newest anyway
const MAX_UNREAD: u32 = 1_000_000;

#[derive(Debug, PartialEq)]
pub struct ProfileColor {
  // lowercase logins
  pub names: Vec<Box<str>>,
  pub color: u32,
}

pub struct Edit {
  pub id: Box<str>,
  pub text: Box<str>,
  pub refs: Option<Box<[crate::model::message::HsEmote]>>,
}

pub struct Meta {
  pub id: Box<str>,
  pub replies: Option<u32>,
  pub heat: Option<u32>,
}

#[derive(Debug, PartialEq)]
pub enum Mod {
  Approved,
  // what moderation flagged the post's media for
  Flagged(Vec<Box<str>>),
}

// the fields of a frame: the frame itself, or its `data` object
fn body(v: &Value) -> &Value {
  v.get("data").filter(|d| d.is_object()).unwrap_or(v)
}

// a post id as the server writes it: base36, lowercased for lookups
fn post_id(v: Option<&Value>) -> Option<Box<str>> {
  v.and_then(Value::as_str).filter(|i| post::id_ok(i)).map(|i| i.to_ascii_lowercase().into())
}

fn count(v: Option<&Value>) -> Option<u32> {
  // a count is a whole number: a number or a numeric string (postgres counts arrive both ways)
  let n = match v? {
    Value::Number(n) => n.as_f64()?,
    Value::String(s) => s.parse::<f64>().ok()?,
    _ => return None,
  };
  (n.is_finite() && n >= 0.0).then_some(n.min(MAX_COUNT) as u32)
}

// `profile:color {userId, usernames[], color}`: a `#rrggbb` colour (readable on black) for up to ten names
pub fn profile_color(v: &Value) -> Option<ProfileColor> {
  let hex = v.get("color").and_then(Value::as_str).filter(|c| c.len() == 7)?;
  let rgb = color::parse_hex(hex)?;
  // the id says whose; it is only checked, never kept
  v.get("userId").and_then(|u| match u {
    Value::String(s) => Some(s.len()).filter(|n| (1..=64).contains(n)),
    Value::Number(_) => Some(1),
    _ => None,
  })?;
  let mut names: Vec<Box<str>> = Vec::new();
  for n in v.get("usernames").and_then(Value::as_array)?.iter().take(MAX_NAMES).filter_map(Value::as_str) {
    let low: Box<str> = n.to_ascii_lowercase().into();
    if ident::relay_key(&low) && !names.contains(&low) {
      names.push(low);
    }
  }
  (!names.is_empty()).then_some(ProfileColor { names, color: color::readable(rgb) })
}

// `message:hidden {id, reason}`: the post is gone for everyone (the reason is never shown)
pub fn hidden(v: &Value) -> Option<Box<str>> {
  post_id(body(v).get("id"))
}

// `message-edited {message_id, content, emote_refs, …}`: the post's new words and refs
pub fn edited(v: &Value) -> Option<Edit> {
  let b = body(v);
  let id = post_id(b.get("message_id").or_else(|| b.get("base36_id")))?;
  let text = post::words(b.get("content").and_then(Value::as_str)?);
  if text.is_empty() {
    return None;
  }
  // refs only when the frame carries an object: null means the edit has none
  let refs = b.get("emote_refs").filter(|r| r.is_object() || r.is_null()).map(|r| post::refs_of(Some(r)));
  Some(Edit { id, text, refs })
}

// `message-updated {base36_id, reply_count, heat, …}`: the thread root's numbers moved (the raw row rides
// along, so only the two numbers are read)
pub fn updated(v: &Value) -> Option<Meta> {
  let b = body(v);
  let id = post_id(b.get("base36_id"))?;
  let (replies, heat) = (count(b.get("reply_count")), count(b.get("heat")));
  (replies.is_some() || heat.is_some()).then_some(Meta { id, replies, heat })
}

// `message:approved {id}` / `message:flagged {id, categories[]}`
pub fn moderated(v: &Value, flagged: bool) -> Option<(Box<str>, Mod)> {
  let b = body(v);
  let id = post_id(b.get("id"))?;
  if !flagged {
    return Some((id, Mod::Approved));
  }
  let cats: Vec<Box<str>> = b
    .get("categories")
    .and_then(Value::as_array)?
    .iter()
    .filter_map(Value::as_str)
    .filter(|c| (1..=32).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_' || b == b'-'))
    .take(MAX_CATS)
    .map(Into::into)
    .collect();
  (!cats.is_empty()).then_some((id, Mod::Flagged(cats)))
}

// `bookmarks:changed {id, on}`
pub fn bookmark(v: &Value) -> Option<(Box<str>, bool)> {
  Some((post_id(v.get("id"))?, v.get("on").and_then(Value::as_bool)?))
}

// `notifications:read {unread}`: your unread count after a look elsewhere (never below 0)
pub fn unread(v: &Value) -> Option<u32> {
  let n = v.get("unread").and_then(Value::as_f64).filter(|n| n.is_finite())?;
  Some(n.clamp(0.0, MAX_UNREAD as f64) as u32)
}

// the site's per-category switches for flagged emotes and pictures, from `user_settings:update` and
// `GET /api/user/settings`: a real bool counts, anything else is nothing. the site names the categories
// sexual, gore, weapon, drug and hate; this window's switches are nsfw, violence, weapons, drugs and hate.
// `show_nsfw_emotes` is the site's older single switch, and the `allow_*` keys its chat-media gates: neither
// has a switch here
pub fn content_prefs(v: &Value) -> Vec<(Cat, bool)> {
  [
    ("show_sexual_emotes", Cat::Sexual),
    ("show_gore_emotes", Cat::Violence),
    ("show_weapon_emotes", Cat::Weapons),
    ("show_drug_emotes", Cat::Drugs),
    ("show_hate_emotes", Cat::Hate),
  ]
  .into_iter()
  .filter_map(|(k, c)| v.get(k).and_then(Value::as_bool).map(|on| (c, on)))
  .collect()
}

// `user_settings:update` also says if plus moved: true when the frame says plus moved
pub fn plus_moved(v: &Value) -> bool {
  v.get("plus").is_some_and(Value::is_boolean) || v.get("plus_expires_at").is_some_and(|p| p.is_null() || p.is_string())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn content_switches_read_real_bools_for_the_five_categories_only() {
    let all = json!({"show_sexual_emotes": true, "show_gore_emotes": false, "show_weapon_emotes": true,
      "show_drug_emotes": false, "show_hate_emotes": true, "ts": 1});
    assert_eq!(
      content_prefs(&all),
      vec![(Cat::Sexual, true), (Cat::Violence, false), (Cat::Weapons, true), (Cat::Drugs, false), (Cat::Hate, true)]
    );
    // one key alone; the older switch, the chat-media gates, wrong types and junk are nothing
    assert_eq!(content_prefs(&json!({"show_gore_emotes": true})), vec![(Cat::Violence, true)]);
    for bad in [
      json!({"show_nsfw_emotes": true, "allow_nsfw_images": true, "allow_weapons": true}),
      json!({"show_sexual_emotes": "true", "show_gore_emotes": 1, "show_hate_emotes": null}),
      json!({"show_drug_emotes": [true], "show_weapon_emotes": {"a": true}}),
      json!("show_sexual_emotes"),
      json!(null),
    ] {
      assert!(content_prefs(&bad).is_empty(), "{bad}");
    }
  }

  #[test]
  fn a_colour_frame_is_checked_and_bounded() {
    let ok = json!({"type":"profile:color","userId":"42","usernames":["Knut","knut","kick_knut"],"color":"#ff8800"});
    let got = profile_color(&ok).unwrap();
    assert_eq!(got.names, vec!["knut".into(), "kick_knut".into()], "lowercased, once each");
    assert_eq!(got.color, color::readable(0xff8800));
    // not #rrggbb, no id, no names, or no list at all: nothing
    for bad in [
      json!({"userId":"1","usernames":["a"],"color":"red"}),
      json!({"userId":"1","usernames":["a"],"color":"#f80"}),
      json!({"userId":"1","usernames":["a"],"color":"#ff88zz"}),
      json!({"usernames":["a"],"color":"#ff8800"}),
      json!({"userId":"1","usernames":[],"color":"#ff8800"}),
      json!({"userId":"1","usernames":["a b","<x>",7],"color":"#ff8800"}),
      json!({"userId":"1","usernames":"a","color":"#ff8800"}),
    ] {
      assert!(profile_color(&bad).is_none(), "{bad}");
    }
    let many: Vec<_> = (0..50).map(|i| format!("u{i}")).collect();
    let big = json!({"userId":"1","usernames":many,"color":"#ff8800"});
    assert_eq!(profile_color(&big).unwrap().names.len(), MAX_NAMES);
  }

  #[test]
  fn post_frames_need_a_real_id() {
    assert_eq!(hidden(&json!({"id":"4A","reason":"x"})).as_deref(), Some("4a"));
    assert_eq!(hidden(&json!({"data":{"id":"4a"}})).as_deref(), Some("4a"));
    for bad in [json!({"id":"../x"}), json!({"id":"123456789"}), json!({"id":""}), json!({"id":7}), json!({})] {
      assert!(hidden(&bad).is_none() && moderated(&bad, false).is_none() && bookmark(&bad).is_none(), "{bad}");
    }
    let e = edited(&json!({"message_id":"4a","content":"a &lt;b&gt;\u{7}","emote_refs":{"KEKW":"https://cdn.7tv.app/emote/1/1x.webp"}}))
      .unwrap();
    assert_eq!((&*e.id, &*e.text), ("4a", "a <b>"));
    assert_eq!(e.refs.unwrap().len(), 1);
    // refs absent = untouched, null = none
    assert!(edited(&json!({"message_id":"4a","content":"x"})).unwrap().refs.is_none());
    assert_eq!(edited(&json!({"message_id":"4a","content":"x","emote_refs":null})).unwrap().refs.unwrap().len(), 0);
    assert!(edited(&json!({"message_id":"4a","content":"   "})).is_none());
    assert!(edited(&json!({"message_id":"4a","content":7})).is_none());
    // an oversized edit is cut, not refused
    let long = "x".repeat(50_000);
    assert!(edited(&json!({"message_id":"4a","content":long})).unwrap().text.len() <= 4000);
  }

  #[test]
  fn numbers_are_clamped_and_moderation_names_are_known_ones() {
    let m = updated(&json!({"base36_id":"4a","reply_count":"7","heat":-3,"user_id":"x"})).unwrap();
    assert_eq!((m.replies, m.heat), (Some(7), None));
    let m = updated(&json!({"base36_id":"4a","reply_count":1e30})).unwrap();
    assert_eq!(m.replies, Some(1_000_000_000));
    assert!(updated(&json!({"base36_id":"4a","reply_count":"many"})).is_none());
    assert_eq!(moderated(&json!({"id":"4a"}), false), Some(("4a".into(), Mod::Approved)));
    let f = moderated(&json!({"id":"4a","categories":["nsfw","ev il","hate",9,"X"]}), true).unwrap();
    assert_eq!(f.1, Mod::Flagged(vec!["nsfw".into(), "hate".into()]));
    assert!(moderated(&json!({"id":"4a","categories":["Evil!"]}), true).is_none());
    assert_eq!(unread(&json!({"unread":-5})), Some(0));
    assert_eq!(unread(&json!({"unread":3})), Some(3));
    assert_eq!(unread(&json!({"unread":1e30})), Some(MAX_UNREAD));
    assert_eq!(unread(&json!({"unread":"3"})), None);
    assert_eq!(bookmark(&json!({"id":"4a","on":true})), Some(("4a".into(), true)));
    assert!(bookmark(&json!({"id":"4a","on":"yes"})).is_none());
    assert!(plus_moved(&json!({"plus":true})) && plus_moved(&json!({"plus_expires_at":null})));
    assert!(!plus_moved(&json!({"show_nsfw_emotes":true,"plus":"yes"})));
  }
}
