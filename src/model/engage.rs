// feed engagement (ext feed-engage.js + input.js): your bookmarks, the ten-minute edit window, the
// text an `op to feed` starts, and the server's answers read as hostile json. pure, so every rule is tested
use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::Value;

use super::post;

// posts a bookmark answer is kept for (ext FEED_BOOKMARKS_MAX); the oldest goes first
const CAP: usize = 2000;
// ids per check (the route's cap)
pub const CHECK_CHUNK: usize = 100;
// ext EDIT_WINDOW_MS: the server enforces it too
pub const EDIT_WINDOW_MS: i64 = 10 * 60 * 1000;
// a post's length cap (the ext box and the server)
pub const MAX_POST: usize = 500;
// what an edit may be, at most, once cleaned
const MAX_EDIT: usize = 4000;

#[derive(Default)]
pub struct Marks {
  on: HashMap<Box<str>, bool>,
  order: VecDeque<Box<str>>,
  // ids a check is out for, so one load asks once
  asked: HashSet<Box<str>>,
  // one request per post at a time: two fast presses would toggle back
  busy: HashSet<Box<str>>,
}

impl Marks {
  pub fn get(&self, id: &str) -> bool {
    self.on.get(id).copied().unwrap_or(false)
  }

  fn set(&mut self, id: &str, on: bool) {
    if self.on.insert(id.into(), on).is_none() {
      self.order.push_back(id.into());
    }
    while self.order.len() > CAP {
      if let Some(old) = self.order.pop_front() {
        self.on.remove(&old);
      }
    }
  }

  // the ids of these not known yet, once each, marked as asked. empty once they are all known or asked
  pub fn ask_about<'a>(&mut self, ids: impl Iterator<Item = &'a str>) -> Vec<Box<str>> {
    let mut out = Vec::new();
    for id in ids {
      if !self.on.contains_key(id) && self.asked.insert(id.into()) {
        out.push(id.into());
      }
    }
    out
  }

  // a check came back. None = it failed: forget that it was asked, so the next load asks again
  pub fn checked(&mut self, ids: &[Box<str>], res: Option<Vec<(Box<str>, bool)>>) {
    for id in ids {
      self.asked.remove(id);
    }
    for (id, on) in res.into_iter().flatten() {
      self.set(&id, on);
    }
  }

  // press: flip it now. Some(wanted) = send that; None = one is already out for this post
  pub fn begin(&mut self, id: &str) -> Option<bool> {
    if !self.busy.insert(id.into()) {
      return None;
    }
    let on = !self.get(id);
    self.set(id, on);
    Some(on)
  }

  // the server answered a `begin`. a refusal puts the old state back
  pub fn end(&mut self, id: &str, wanted: bool, ok: bool) {
    self.busy.remove(id);
    if !ok {
      self.set(id, !wanted);
    }
  }
}

// POST /api/bookmarks/check: { bookmarked: { id: bool } }. one answer per id asked (true only when the
// server said exactly true); anything else in the body is ignored. None = not that shape
pub fn parse_checked(v: &Value, asked: &[Box<str>]) -> Option<Vec<(Box<str>, bool)>> {
  let map = v.get("bookmarked").and_then(Value::as_object)?;
  Some(asked.iter().map(|id| (id.clone(), map.get(&**id) == Some(&Value::Bool(true)))).collect())
}

// PATCH /api/messages/:id: { success: true, message: { content } } -> the saved words, as a post shows them
pub fn parse_edit(v: &Value) -> Option<Box<str>> {
  if v.get("success").and_then(Value::as_bool) != Some(true) {
    return None;
  }
  let text = post::unescape(v.get("message")?.get("content")?.as_str()?);
  let text: String = text.chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).take(MAX_EDIT).collect();
  let text = text.trim();
  (!text.is_empty()).then(|| text.into())
}

// ms left to edit a post made at `ts`; None = the window is shut
pub fn edit_left(ts: i64, now: i64) -> Option<i64> {
  let left = EDIT_WINDOW_MS - (now - ts);
  (left > 0 && left <= EDIT_WINDOW_MS).then_some(left)
}

// "9:41"
pub fn clock(ms: i64) -> String {
  let s = ms.max(0) / 1000;
  format!("{}:{:02}", s / 60, s % 60)
}

// `op to feed` on a chat line: the box gets `/op @name: words`, cut to the post cap, so the post is
// read and sent by hand (ext _quickOpToFeed posts at once; here nothing public goes out unseen)
pub fn op_line(nick: &str, body: &str) -> Option<String> {
  let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
  if body.is_empty() {
    return None;
  }
  let words: String = format!("@{nick}: {body}").chars().take(MAX_POST).collect();
  Some(format!("/op {words}"))
}

pub fn too_long(text: &str) -> bool {
  text.chars().count() > MAX_POST
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn ids(a: &[&str]) -> Vec<Box<str>> {
    a.iter().map(|s| (*s).into()).collect()
  }

  #[test]
  fn a_check_asks_for_unknown_ids_once_and_a_failure_asks_again() {
    let mut m = Marks::default();
    let first = m.ask_about(["a", "b", "a"].into_iter());
    assert_eq!(first, ids(&["a", "b"]));
    assert!(m.ask_about(["a", "b"].into_iter()).is_empty(), "already asked");
    m.checked(&first, None);
    assert_eq!(m.ask_about(["a"].into_iter()), ids(&["a"]), "a failed check is asked again");
    m.checked(&ids(&["a"]), Some(vec![("a".into(), true)]));
    assert!(m.get("a") && !m.get("b"));
    assert!(m.ask_about(["a"].into_iter()).is_empty(), "known now");
  }

  #[test]
  fn a_press_flips_now_and_a_refusal_puts_it_back() {
    let mut m = Marks::default();
    assert_eq!(m.begin("a"), Some(true));
    assert!(m.get("a"));
    assert_eq!(m.begin("a"), None, "one request per post");
    m.end("a", true, false);
    assert!(!m.get("a"), "rolled back");
    assert_eq!(m.begin("a"), Some(true));
    m.end("a", true, true);
    assert!(m.get("a"));
    assert_eq!(m.begin("a"), Some(false), "now it removes");
    m.end("a", false, false);
    assert!(m.get("a"), "a refused removal keeps it");
  }

  #[test]
  fn the_oldest_answers_go_first() {
    let mut m = Marks::default();
    for i in 0..CAP + 5 {
      m.set(&i.to_string(), true);
    }
    assert!(!m.get("0") && m.get(&(CAP + 4).to_string()));
    assert_eq!(m.order.len(), CAP);
  }

  #[test]
  fn the_check_answer_is_read_as_hostile() {
    let asked = ids(&["a", "b", "c"]);
    let v = json!({"bookmarked": {"a": true, "b": "true", "c": 1, "zzz": true}});
    let got = parse_checked(&v, &asked).unwrap();
    assert_eq!(got, vec![("a".into(), true), ("b".into(), false), ("c".into(), false)]);
    assert!(parse_checked(&json!({"bookmarked": []}), &asked).is_none());
    assert!(parse_checked(&json!({"bookmarked": null}), &asked).is_none());
    assert!(parse_checked(&Value::Null, &asked).is_none());
  }

  #[test]
  fn an_edit_answer_needs_success_and_words() {
    let ok = json!({"success": true, "message": {"content": "a &amp; b\u{7}  "}});
    assert_eq!(parse_edit(&ok).as_deref(), Some("a & b"));
    assert!(parse_edit(&json!({"success": false, "message": {"content": "x"}})).is_none());
    assert!(parse_edit(&json!({"success": true, "message": {"content": "  "}})).is_none());
    assert!(parse_edit(&json!({"success": true, "message": {"content": 7}})).is_none());
    assert!(parse_edit(&json!({"success": true})).is_none());
    let long = json!({"success": true, "message": {"content": "x".repeat(MAX_EDIT * 2)}});
    assert_eq!(parse_edit(&long).unwrap().chars().count(), MAX_EDIT);
  }

  #[test]
  fn the_edit_window_is_ten_minutes() {
    assert_eq!(edit_left(1000, 1000), Some(EDIT_WINDOW_MS));
    assert_eq!(edit_left(0, EDIT_WINDOW_MS - 1), Some(1));
    assert_eq!(edit_left(0, EDIT_WINDOW_MS), None);
    assert_eq!(edit_left(10_000, 0), None, "a post from the future is not editable");
    assert_eq!(clock(412_000), "6:52");
    assert_eq!(clock(59_999), "0:59");
    assert_eq!(clock(-5), "0:00");
  }

  #[test]
  fn op_to_feed_quotes_the_author_and_stays_in_the_cap() {
    assert_eq!(op_line("bob", "  hi   there ").as_deref(), Some("/op @bob: hi there"));
    assert!(op_line("bob", "  ").is_none());
    let long = op_line("bob", &"x".repeat(900)).unwrap();
    assert_eq!(long.strip_prefix("/op ").unwrap().chars().count(), MAX_POST);
    assert!(!too_long(long.strip_prefix("/op ").unwrap()));
    assert!(too_long(&"x".repeat(MAX_POST + 1)));
  }
}
