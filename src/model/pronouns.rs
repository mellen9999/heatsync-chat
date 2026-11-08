// someone's pronouns on their card and name popup (ext fetch_pronouns): what they set on pronoundb.org,
// asked through heatsync's own proxy (`GET /api/pronouns?platform=twitch&ids=<id>`) so a chatter's twitch id
// never reaches a third party. twitch ids only (pronoundb has no kick or youtube). the answer is hostile: a few
// short words or nothing. answers are kept a day (they almost never change), a failed or degraded ask is
// tried again after 10min
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
  format!("{}/api/pronouns?platform=twitch&ids={uid}", crate::net::ORIGIN)
}

// `{"pronouns": {"<uid>": ["he", "they"]}, "degraded": false}` → Ok(Some("he/they")). anything that isn't a
// short plain word is dropped; nothing left, or the id absent = they set none (Ok(None)). `degraded` true (or
// missing / not a bool) means the proxy could not find out, which is NOT "no pronouns": Err, so it is asked
// again later and never remembered as an answer
pub fn parse(v: &Value, uid: u64) -> Result<Option<Box<str>>, ()> {
  if v.get("degraded").and_then(Value::as_bool) != Some(false) {
    return Err(());
  }
  let map = v.get("pronouns").and_then(Value::as_object).ok_or(())?;
  let Some(sets) = map.get(&uid.to_string()).and_then(Value::as_array) else {
    return Ok(None);
  };
  let words: Vec<String> = sets
    .iter()
    .take(MAX_SETS)
    .filter_map(Value::as_str)
    .filter(|w| (1..=MAX_WORD).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_alphabetic()))
    .map(str::to_ascii_lowercase)
    .collect();
  Ok((!words.is_empty()).then(|| words.join("/").into()))
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
  fn the_ask_goes_to_heatsync_never_pronoundb() {
    assert_eq!(url(42), "https://heatsync.org/api/pronouns?platform=twitch&ids=42");
  }

  #[test]
  fn sets_read_like_the_ext() {
    let p = |v: Value| parse(&v, 42);
    let ok = |w: Value| json!({"pronouns": {"42": w}, "degraded": false});
    assert_eq!(p(ok(json!(["he", "they"]))), Ok(Some("he/they".into())));
    assert_eq!(p(ok(json!(["She"]))), Ok(Some("she".into())));
    assert_eq!(p(ok(json!(["any", "it", "he", "she"]))), Ok(Some("any/it/he".into())));
    // somebody else's answer, nothing set, junk shapes = they set none
    for v in [
      json!({"pronouns": {}, "degraded": false}),
      json!({"pronouns": {"7": ["he"]}, "degraded": false}),
      ok(json!([])),
      ok(json!("he")),
      ok(json!(null)),
      ok(json!({"en": ["he"]})),
    ] {
      assert_eq!(p(v.clone()), Ok(None), "{v}");
    }
    // hostile words are dropped, the rest stay
    assert_eq!(
      p(ok(json!(["he\u{202e}", "x".repeat(13), 5, "they", "<b>"]))),
      Ok(None),
      "only the first three sets are read, none of them clean"
    );
    assert_eq!(p(ok(json!(["she", "a b", "they"]))), Ok(Some("she/they".into())));
  }

  #[test]
  fn a_degraded_or_malformed_answer_is_no_answer_not_no_pronouns() {
    let p = |v: Value| parse(&v, 42);
    // the proxy could not look: even a body that names the id is not trusted
    assert_eq!(p(json!({"pronouns": {}, "degraded": true})), Err(()));
    assert_eq!(p(json!({"pronouns": {"42": ["he"]}, "degraded": true})), Err(()));
    for v in [
      json!({"pronouns": {}}),
      json!({"pronouns": {}, "degraded": "no"}),
      json!({"pronouns": [], "degraded": false}),
      json!({"degraded": false}),
      json!([1]),
      json!(null),
    ] {
      assert_eq!(p(v.clone()), Err(()), "{v}");
    }
    // and an Err is retried after 10min, where an answer would be kept a day
    let mut c = Pronouns::default();
    assert!(c.want(42, 0));
    c.put(42, p(json!({"pronouns": {}, "degraded": true})), 0);
    assert_eq!(c.get(42, 1), None);
    assert!(!c.want(42, FAILED_MS - 1));
    assert!(c.want(42, FAILED_MS));
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
