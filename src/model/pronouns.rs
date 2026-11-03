// someone's pronouns on their card and name popup (ext fetch_pronouns): what they set on pronoundb.org,
// asked by twitch user id (pronoundb has no kick or youtube). the answer is hostile: a few short words or
// nothing. answers are kept a day (they almost never change), a failed ask is tried again after 10min
use std::collections::HashMap;

use serde_json::Value;

const DAY_MS: i64 = 24 * 3_600_000;
const FAILED_MS: i64 = 10 * 60_000;
// an ask with no answer yet; past this it may be asked again
const ASKING_MS: i64 = 15_000;
const CAP: usize = 500;
// the ext shows the first three sets
const MAX_SETS: usize = 3;
const MAX_WORD: usize = 12;

pub fn url(uid: u64) -> String {
  format!("https://pronoundb.org/api/v2/lookup?platform=twitch&ids={uid}")
}

// `{"<uid>": {"sets": {"en": ["he", "they"]}}}` → "he/they". anything that isn't a short plain word is
// dropped; nothing left = they set none
pub fn parse(v: &Value, uid: u64) -> Option<Box<str>> {
  let sets = v.get(uid.to_string())?.pointer("/sets/en")?.as_array()?;
  let words: Vec<String> = sets
    .iter()
    .take(MAX_SETS)
    .filter_map(Value::as_str)
    .filter(|w| (1..=MAX_WORD).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_alphabetic()))
    .map(str::to_ascii_lowercase)
    .collect();
  (!words.is_empty()).then(|| words.join("/").into())
}

enum State {
  Asking,
  // None = they set none
  Known(Option<Box<str>>),
  Failed,
}

#[derive(Default)]
pub struct Pronouns {
  by: HashMap<u64, (i64, State)>,
}

impl Pronouns {
  fn fresh(&self, uid: u64, now: i64) -> Option<&State> {
    let (at, s) = self.by.get(&uid)?;
    let ttl = match s {
      State::Asking => ASKING_MS,
      State::Known(_) => DAY_MS,
      State::Failed => FAILED_MS,
    };
    (now - at < ttl).then_some(s)
  }

  // what to show beside their name, when known
  pub fn get(&self, uid: u64, now: i64) -> Option<&str> {
    match self.fresh(uid, now)? {
      State::Known(w) => w.as_deref(),
      _ => None,
    }
  }

  // true = nobody asked lately: the caller asks (and it is marked as asked, so a second look waits)
  pub fn want(&mut self, uid: u64, now: i64) -> bool {
    if self.fresh(uid, now).is_some() {
      return false;
    }
    self.set(uid, now, State::Asking);
    true
  }

  // the answer landed: Ok(words or none), Err = no answer (asked again later)
  pub fn put(&mut self, uid: u64, res: Result<Option<Box<str>>, ()>, now: i64) {
    self.set(uid, now, res.map_or(State::Failed, State::Known));
  }

  fn set(&mut self, uid: u64, now: i64, s: State) {
    if self.by.len() >= CAP && !self.by.contains_key(&uid) {
      let old = self.by.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| *k);
      if let Some(k) = old {
        self.by.remove(&k);
      }
    }
    self.by.insert(uid, (now, s));
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn sets_read_like_the_ext() {
    let p = |v: Value| parse(&v, 42);
    assert_eq!(p(json!({"42": {"sets": {"en": ["he", "they"]}}})).as_deref(), Some("he/they"));
    assert_eq!(p(json!({"42": {"sets": {"en": ["She"]}}})).as_deref(), Some("she"));
    // a real answer (2026-11), other languages and the decoration ignored
    let real = json!({"94762492923748352":{"decoration":"catgirl_chief","sets":{"en":["she","it"],"fr":["elle"]}}});
    assert_eq!(parse(&real, 94762492923748352).as_deref(), Some("she/it"));
    assert_eq!(p(json!({"42": {"sets": {"en": ["any", "it", "he", "she"]}}})).as_deref(), Some("any/it/he"));
    // somebody else's answer, nothing set, junk
    for v in [
      json!({}),
      json!({"7": {"sets": {"en": ["he"]}}}),
      json!({"42": {"sets": {}}}),
      json!({"42": {"sets": {"en": []}}}),
      json!({"42": {"sets": {"en": "he"}}}),
      json!({"42": null}),
      json!([1]),
      json!(null),
    ] {
      assert_eq!(p(v.clone()), None, "{v}");
    }
    // hostile words are dropped, the rest stay
    let v = json!({"42": {"sets": {"en": ["he\u{202e}", "x".repeat(13), 5, "they", "<b>"]}}});
    assert_eq!(p(v), None, "only the first three sets are read, none of them clean");
    let v = json!({"42": {"sets": {"en": ["she", "a b", "they"]}}});
    assert_eq!(p(v).as_deref(), Some("she/they"));
  }

  #[test]
  fn asked_once_kept_a_day_failures_retried() {
    let mut p = Pronouns::default();
    assert!(p.want(1, 0));
    assert!(!p.want(1, 1000), "asking: a second look waits");
    assert!(p.want(1, ASKING_MS), "an ask that never answered may go again");
    p.put(1, Ok(Some("he/him".into())), 0);
    assert_eq!(p.get(1, DAY_MS - 1), Some("he/him"));
    assert!(!p.want(1, DAY_MS - 1));
    assert_eq!(p.get(1, DAY_MS), None);
    assert!(p.want(1, DAY_MS), "a day later: ask again");
    p.put(2, Ok(None), 0);
    assert!(!p.want(2, 1000), "none set is an answer too");
    p.put(3, Err(()), 0);
    assert!(!p.want(3, FAILED_MS - 1));
    assert!(p.want(3, FAILED_MS));
  }

  #[test]
  fn the_cache_stays_bounded() {
    let mut p = Pronouns::default();
    for i in 0..CAP as u64 + 50 {
      p.put(i, Ok(None), i as i64);
    }
    assert_eq!(p.by.len(), CAP);
    assert!(!p.by.contains_key(&0), "the oldest went first");
  }
}
