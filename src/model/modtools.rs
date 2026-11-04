// the mod tools screen's data (ext mod-suite.js): shield mode, pending unban requests, automod levels and
// who is in chat, read from heatsync's /api/mod/* answers. every field is bounded; junk rows are dropped
use super::ident;
use serde_json::Value;

pub const MAX_UNBANS: usize = 500;
pub const MAX_CHATTERS: usize = 10_000;
pub const MAX_LEVEL: u8 = 4;

// helix's names, and how they read: overall first, then the eight categories
pub const LEVELS: [(&str, &str); 9] = [
  ("overall_level", "overall"),
  ("disability", "disability"),
  ("aggression", "aggression"),
  ("sexuality_sex_or_gender", "sexuality, sex or gender"),
  ("misogyny", "misogyny"),
  ("bullying", "bullying"),
  ("swearing", "swearing"),
  ("race_ethnicity_or_religion", "race, ethnicity or religion"),
  ("sex_based_terms", "sex-based terms"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unban {
  pub id: Box<str>,
  pub login: Box<str>,
  pub name: Box<str>,
  pub text: Box<str>,
  pub at: i64,
}

fn text(v: &Value, k: &str, max: usize) -> Box<str> {
  let s = v.get(k).and_then(Value::as_str).unwrap_or("");
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().into()
}

pub fn unbans(v: &Value) -> Vec<Unban> {
  let rows = v.get("requests").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
  rows
    .iter()
    .take(MAX_UNBANS)
    .filter_map(|r| {
      let id = text(r, "id", 128);
      let login = text(r, "user_login", 25).to_ascii_lowercase();
      (ident::slug(&id, 128) && ident::twitch_login(&login)).then(|| Unban {
        name: Some(text(r, "user_name", 40)).filter(|n| !n.is_empty()).unwrap_or_else(|| login.as_str().into()),
        login: login.into(),
        text: text(r, "text", 500),
        at: r.get("created_at").and_then(Value::as_str).and_then(crate::util::time::parse_iso).unwrap_or(0),
        id,
      })
    })
    .collect()
}

// each of LEVELS: 0-4, None = twitch has none set
pub type Levels = [Option<u8>; 9];

pub fn levels(v: &Value) -> Levels {
  let mut out = [None; 9];
  for (slot, (k, _)) in out.iter_mut().zip(LEVELS) {
    *slot = v.get(k).and_then(Value::as_u64).filter(|n| *n <= u64::from(MAX_LEVEL)).map(|n| n as u8);
  }
  out
}

// (how many twitch counts, the logins it listed, sorted)
pub fn chatters(v: &Value) -> (u64, Vec<Box<str>>) {
  let rows = v.get("chatters").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
  let mut out: Vec<Box<str>> = rows
    .iter()
    .take(MAX_CHATTERS)
    .map(|c| text(c, "user_login", 25).to_ascii_lowercase())
    .filter(|l| ident::twitch_login(l))
    .map(Into::into)
    .collect();
  out.sort_unstable();
  out.dedup();
  let total = v.get("total").and_then(Value::as_u64).unwrap_or(0).max(out.len() as u64);
  (total, out)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn reads_the_servers_answers() {
    let v = json!({"requests": [
      {"id": "abc-1", "user_login": "Bob", "user_name": "Bob", "text": "sorry\nplease", "created_at": "2026-11-03T10:00:00Z"},
      {"id": "bad id!", "user_login": "amy"},
      {"id": "x", "user_login": "a b"},
    ]});
    let u = unbans(&v);
    assert_eq!(u.len(), 1);
    assert_eq!((&*u[0].login, &*u[0].text), ("bob", "sorry please"));
    assert!(u[0].at > 0);
    let l = levels(&json!({"overall_level": 2, "swearing": 4, "bullying": 9, "misogyny": null}));
    assert_eq!((l[0], l[6], l[5], l[4]), (Some(2), Some(4), None, None));
    let (n, c) =
      chatters(&json!({"total": 2, "chatters": [{"user_login": "zed"}, {"user_login": "Amy"}, {"user_login": "?"}]}));
    assert_eq!((n, c), (2, vec!["amy".into(), "zed".into()]));
    assert!(unbans(&json!("junk")).is_empty());
  }
}
