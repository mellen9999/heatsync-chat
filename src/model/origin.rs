// lines you sent through heatsync.org: the site tells your other clients (`chat:origin_broadcast`, ext
// social.js trackSentMessage(text, 'heatsync')) and the platform's echo of it wears [H] instead of
// [T]/[K]. matched by text, only on your own lines; bounded; an echo must also be within seconds of the send
use std::collections::VecDeque;

const WINDOW_MS: i64 = 24 * 3600 * 1000;
// an echo that beat the broadcast here is seconds old (ext retroFoldOwnEchoes, SENT_DEDUP_WINDOW)
pub const RACE_MS: i64 = 10_000;
// how far a line's own time may sit from the broadcast and still be its echo (youtube batches lag)
const ECHO_MS: i64 = 30_000;
const MAX: usize = 64;
// the server's own bound on the text
const MAX_TEXT: usize = 500;

#[derive(Default)]
pub struct Origins(VecDeque<(Box<str>, i64)>);

// the frame's text, as the echo will read (a kick send's `[emote:id:name]` tokens are their names, ext
// _unkickEmotes); None = junk
pub fn text(v: &serde_json::Value) -> Option<Box<str>> {
  let t = v.get("text").and_then(serde_json::Value::as_str)?.trim();
  if t.is_empty() || t.chars().count() > MAX_TEXT {
    return None;
  }
  Some(crate::net::relay::unkick(t).trim().into())
}

impl Origins {
  pub fn note(&mut self, now: i64, text: Box<str>) {
    self.0.retain(|(_, at)| now - at < WINDOW_MS);
    if self.0.len() == MAX {
      self.0.pop_front();
    }
    self.0.push_back((text, now));
  }

  // `ts`: when the line was said. an echo lands seconds after the send; the same words said natively
  // later are not that heatsync send (M1)
  pub fn sent(&self, ts: i64, body: &str) -> bool {
    let body = body.trim();
    !body.is_empty() && self.0.iter().any(|(t, at)| (ts - at).abs() < ECHO_MS && **t == *body)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn a_heatsync_send_is_known_by_its_words_and_moment() {
    let mut o = Origins::default();
    o.note(0, text(&json!({"type": "chat:origin_broadcast", "text": " gg wp ", "origin": "heatsync"})).unwrap());
    assert!(o.sent(1000, "gg wp"));
    assert!(!o.sent(1000, "gg"));
    assert!(!o.sent(WINDOW_MS + 1, "gg wp"), "a day later it is forgotten");
    // M1: the same words typed straight into twitch an hour later are not that heatsync send
    assert!(!o.sent(3_600_000, "gg wp"), "an echo is seconds after the send, not hours");
    assert!(text(&json!({"text": ""})).is_none());
    assert!(text(&json!({"text": "x".repeat(501)})).is_none());
    assert!(text(&json!({"text": 5})).is_none());
    assert_eq!(&*text(&json!({"text": "hi [emote:37225:KEKLEO]"})).unwrap(), "hi KEKLEO", "as kick's echo reads");
    for i in 0..200 {
      o.note(2000 + i, format!("{i}").into());
    }
    assert_eq!(o.0.len(), MAX, "bounded");
  }
}
