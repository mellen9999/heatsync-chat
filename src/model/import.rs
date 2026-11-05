// the twitch channels you follow, for /import (the ext's "import follows", channel-mgmt.js
// renderFollowImportPicker). pure: no ui, no net. every row came from a server, so counts, logins and names
// are bounded and checked here
use serde_json::Value;

use super::ident;

// the most follows kept (the server walks up to 2000; a few hundred is a list a person can pick from)
pub const MAX_FOLLOWS: usize = 500;
// the live check takes this many names a call (the server drops the rest)
pub const BATCH: usize = 20;
const MAX_NAME: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Follow {
  // lowercase, a valid twitch room
  pub login: Box<str>,
  // their own spelling, only when it says more than the login does
  pub name: Box<str>,
  pub live: bool,
}

impl Follow {
  // `login`, or `login (Name)` when the name is more than the login in other letters
  pub fn label(&self) -> String {
    if self.name.is_empty() { self.login.to_string() } else { format!("{} ({})", self.login, self.name) }
  }
}

fn name(raw: &str, login: &str) -> Box<str> {
  let n: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_NAME).collect();
  let n = n.trim();
  if n.is_empty() || n.eq_ignore_ascii_case(login) { "".into() } else { n.into() }
}

// the server's follow list: None = no list in the body (that is no answer, not "follows nobody").
// rows are checked, de-duplicated and capped; the flag says the list was cut (by the server or by us)
pub fn parse(v: &Value) -> Option<(Vec<Follow>, bool)> {
  let list = v.get("channels")?.as_array()?;
  let mut cut = v.get("truncated").and_then(Value::as_bool) == Some(true);
  let mut rows: Vec<Follow> = Vec::new();
  for c in list {
    let Some(login) = c.get("login").and_then(Value::as_str).map(str::to_ascii_lowercase) else { continue };
    if !ident::twitch_login(&login) || rows.iter().any(|r| *r.login == *login) {
      continue;
    }
    if rows.len() >= MAX_FOLLOWS {
      cut = true;
      break;
    }
    let nm = name(c.get("displayName").and_then(Value::as_str).unwrap_or(""), &login);
    rows.push(Follow { login: login.into(), name: nm, live: false });
  }
  Some((rows, cut))
}

// the logins a live-status answer says are live (checked, lowercase)
pub fn live_in(v: &Value) -> Vec<Box<str>> {
  let Some(a) = v.get("live").and_then(Value::as_array) else { return Vec::new() };
  a.iter()
    .filter_map(Value::as_str)
    .map(str::to_ascii_lowercase)
    .filter(|l| ident::twitch_login(l))
    .take(BATCH)
    .map(Into::into)
    .collect()
}

// live ones first, then a to z
pub fn sort(rows: &mut [Follow]) {
  rows.sort_by(|a, b| b.live.cmp(&a.live).then_with(|| a.login.cmp(&b.login)));
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn rows_are_checked_deduped_and_named_only_when_the_name_says_more() {
    let v = json!({"channels": [
      {"login": "Big", "displayName": "Big"},
      {"login": "small", "displayName": "SmAll\u{7}\nx"},
      {"login": "big", "displayName": "dupe"},
      {"login": "bad name", "displayName": "x"},
      {"login": "", "displayName": "x"},
      {"login": 5},
      {"login": "plain"},
    ], "truncated": false});
    let (rows, cut) = parse(&v).unwrap();
    assert!(!cut);
    let got: Vec<_> = rows.iter().map(|r| (r.login.to_string(), r.label())).collect();
    assert_eq!(
      got,
      [("big".into(), "big".into()), ("small".into(), "small (SmAll  x)".into()), ("plain".into(), "plain".into())]
    );
  }

  #[test]
  fn no_list_is_no_answer_and_a_long_one_is_cut() {
    assert!(parse(&json!({})).is_none());
    assert!(parse(&json!({"channels": "x"})).is_none());
    assert_eq!(parse(&json!({"channels": []})).unwrap().0.len(), 0, "following nobody is an answer");
    let many: Vec<_> = (0..MAX_FOLLOWS + 50).map(|i| json!({"login": format!("u{i}")})).collect();
    let (rows, cut) = parse(&json!({"channels": many})).unwrap();
    assert_eq!((rows.len(), cut), (MAX_FOLLOWS, true));
    assert!(parse(&json!({"channels": [], "truncated": true})).unwrap().1, "the server's own cut counts");
    let long = "x".repeat(500);
    let (rows, _) = parse(&json!({"channels": [{"login": "a", "displayName": long}]})).unwrap();
    assert_eq!(rows[0].name.chars().count(), MAX_NAME);
  }

  #[test]
  fn live_names_are_checked_and_live_sorts_first_then_alphabetical() {
    assert_eq!(live_in(&json!({"live": ["B", "bad name", 3, "a"]})), ["b".into(), "a".into()] as [Box<str>; 2]);
    assert!(live_in(&json!({"live": "x"})).is_empty() && live_in(&json!({})).is_empty());
    let f = |l: &str, live| Follow { login: l.into(), name: "".into(), live };
    let mut rows = vec![f("zed", false), f("amy", false), f("mia", true), f("bob", true)];
    sort(&mut rows);
    let order: Vec<_> = rows.iter().map(|r| &*r.login).collect();
    assert_eq!(order, ["bob", "mia", "amy", "zed"]);
  }
}
