// live 7tv channel-emote edits from the EventAPI (`emote_set.update`): adds, removals, renames.
// the body is hostile: ids are checked, lists capped, every url is built by us on the 7tv cdn
use serde_json::Value;

use super::{Emote, MAX_SET, Provider, Scope, valid_name};

const MAX_ITEMS: usize = 200;

#[derive(Default, Debug, Clone)]
pub struct Delta {
  // (emote id, name, emote)
  pub add: Vec<(Box<str>, Box<str>, Emote)>,
  pub remove: Vec<Box<str>>,
  // (emote id, new name, zero-width when the event says)
  pub rename: Vec<(Box<str>, Box<str>, Option<bool>)>,
}

impl Delta {
  pub fn is_empty(&self) -> bool {
    self.add.is_empty() && self.remove.is_empty() && self.rename.is_empty()
  }
}

pub fn valid_id(id: &str) -> bool {
  (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

// the 7tv emote id inside one of our cdn urls
pub fn id_of(url: &str) -> Option<&str> {
  url.strip_prefix("https://cdn.7tv.app/emote/")?.split('/').next()
}

fn flags_zero_width(v: &Value) -> Option<bool> {
  let f = v.get("flags").and_then(Value::as_u64);
  let d = v.get("data").and_then(|d| d.get("flags")).and_then(Value::as_u64);
  (f.is_some() || d.is_some()).then(|| f.unwrap_or(0) & 1 != 0 || d.unwrap_or(0) & 256 != 0)
}

fn emote_of(v: &Value) -> Option<(Box<str>, Box<str>, Emote)> {
  let id = v.get("id")?.as_str().filter(|i| valid_id(i))?;
  let name = v.get("name")?.as_str().filter(|n| valid_name(n))?;
  let file = v
    .get("data")
    .and_then(|d| d.get("host"))
    .and_then(|h| h.get("files"))
    .and_then(Value::as_array)
    .and_then(|fs| fs.iter().find(|f| f.get("name").and_then(Value::as_str) == Some("1x.webp")));
  let dim = |k| file.and_then(|f| f.get(k)).and_then(Value::as_u64).map_or(0, |n| n.min(u16::MAX as u64) as u16);
  let e = Emote {
    url: format!("https://cdn.7tv.app/emote/{id}/1x.webp").into(),
    w: dim("width"),
    h: dim("height"),
    zero_width: flags_zero_width(v).unwrap_or(false),
  };
  Some((id.into(), name.into(), e))
}

fn items<'a>(body: &'a Value, k: &str) -> impl Iterator<Item = &'a Value> {
  body
    .get(k)
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .take(MAX_ITEMS)
    .filter(|i| i.get("key").and_then(Value::as_str) == Some("emotes"))
}

// set id + what changed
pub fn parse(body: &Value) -> Option<(Box<str>, Delta)> {
  let set = body.get("id")?.as_str().filter(|i| valid_id(i))?;
  let mut d = Delta::default();
  for i in items(body, "pushed") {
    d.add.extend(i.get("value").and_then(emote_of));
  }
  for i in items(body, "pulled") {
    if let Some(id) = i.get("old_value").and_then(|o| o.get("id")).and_then(Value::as_str).filter(|i| valid_id(i)) {
      d.remove.push(id.into());
    }
  }
  for i in items(body, "updated") {
    let Some(v) = i.get("value") else { continue };
    let id = v.get("id").and_then(Value::as_str).filter(|i| valid_id(i));
    let name = v.get("name").and_then(Value::as_str).filter(|n| valid_name(n));
    if let (Some(id), Some(name)) = (id, name) {
      d.rename.push((id.into(), name.into(), flags_zero_width(v)));
    }
  }
  Some((set.into(), d))
}

impl super::Emotes {
  // apply an edit to a channel's 7tv set. true when anything changed
  pub fn patch(&mut self, channel: &str, d: &Delta) -> bool {
    let scope: &mut Scope = self.channels.entry(channel.into()).or_default();
    let set = scope.sets.entry(Provider::SevenTv).or_default();
    let before = set.len();
    let find = |set: &super::Set, id: &str| set.iter().find(|(_, e)| id_of(&e.url) == Some(id)).map(|(k, _)| k.clone());
    let mut changed = false;
    for id in &d.remove {
      if let Some(k) = find(set, id) {
        set.remove(&k);
        changed = true;
      }
    }
    for (id, name, zw) in &d.rename {
      if let Some(k) = find(set, id)
        && let Some(mut e) = set.remove(&k)
      {
        e.zero_width = zw.unwrap_or(e.zero_width);
        set.insert(name.clone(), e);
        changed = true;
      }
    }
    for (id, name, e) in &d.add {
      // a re-push of an emote we hold under another name is a rename
      if let Some(k) = find(set, id) {
        set.remove(&k);
      }
      if set.len() < MAX_SET || set.contains_key(name) {
        set.insert(name.clone(), e.clone());
        changed = true;
      }
    }
    changed || set.len() != before
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::emotes::Emotes;
  use serde_json::json;

  fn pushed(id: &str, name: &str) -> Value {
    json!({"key": "emotes", "index": 0, "value": {"id": id, "name": name, "flags": 0,
      "data": {"flags": 256, "host": {"files": [{"name": "1x.webp", "width": 32, "height": 28}]}}}})
  }

  #[test]
  fn add_rename_remove_roundtrip() {
    let mut em = Emotes::default();
    let body = json!({"id": "SET1", "pushed": [pushed("A1", "Kappa"), pushed("B2", "Pog")]});
    let (set, d) = parse(&body).unwrap();
    assert_eq!(&*set, "SET1");
    assert!(em.patch("c", &d));
    let e = em.channel("c", "Kappa").unwrap();
    assert_eq!((&*e.url, e.w, e.h, e.zero_width), ("https://cdn.7tv.app/emote/A1/1x.webp", 32, 28, true));
    let ren = json!({"id": "SET1", "updated": [{"key": "emotes", "old_value": {"id": "A1", "name": "Kappa"},
      "value": {"id": "A1", "name": "Kappa2"}}]});
    assert!(em.patch("c", &parse(&ren).unwrap().1));
    assert!(em.channel("c", "Kappa").is_none());
    assert!(em.channel("c", "Kappa2").unwrap().zero_width);
    let gone = json!({"id": "SET1", "pulled": [{"key": "emotes", "old_value": {"id": "B2", "name": "Pog"}}]});
    assert!(em.patch("c", &parse(&gone).unwrap().1));
    assert!(em.channel("c", "Pog").is_none());
    // removing what isn't there changes nothing
    assert!(!em.patch("c", &parse(&gone).unwrap().1));
  }

  #[test]
  fn hostile_bodies_are_dropped() {
    assert!(parse(&json!({"id": "bad id!"})).is_none());
    assert!(parse(&json!({"id": 5})).is_none());
    let body = json!({"id": "S", "pushed": [
      pushed("../../evil", "x"), pushed("A", "has space"), pushed("A", ""),
      {"key": "other", "value": {"id": "Z", "name": "z"}}, {"value": 7}, 3, null]});
    assert!(parse(&body).unwrap().1.is_empty());
    let many: Vec<Value> = (0..1000).map(|i| pushed(&format!("I{i}"), &format!("n{i}"))).collect();
    assert_eq!(parse(&json!({"id": "S", "pushed": many})).unwrap().1.add.len(), MAX_ITEMS);
  }

  #[test]
  fn set_stays_bounded() {
    let mut em = Emotes::default();
    let d = Delta {
      add: (0..MAX_SET + 50)
        .map(|i| {
          (format!("I{i}").into(), format!("n{i}").into(), Emote { url: "u".into(), w: 0, h: 0, zero_width: false })
        })
        .collect(),
      ..Delta::default()
    };
    em.patch("c", &d);
    assert_eq!(em.channel_names("c").len(), MAX_SET);
  }
}
