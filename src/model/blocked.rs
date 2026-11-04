// a twitch room's blocked terms, for its moderators (ext blocked-terms.js): the list lives on twitch,
// heatsync's server reads and changes it (/api/mod/blocked-terms). a term is attacker-shaped text:
// bounded, control chars made spaces, its id checked before it rides back in a request
use super::ident;
use serde_json::Value;

pub const MIN: usize = 2;
pub const MAX: usize = 500;
// the server pages up to this many
pub const MAX_TERMS: usize = 10_000;
const MAX_ID: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
  pub id: Box<str>,
  pub text: Box<str>,
}

pub fn valid_id(s: &str) -> bool {
  ident::slug(s, MAX_ID)
}

pub fn term(v: &Value) -> Option<Term> {
  let id = v.get("id").and_then(Value::as_str).filter(|s| valid_id(s))?;
  let text: String =
    v.get("text").and_then(Value::as_str)?.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX).collect();
  let text = text.trim();
  (!text.is_empty()).then(|| Term { id: id.into(), text: text.into() })
}

// `{terms: [...], truncated}`, newest first as the server sends
pub fn list(v: &Value) -> (Vec<Term>, bool) {
  let terms = v.get("terms").and_then(Value::as_array).into_iter().flatten().take(MAX_TERMS).filter_map(term).collect();
  (terms, v.get("truncated").and_then(Value::as_bool) == Some(true))
}

// what can be added: 2 to 500 chars once trimmed
pub fn addable(text: &str) -> Option<&str> {
  let t = text.trim();
  (MIN..=MAX).contains(&t.chars().count()).then_some(t)
}

// the terms shown under a filter: a case-insensitive substring, order kept
pub fn visible<'a>(terms: &'a [Term], q: &str) -> Vec<&'a Term> {
  let q = q.trim().to_lowercase();
  terms.iter().filter(|t| q.is_empty() || t.text.to_lowercase().contains(&q)).collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn terms_are_read_bounded_and_checked() {
    let v = json!({"terms": [
      {"id": "a-1", "text": "badword", "created_at": "x", "expires_at": null},
      {"id": "bad id", "text": "x"},
      {"id": "b_2", "text": "two\nlines"},
      {"id": "c3", "text": "   "},
      {"id": "d4", "text": "y".repeat(900)},
      {"id": "e5"},
    ], "truncated": true});
    let (t, more) = list(&v);
    assert!(more);
    assert_eq!(t.iter().map(|t| &*t.id).collect::<Vec<_>>(), ["a-1", "b_2", "d4"]);
    assert_eq!(&*t[1].text, "two lines");
    assert_eq!(t[2].text.chars().count(), MAX);
    assert_eq!(list(&json!(null)), (vec![], false));
  }

  #[test]
  fn adding_and_filtering() {
    assert_eq!(addable("  ab "), Some("ab"));
    assert_eq!(addable("a"), None);
    assert_eq!(addable(&"z".repeat(501)), None);
    let t = |id: &str, s: &str| Term { id: id.into(), text: s.into() };
    let all = [t("1", "Spam Link"), t("2", "other"), t("3", "spammer")];
    assert_eq!(visible(&all, " SPAM ").iter().map(|t| &*t.id).collect::<Vec<_>>(), ["1", "3"]);
    assert_eq!(visible(&all, "").len(), 3);
  }
}
