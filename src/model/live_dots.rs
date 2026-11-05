// which open channels are live, for the red dot on their tab (ext data-live): heatsync's live-status
// answer for twitch and kick, a go-live or sign-off the moment the socket says so, and a youtube feed
// while its relay is connected. pure: no ui, no net
use std::collections::HashSet;

use serde_json::Value;

use super::ident;
use super::message::Platform;

// the server reads this many names of each platform a call and drops the rest
pub const BATCH: usize = 20;

// channel keys, lowercase
pub type Names = Vec<Box<str>>;

#[derive(Default)]
pub struct Live(HashSet<(Platform, Box<str>)>);

impl Live {
  pub fn has(&self, p: Platform, key: &str) -> bool {
    self.0.contains(&(p, key.to_ascii_lowercase().into()))
  }

  // true when it changed
  pub fn set(&mut self, p: Platform, key: &str, on: bool) -> bool {
    let k = (p, key.to_ascii_lowercase().into());
    if on { self.0.insert(k) } else { self.0.remove(&k) }
  }

  // one answer: of the channels asked, the ones it names are live and the rest are not
  pub fn answer(&mut self, p: Platform, asked: &[Box<str>], live: &[Box<str>]) -> bool {
    let mut changed = false;
    for a in asked {
      changed |= self.set(p, a, live.iter().any(|l| l.eq_ignore_ascii_case(a)));
    }
    changed
  }

  // only channels still open keep a mark
  pub fn keep(&mut self, open: impl Fn(Platform, &str) -> bool) {
    self.0.retain(|(p, k)| open(*p, k));
  }
}

// what the server would read: twitch logins, kick slugs (letters, digits, `_`, `-`, up to 25)
pub fn askable(p: Platform, key: &str) -> bool {
  match p {
    Platform::Twitch => ident::twitch_login(key),
    Platform::Kick => {
      (1..=25).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    }
    Platform::Youtube => false,
  }
}

// `{live: [...], kickLive: [...]}`: each list the server named, checked and bounded. a list it left out is
// None (keep what we had), not "nobody is live"
pub fn parse(v: &Value) -> (Option<Names>, Option<Names>) {
  let list = |k: &str, p: Platform| {
    v.get(k).and_then(Value::as_array).map(|a| {
      a.iter()
        .filter_map(Value::as_str)
        .map(str::to_ascii_lowercase)
        .filter(|s| askable(p, s))
        .take(BATCH)
        .map(Into::into)
        .collect()
    })
  };
  (list("live", Platform::Twitch), list("kickLive", Platform::Kick))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn an_answer_marks_the_asked_and_leaves_the_rest() {
    let mut l = Live::default();
    l.set(Platform::Kick, "adin", true);
    let asked: Vec<Box<str>> = vec!["xqc".into(), "forsen".into()];
    assert!(l.answer(Platform::Twitch, &asked, &["XQC".into()]));
    assert!(l.has(Platform::Twitch, "xqc") && !l.has(Platform::Twitch, "forsen"));
    assert!(l.has(Platform::Kick, "adin"), "another platform's mark stays");
    assert!(!l.answer(Platform::Twitch, &asked, &["xqc".into()]), "same answer, no change");
    l.keep(|p, k| p == Platform::Twitch && k == "xqc");
    assert!(!l.has(Platform::Kick, "adin"));
  }

  #[test]
  fn a_hostile_answer_is_bounded_and_checked() {
    let many: Vec<String> = (0..100).map(|i| format!("c{i}")).collect();
    let v = json!({"live": many, "kickLive": ["ok-slug", "bad slug", "<b>", 7]});
    let (t, k) = parse(&v);
    assert_eq!(t.unwrap().len(), BATCH);
    assert_eq!(k.unwrap(), vec![Box::<str>::from("ok-slug")]);
    assert_eq!(parse(&json!({"error": "rate limited"})), (None, None), "no lists: keep the dots");
  }
}
