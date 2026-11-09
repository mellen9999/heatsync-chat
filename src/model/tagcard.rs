// #tags in chat (ext highlightHashtagsInHtml + tag-card.js): a `#word` in a line reads magenta and
// opens its heatsync tag page; resting the pointer on it shows the tag's posts, followers, when it was
// last used and (signed in) whether you follow it, from GET /api/tags/:tag/card. a tag heatsync
// doesn't know shows nothing. answers (unknown tags too) are kept a minute
use crate::i18n::{f, t as t_};
use std::collections::HashMap;
use std::ops::Range;
use std::time::{Duration, Instant};

use serde_json::Value;

const TTL: Duration = Duration::from_secs(60);
// an ask with no answer yet; past this it may go again
const ASKING: Duration = Duration::from_secs(15);
const CAP: usize = 200;
// the most tags marked in one line
const MAX_TAGS: usize = 8;

// `#tag` in `s` from `from` on: a letter then 1-29 letters, digits or `_`, ending at a word's end
// (the ext's /#([a-zA-Z][a-zA-Z0-9_]{1,29})\b/). (range of `#tag`, the tag)
pub fn find(s: &str, from: usize) -> Vec<(Range<usize>, Box<str>)> {
  let b = s.as_bytes();
  let mut out = Vec::new();
  let mut i = from;
  while let Some(k) = s[i..].find('#') {
    let a = i + k;
    let n = b[a + 1..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_').count();
    let end = a + 1 + n;
    let ends = s[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
    if ends && (2..=30).contains(&n) && b[a + 1].is_ascii_alphabetic() {
      out.push((a..end, s[a + 1..end].into()));
      if out.len() == MAX_TAGS {
        break;
      }
    }
    i = end.max(a + 1);
  }
  out
}

pub fn page(tag: &str) -> String {
  format!("https://heatsync.org/feed/tags/{}", tag.to_ascii_lowercase())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
  pub name: Box<str>,
  // label: value, like the user card
  pub rows: Vec<(&'static str, String)>,
}

// the server's answer: None when it names no tag
pub fn card(v: &Value, now_ms: i64) -> Option<Card> {
  let name: String = v.get("name").and_then(Value::as_str)?.chars().filter(|c| !c.is_control()).take(30).collect();
  if name.is_empty() {
    return None;
  }
  let n = |k: &str| v.get(k).and_then(Value::as_f64).filter(|f| f.is_finite() && *f > 0.0).map_or(0, |f| f as u64);
  let mut rows = vec![
    (t_("chat2.tagcard.posts"), compact(n("post_count"))),
    (t_("chat2.tagcard.followers"), compact(n("follower_count"))),
  ];
  if let Some(t) = v.get("last_used_at").and_then(Value::as_str).and_then(crate::util::time::parse_iso) {
    let when = super::card::ago(now_ms, t);
    let shown = if when == t_("chat1.ago.now") { when } else { f("chat2.tagcard.ago", &[("when", &when)]) };
    rows.push((t_("chat2.tagcard.last"), shown));
  }
  // only a signed-in ask carries it
  if let Some(fol) = v.get("following").and_then(Value::as_bool) {
    rows.push((
      t_("chat2.tagcard.you"),
      t_(if fol { "chat2.tagcard.following" } else { "chat2.tagcard.not_following" }).into(),
    ));
  }
  Some(Card { name: name.into(), rows })
}

fn compact(n: u64) -> String {
  match n {
    n if n >= 1_000_000 => format!("{:.1}m", n as f64 / 1e6).replace(".0m", "m"),
    n if n >= 1000 => format!("{:.1}k", n as f64 / 1e3).replace(".0k", "k"),
    n => n.to_string(),
  }
}

enum Load {
  Asking,
  // None = heatsync doesn't know the tag
  Known(Option<Card>),
}

#[derive(Default)]
pub struct Cards {
  by: HashMap<Box<str>, (Instant, Load)>,
}

impl Cards {
  fn fresh(&self, tag: &str) -> Option<&Load> {
    let (at, l) = self.by.get(tag)?;
    let ttl = if matches!(l, Load::Asking) { ASKING } else { TTL };
    (at.elapsed() < ttl).then_some(l)
  }

  pub fn get(&self, tag: &str) -> Option<&Card> {
    match self.fresh(&tag.to_ascii_lowercase())? {
      Load::Known(c) => c.as_ref(),
      Load::Asking => None,
    }
  }

  // true = nobody asked lately: the caller asks (and it is marked as asked)
  pub fn want(&mut self, tag: &str) -> bool {
    let k = tag.to_ascii_lowercase();
    if self.fresh(&k).is_some() {
      return false;
    }
    self.put_load(k, Load::Asking);
    true
  }

  // Ok = an answer (None: unknown tag); Err = no answer, asked again on the next rest
  pub fn put(&mut self, tag: &str, res: Result<Option<Card>, ()>) {
    let k = tag.to_ascii_lowercase();
    match res {
      Ok(c) => self.put_load(k, Load::Known(c)),
      Err(()) => {
        self.by.remove(&*k);
      }
    }
  }

  fn put_load(&mut self, k: String, l: Load) {
    if self.by.len() >= CAP && !self.by.contains_key(&*k) {
      let old = self.by.iter().min_by_key(|e| e.1.0).map(|e| e.0.clone());
      if let Some(o) = old {
        self.by.remove(&o);
      }
    }
    self.by.insert(k.into(), (Instant::now(), l));
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn tags(s: &str) -> Vec<String> {
    find(s, 0).into_iter().map(|(r, t)| format!("{}={t}", &s[r])).collect()
  }

  #[test]
  fn tags_read_like_the_ext() {
    assert_eq!(tags("go #forsen and #Rust_2 now"), ["#forsen=forsen", "#Rust_2=Rust_2"]);
    assert_eq!(tags("#ab."), ["#ab=ab"]);
    // too short or long, a digit first
    assert!(tags("#a #1abc &#39 #").is_empty());
    assert!(tags(&format!("#{}", "x".repeat(31))).is_empty());
    assert_eq!(tags(&format!("#{}", "x".repeat(30))).len(), 1);
    assert_eq!(find(&"#tag ".repeat(20), 0).len(), MAX_TAGS);
    // only from `from` on
    let s = "#name: #tag";
    assert_eq!(find(s, 6), vec![(7..11, "tag".into())]);
    assert_eq!(page("Rust"), "https://heatsync.org/feed/tags/rust");
  }

  #[test]
  fn the_card_reads_the_answer() {
    let now = crate::util::time::parse_iso("2026-11-03T12:00:00Z").unwrap();
    let v = json!({"name": "rust", "post_count": 1234, "follower_count": 7, "last_used_at": "2026-11-03T09:00:00Z", "following": true});
    let c = card(&v, now).unwrap();
    assert_eq!(&*c.name, "rust");
    assert_eq!(
      c.rows,
      vec![("posts", "1.2k".into()), ("followers", "7".into()), ("last", "3h ago".into()), ("you", "following".into())]
    );
    // signed out: no `you`; junk numbers read 0
    let v = json!({"name": "rust", "post_count": -5, "follower_count": "x"});
    assert_eq!(card(&v, now).unwrap().rows, vec![("posts", "0".into()), ("followers", "0".into())]);
    assert!(card(&json!({"error": "Tag not found"}), now).is_none());
  }

  #[test]
  fn answers_are_kept_and_unknown_tags_too() {
    let mut c = Cards::default();
    assert!(c.want("Rust"));
    assert!(!c.want("rust"), "asking");
    c.put("rust", Ok(None));
    assert!(!c.want("rust"), "unknown is an answer");
    assert!(c.get("rust").is_none());
    c.put("go", Err(()));
    assert!(c.want("go"), "a failure is asked again");
    for i in 0..CAP + 5 {
      c.put(&format!("t{i}"), Ok(None));
    }
    assert!(c.by.len() <= CAP);
  }
}
