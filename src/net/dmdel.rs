// take back one of your own heatsync dms. the site's route is per conversation (DELETE
// /api/dm/threads/:thread/messages/:id), and a dm row only knows who it was with, so the conversation is
// looked up first: GET /api/dm/threads, the one-to-one one that holds both of you. heatsync then tells every
// member's socket (`dm:message-deleted`), so the line leaves the other side's window too
use serde_json::Value;

use super::social::call;
use super::{NetEvent, ORIGIN, Out};
use crate::model::whisper;

// threads and members read from one answer: more than anyone has, fewer than a hostile reply could send
const MAX_THREADS: usize = 5000;

// why a delete did not happen
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
  Auth,
  // heatsync won't let this account delete it
  Refused,
  // it is already gone
  Gone,
  // the account has no one-to-one conversation with them
  NoThread,
  Limited,
  // no answer, or one we can't use
  Other,
}

// the one-to-one conversation with `other` in a GET /api/dm/threads answer: not a group, and no one in it but
// the two of you. its id (digits), or None
pub fn thread_with(v: &Value, other: &str) -> Option<Box<str>> {
  v.get("threads")?.as_array()?.iter().take(MAX_THREADS).find_map(|t| {
    if t.get("is_group").and_then(Value::as_bool) != Some(false) {
      return None;
    }
    let members = t.get("members")?.as_array().filter(|m| m.len() <= 2)?;
    let with = members.iter().any(|m| whisper::user_id(m.get("user_id")).is_some_and(|id| *id == *other));
    if with { whisper::digits(t.get("id")) } else { None }
  })
}

fn fail_of(status: u16) -> Fail {
  match status {
    401 => Fail::Auth,
    403 => Fail::Refused,
    404 | 410 => Fail::Gone,
    429 => Fail::Limited,
    _ => Fail::Other,
  }
}

async fn delete(http: &reqwest::Client, token: &str, with: &str, id: &str) -> Result<(), Fail> {
  let (status, list) = call(http.get(format!("{ORIGIN}/api/dm/threads")).bearer_auth(token)).await;
  if status != 200 {
    return Err(fail_of(status));
  }
  let Some(thread) = thread_with(&list, with) else { return Err(Fail::NoThread) };
  let url =
    format!("{ORIGIN}/api/dm/threads/{}/messages/{}", crate::util::path_seg(&thread), crate::util::path_seg(id));
  let (status, v) = call(http.delete(url).bearer_auth(token)).await;
  match status {
    200..=299 if v.get("error").is_none() => Ok(()),
    s => Err(fail_of(s)),
  }
}

// `with` is their heatsync id, `id` the dm's (digits). the answer carries the id back so the app finds the row
pub async fn run(http: reqwest::Client, token: Option<String>, with: Box<str>, id: Box<str>, out: Out) {
  let res = match token {
    Some(t) => delete(&http, &t, &with, &id).await,
    None => Err(Fail::Auth),
  };
  out.send(NetEvent::DmDeleted { id, res });
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn threads() -> Value {
    json!({ "threads": [
      { "id": 3, "is_group": true, "members": [{ "user_id": "9" }, { "user_id": "7" }, { "user_id": "5" }] },
      { "id": "12", "is_group": false, "members": [{ "user_id": 9 }, { "user_id": "5" }] },
      { "id": 14, "is_group": false, "members": [{ "user_id": "9" }, { "user_id": "7" }] },
    ]})
  }

  #[test]
  fn the_one_to_one_thread_is_found_by_who_is_in_it() {
    assert_eq!(thread_with(&threads(), "7").as_deref(), Some("14"), "not the group they are also in");
    assert_eq!(thread_with(&threads(), "5").as_deref(), Some("12"), "ids as strings or numbers");
    assert_eq!(thread_with(&threads(), "9").as_deref(), Some("12"), "first of two that hold you both");
    assert!(thread_with(&threads(), "8").is_none());
  }

  #[test]
  fn a_group_or_a_hostile_answer_is_never_a_thread() {
    let only_group = json!({ "threads": [{ "id": 3, "is_group": true, "members": [{ "user_id": "7" }] }] });
    assert!(thread_with(&only_group, "7").is_none());
    for bad in [
      json!(null),
      json!({}),
      json!({ "threads": "x" }),
      json!({ "threads": [null, 1, "a"] }),
      // no is_group: not known to be one-to-one
      json!({ "threads": [{ "id": 3, "members": [{ "user_id": "7" }] }] }),
      // a thread id that is not digits would put something else in the path
      json!({ "threads": [{ "id": "../x", "is_group": false, "members": [{ "user_id": "7" }] }] }),
      json!({ "threads": [{ "id": 3, "is_group": false, "members": "7" }] }),
      // more than two in a one-to-one: not one
      json!({ "threads": [{ "id": 3, "is_group": false, "members": [{"user_id":"7"},{"user_id":"8"},{"user_id":"9"}] }] }),
    ] {
      assert!(thread_with(&bad, "7").is_none(), "{bad}");
    }
  }

  #[test]
  fn each_answer_reads_as_one_failure() {
    assert_eq!(fail_of(401), Fail::Auth);
    assert_eq!(fail_of(403), Fail::Refused);
    assert_eq!(fail_of(404), Fail::Gone);
    assert_eq!(fail_of(429), Fail::Limited);
    assert_eq!(fail_of(0), Fail::Other);
    assert_eq!(fail_of(500), Fail::Other);
  }
}
