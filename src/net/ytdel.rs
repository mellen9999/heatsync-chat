// `youtube:delete {videoId, messageIds[], authorChannelIds[], channelId}` (ext applyYtDeletion): a mod deleted
// lines, or banned an author and youtube took back everything they said. hostile: bounded counts and lengths,
// no control bytes; the ids are only ever compared to lines we hold, never shown or sent anywhere
use serde_json::Value;

const MAX_IDS: usize = 256;
const MAX_AUTHORS: usize = 50;
const MAX_LEN: usize = 128;

#[derive(Debug, PartialEq, Eq)]
pub struct Gone {
  pub ids: Vec<Box<str>>,
  pub authors: Vec<Box<str>>,
}

fn list(v: Option<&Value>, cap: usize) -> Vec<Box<str>> {
  let ok = |s: &&str| (1..=MAX_LEN).contains(&s.len()) && !s.chars().any(|c| c.is_control() || c.is_whitespace());
  let a = v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  a.iter().filter_map(Value::as_str).filter(ok).take(cap).map(Into::into).collect()
}

// None = nothing to remove
pub fn parse(v: &Value) -> Option<Gone> {
  let (ids, authors) = (list(v.get("messageIds"), MAX_IDS), list(v.get("authorChannelIds"), MAX_AUTHORS));
  (!ids.is_empty() || !authors.is_empty()).then_some(Gone { ids, authors })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn keeps_good_ids_and_drops_hostile_ones() {
    let g =
      parse(&json!({"messageIds": ["a1", "", 7, "bad id", "x\u{0}y", "k".repeat(129)], "authorChannelIds": ["UC1"]}))
        .unwrap();
    assert_eq!((g.ids, g.authors), (vec!["a1".into()], vec!["UC1".into()]));
  }

  #[test]
  fn counts_are_capped_and_junk_is_nothing() {
    let many: Vec<String> = (0..600).map(|i| format!("m{i}")).collect();
    let g = parse(&json!({"messageIds": many, "authorChannelIds": many})).unwrap();
    assert_eq!((g.ids.len(), g.authors.len()), (MAX_IDS, MAX_AUTHORS));
    for bad in [json!({}), json!({"messageIds": "x"}), json!({"messageIds": [], "authorChannelIds": [null]})] {
      assert!(parse(&bad).is_none());
    }
  }
}
