// what 7tv says about personal emote sets, read as hostile input: the EventAPI's `entitlement.create|delete`
// (who owns a set) and `emote_set.update` (a live edit of one), the set itself (`GET /v3/emote-sets/<id>`)
// and the owned sets in a user lookup. ids are checked, lists capped, names must be one clean token, and
// every picture url is built by us on the 7tv cdn (`live::emote_of`), never copied from the payload
use serde_json::Value;

use super::Set;
use super::live::{emote_of, valid_id};
use super::personal::Ev;

// emotes kept per set: a personal set holds a handful (5 today), an event set a few more
pub const MAX_SET_EMOTES: usize = 100;
// connections read per entitled user, and twitch ids taken from them (one 7tv account can link a few)
const MAX_CONNECTIONS: usize = 16;
const MAX_OWNERS: usize = 4;
// 7tv's EmoteSetFlag::Personal
const PERSONAL: u64 = 4;

fn twitch_id(v: &Value) -> Option<u64> {
  let id = match v {
    Value::String(s) if (1..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) => s.parse().ok(),
    Value::Number(n) => n.as_u64(),
    _ => None,
  };
  id.filter(|&n| n > 0)
}

// `entitlement.create` / `.delete` body → the set it names and the twitch accounts that own it. only an
// EMOTE_SET entitlement counts (paints, badges and kinds we do not know are not this), and only the
// owner's TWITCH connections: a set is never tied to a chatter by anything but their twitch id
fn entitlement(body: &Value) -> Option<(Box<str>, Vec<u64>)> {
  let o = body.get("object")?;
  if o.get("kind")?.as_str()? != "EMOTE_SET" {
    return None;
  }
  let set = o.get("ref_id")?.as_str().filter(|s| valid_id(s))?;
  let conns = o.get("user")?.get("connections")?.as_array()?;
  let users: Vec<u64> = conns
    .iter()
    .take(MAX_CONNECTIONS)
    .filter(|c| c.get("platform").and_then(Value::as_str) == Some("TWITCH"))
    .filter_map(|c| twitch_id(c.get("id")?))
    .take(MAX_OWNERS)
    .collect();
  (!users.is_empty()).then(|| (set.into(), users))
}

// an `entitlement.create` / `entitlement.delete` dispatch → who got a personal set or lost it (None = not that)
pub fn event(kind: &str, body: &Value) -> Option<Ev> {
  match kind {
    "entitlement.create" => entitlement(body).map(|(set, users)| Ev::Grant { users, set }),
    "entitlement.delete" => entitlement(body).map(|(set, users)| Ev::Revoke { users, set }),
    _ => None,
  }
}

// a whole set as `GET /v3/emote-sets/<set>` answers: the first emote of a name wins, at most MAX_SET_EMOTES.
// a body that names another set is not the answer to the ask
pub fn set_emotes(v: &Value, set: &str) -> Option<Set> {
  if v.get("id")?.as_str()? != set {
    return None;
  }
  let mut out = Set::new();
  for e in v.get("emotes").and_then(Value::as_array).into_iter().flatten().take(MAX_SET_EMOTES) {
    if let Some((_, name, emote)) = emote_of(e) {
      out.entry(name).or_insert(emote);
    }
  }
  Some(out)
}

// the personal sets a user lookup (`GET /v3/users/twitch/<id>` → `user.emote_sets`) says they own
pub fn own_sets(user: &Value) -> Vec<Box<str>> {
  let sets = user.get("emote_sets").and_then(Value::as_array);
  sets
    .into_iter()
    .flatten()
    .take(MAX_CONNECTIONS)
    .filter(|s| s.get("flags").and_then(Value::as_u64).is_some_and(|f| f & PERSONAL != 0))
    .filter_map(|s| s.get("id")?.as_str().filter(|i| valid_id(i)))
    .take(MAX_OWNERS)
    .map(Into::into)
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn grant(kind: &str, set: &str, conns: Value) -> Value {
    json!({"id": "0", "kind": 5, "object": {"id": "0", "kind": kind, "ref_id": set,
      "user": {"id": "U1", "username": "x", "connections": conns}}})
  }

  fn twitch(id: &str) -> Value {
    json!({"id": id, "platform": "TWITCH", "username": "x"})
  }

  #[test]
  fn an_emote_set_entitlement_names_the_twitch_accounts_that_own_the_set() {
    let b = grant("EMOTE_SET", "01SET", json!([{"id": "9", "platform": "DISCORD"}, twitch("555"), twitch("777")]));
    let Some(Ev::Grant { users, set }) = event("entitlement.create", &b) else { panic!("not a grant") };
    assert_eq!((&*set, users), ("01SET", vec![555, 777]));
    assert!(matches!(event("entitlement.delete", &b), Some(Ev::Revoke { .. })));
  }

  #[test]
  fn hostile_entitlements_are_dropped() {
    let ok = |b: Value| event("entitlement.create", &b).is_some();
    assert!(ok(grant("EMOTE_SET", "S", json!([twitch("5")]))));
    // other kinds, and kinds we have never heard of
    for kind in ["PAINT", "BADGE", "emote_set", "", "SOMETHING_NEW"] {
      assert!(!ok(grant(kind, "S", json!([twitch("5")]))), "{kind}");
    }
    // a set id that could escape the url or is not an id at all
    for set in ["", "../x", "a b", "a/b", &"A".repeat(41)] {
      assert!(!ok(grant("EMOTE_SET", set, json!([twitch("5")]))), "{set}");
    }
    // no twitch account to tie it to: other platforms, junk ids, an id that is not a number
    for conns in [
      json!([]),
      json!([{"id": "5", "platform": "KICK"}]),
      json!([twitch("")]),
      json!([twitch("0")]),
      json!([twitch("-5")]),
      json!([twitch("5x")]),
      json!([twitch("123456789012345678901")]),
      json!([{"id": ["5"], "platform": "TWITCH"}]),
      json!("TWITCH"),
      json!(null),
    ] {
      assert!(!ok(grant("EMOTE_SET", "S", conns.clone())), "{conns}");
    }
    for b in [json!(null), json!({}), json!({"object": 5}), json!({"object": {"kind": "EMOTE_SET"}}), json!([1])] {
      assert!(!ok(b));
    }
    // an event type we do not read
    assert!(event("entitlement.update", &grant("EMOTE_SET", "S", json!([twitch("5")]))).is_none());
    assert!(event("user.update", &json!({})).is_none());
  }

  #[test]
  fn owners_and_connections_are_bounded() {
    let many: Vec<Value> = (1..100).map(|i| twitch(&i.to_string())).collect();
    let Some(Ev::Grant { users, .. }) = event("entitlement.create", &grant("EMOTE_SET", "S", json!(many))) else {
      panic!("not a grant")
    };
    assert_eq!(users.len(), MAX_OWNERS);
    // twitch connections past the first 16 connections are never read
    let mut late: Vec<Value> = (0..MAX_CONNECTIONS).map(|_| json!({"id": "1", "platform": "KICK"})).collect();
    late.push(twitch("5"));
    assert!(event("entitlement.create", &grant("EMOTE_SET", "S", json!(late))).is_none());
  }

  fn row(id: &str, name: &str) -> Value {
    json!({"id": id, "name": name, "flags": 0, "data": {"flags": 0, "host": {"url": "//evil.example/x",
      "files": [{"name": "1x.webp", "width": 32, "height": 28}]}}})
  }

  #[test]
  fn a_set_keeps_clean_names_and_builds_its_own_urls() {
    let body = json!({"id": "S1", "name": "Personal Emotes", "flags": 4, "emotes": [
      row("A1", "Kappa"), row("B2", "has space"), row("../C", "Evil"), row("D4", ""), row("E5", "Kappa"), row("F6", "Pog")]});
    let set = set_emotes(&body, "S1").unwrap();
    let mut names: Vec<_> = set.keys().map(|k| &**k).collect();
    names.sort_unstable();
    assert_eq!(names, ["Kappa", "Pog"]);
    // the picture is the 7tv cdn's, whatever the payload's host says, at the file's native size
    let e = &set["Kappa"];
    assert_eq!((&*e.url, e.w, e.h), ("https://cdn.7tv.app/emote/A1/1x.webp", 32, 28));
  }

  #[test]
  fn a_huge_or_wrong_set_is_cut_or_refused() {
    let many: Vec<Value> = (0..5000).map(|i| row(&format!("I{i}"), &format!("n{i}"))).collect();
    assert_eq!(set_emotes(&json!({"id": "S", "emotes": many}), "S").unwrap().len(), MAX_SET_EMOTES);
    // a body for another set, with no id, or not an object: not the answer
    assert!(set_emotes(&json!({"id": "OTHER", "emotes": [row("A", "x1")]}), "S").is_none());
    assert!(set_emotes(&json!({"emotes": [row("A", "x1")]}), "S").is_none());
    assert!(set_emotes(&json!([1, 2]), "S").is_none());
    // an empty set (7tv leaves `emotes` out of one) is an answer: nothing
    assert_eq!(set_emotes(&json!({"id": "S", "capacity": 5}), "S").unwrap().len(), 0);
    assert_eq!(set_emotes(&json!({"id": "S", "emotes": "no"}), "S").unwrap().len(), 0);
  }

  #[test]
  fn owned_personal_sets_are_the_flagged_ones() {
    let user = json!({"emote_sets": [
      {"id": "CHAN", "name": "x's Emotes", "flags": 0},
      {"id": "PERS", "name": "Personal Emotes", "flags": 4},
      {"id": "../bad", "flags": 4}, {"id": "NOFLAG"}, {"flags": 4}, 5, null]});
    assert_eq!(own_sets(&user), vec![Box::<str>::from("PERS")]);
    assert!(own_sets(&json!({})).is_empty());
    assert!(own_sets(&json!({"emote_sets": "no"})).is_empty());
    let many: Vec<Value> = (0..50).map(|i| json!({"id": format!("S{i}"), "flags": 4})).collect();
    assert_eq!(own_sets(&json!({"emote_sets": many})).len(), MAX_OWNERS);
  }
}
