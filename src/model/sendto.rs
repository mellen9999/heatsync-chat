// which of a merged tab's feeds a line goes to (the ext's send-targets.js): every feed the tab has, unless the
// tab says otherwise. the choice lives in the tab's synced entry as `sendTargets {twitch?, kick?, youtube?}`,
// a missing key meaning on, so both clients read the same thing
use serde_json::{Map, Value};

use super::message::Platform;

const ALL: [Platform; 3] = [Platform::Twitch, Platform::Kick, Platform::Youtube];
const KEYS: [&str; 3] = ["twitch", "kick", "youtube"];

fn slot(p: Platform) -> usize {
  ALL.iter().position(|a| *a == p).unwrap_or(0)
}

// what a tab says about each platform: Some(false) = off, Some(true) = on, None = never touched (on)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Targets([Option<bool>; 3]);

impl Targets {
  // the entry's `sendTargets`; anything that isn't an object of booleans is read as untouched
  pub fn from_value(v: Option<&Value>) -> Self {
    let mut t = Targets::default();
    if let Some(o) = v.and_then(Value::as_object) {
      for (i, k) in KEYS.iter().enumerate() {
        t.0[i] = o.get(*k).and_then(Value::as_bool);
      }
    }
    t
  }

  // written onto the entry's `sendTargets`: only the three platform keys change, any other key stays. an
  // untouched tab writes nothing
  pub fn write(&self, entry: &mut Map<String, Value>) {
    if self.0.iter().all(Option::is_none) {
      return;
    }
    let mut o = entry.get("sendTargets").and_then(Value::as_object).cloned().unwrap_or_default();
    for (i, k) in KEYS.iter().enumerate() {
      if let Some(b) = self.0[i] {
        o.insert((*k).into(), Value::Bool(b));
      }
    }
    entry.insert("sendTargets".into(), Value::Object(o));
  }

  // the feeds a send reaches (resolveSendTargets). a config that turns every linked one off sends to all of
  // them rather than nowhere
  pub fn resolve(&self, linked: &[Platform]) -> Vec<Platform> {
    let on: Vec<Platform> = linked.iter().copied().filter(|p| self.0[slot(*p)] != Some(false)).collect();
    if on.is_empty() { linked.to_vec() } else { on }
  }

  // one platform flipped (nextSendTargets): everything linked is pinned on first, so a feed linked later never
  // goes off by itself. None = that would leave nowhere to send
  pub fn toggled(&self, linked: &[Platform], p: Platform) -> Option<Targets> {
    if !linked.contains(&p) {
      return None;
    }
    let mut t = *self;
    if t.0.iter().all(Option::is_none) {
      for l in linked {
        t.0[slot(*l)] = Some(true);
      }
    }
    let now = self.resolve(linked).contains(&p);
    t.0[slot(p)] = Some(!now);
    let any = linked.iter().any(|l| t.0[slot(*l)] != Some(false));
    any.then_some(t)
  }

  // ctrl-n: all feeds, then each alone in turn, then all again
  pub fn cycled(&self, linked: &[Platform]) -> Targets {
    let on = self.resolve(linked);
    let mut t = *self;
    let next = match on.as_slice() {
      [one] => linked.iter().position(|l| l == one).and_then(|i| linked.get(i + 1)),
      all if all.len() == linked.len() => linked.first(),
      _ => None,
    };
    for l in linked {
      t.0[slot(*l)] = Some(next.is_none_or(|n| n == l));
    }
    t
  }
}

// the most characters the tightest feed in `legs` takes (composerCharLimit); no feed = the common 500
pub fn limit(legs: &[Platform]) -> usize {
  legs.iter().map(|p| p.max_chars()).min().unwrap_or(500)
}

// where a line stands against the cap once the counter shows: close to it, or past it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
  Near,
  Over,
}

// the "n/limit" counter (ext charCountLabel): silent until the line is within a tenth of the cap, then
// the count of what goes on the wire (trimmed, as sent) in characters, not bytes: an é or an emoji is one.
// `Near` up to the cap, `Over` past it
pub fn count(text: &str, limit: usize) -> Option<(String, Fill)> {
  let text = text.trim();
  // bytes only ever overcount characters: a short line is settled without counting
  if limit == 0 || text.len() * 10 <= limit * 9 {
    return None;
  }
  let n = text.chars().count();
  (n * 10 > limit * 9).then(|| (format!("{n}/{limit}"), if n > limit { Fill::Over } else { Fill::Near }))
}

// youtube has no reply threading, so the youtube feed's copy of a reply opens with `@author` (ext ytReplyText):
// left alone when it already does, and when the mention would push it past what youtube takes
pub fn yt_reply_text(text: &str, author: &str) -> String {
  let author = author.trim().trim_start_matches('@');
  if author.is_empty() {
    return text.to_owned();
  }
  let head = format!("@{author}");
  let t = text.trim_start();
  let already = t.get(..head.len()).is_some_and(|h| h.eq_ignore_ascii_case(&head))
    && t[head.len()..].chars().next().is_none_or(char::is_whitespace);
  let with = format!("{head} {text}");
  if already || with.chars().count() > Platform::Youtube.max_chars() { text.to_owned() } else { with }
}

#[cfg(test)]
mod tests {
  use super::*;
  use Platform::*;
  use serde_json::json;

  const TK: [Platform; 2] = [Twitch, Kick];
  const TKY: [Platform; 3] = [Twitch, Kick, Youtube];

  #[test]
  fn the_counter_wakes_within_a_tenth_of_the_cap_and_counts_characters_trimmed() {
    let n = |len: usize, limit| count(&"a".repeat(len), limit);
    assert_eq!(n(450, 500), None, "exactly nine tenths is still quiet");
    assert_eq!(n(451, 500), Some(("451/500".into(), Fill::Near)));
    assert_eq!(n(500, 500), Some(("500/500".into(), Fill::Near)), "at the cap is not over");
    assert_eq!(n(501, 500), Some(("501/500".into(), Fill::Over)));
    assert_eq!(n(181, 200), Some(("181/200".into(), Fill::Near)), "youtube's cap");
    assert_eq!(n(180, 200), None);
    assert_eq!(n(0, 500), None);
    // what is sent is trimmed: the spaces around it are not counted
    assert_eq!(count(&format!("  {}  ", "a".repeat(451)), 500).unwrap().0, "451/500");
    assert_eq!(count(&format!("  {}  ", "a".repeat(400)), 500), None);
    // characters, not bytes: 451 two-byte letters and 451 emoji are each 451
    assert_eq!(count(&"é".repeat(451), 500).unwrap().0, "451/500");
    assert_eq!(count(&"😂".repeat(451), 500).unwrap().0, "451/500");
    assert_eq!(count(&"😂".repeat(300), 500), None, "1200 bytes, 300 characters");
  }

  #[test]
  fn nothing_said_means_every_linked_feed() {
    assert_eq!(Targets::default().resolve(&TKY), TKY);
    assert_eq!(Targets::from_value(Some(&json!({"kick": false}))).resolve(&TKY), [Twitch, Youtube]);
    // a key for a feed the tab doesn't have changes nothing; junk is untouched
    assert_eq!(Targets::from_value(Some(&json!({"youtube": false}))).resolve(&TK), TK);
    assert_eq!(Targets::from_value(Some(&json!("kick"))), Targets::default());
    assert_eq!(Targets::from_value(Some(&json!({"kick": "no"}))), Targets::default());
  }

  #[test]
  fn a_config_that_turns_everything_off_still_sends_somewhere() {
    let t = Targets::from_value(Some(&json!({"twitch": false, "kick": false})));
    assert_eq!(t.resolve(&TK), TK);
  }

  #[test]
  fn toggling_pins_the_rest_on_and_never_leaves_nowhere() {
    let t = Targets::default().toggled(&TKY, Kick).unwrap();
    assert_eq!(t.resolve(&TKY), [Twitch, Youtube]);
    let mut o = Map::new();
    t.write(&mut o);
    assert_eq!(o["sendTargets"], json!({"twitch": true, "kick": false, "youtube": true}));
    let t = t.toggled(&TKY, Twitch).unwrap();
    assert_eq!(t.resolve(&TKY), [Youtube]);
    assert!(t.toggled(&TKY, Youtube).is_none(), "the last one stays on");
    assert!(t.toggled(&TK, Youtube).is_none(), "a feed the tab doesn't have");
    assert_eq!(t.toggled(&TKY, Kick).unwrap().resolve(&TKY), [Kick, Youtube]);
  }

  #[test]
  fn ctrl_n_goes_all_then_each_then_all() {
    let mut t = Targets::default();
    let mut seen = Vec::new();
    for _ in 0..5 {
      t = t.cycled(&TKY);
      seen.push(t.resolve(&TKY));
    }
    assert_eq!(seen, [vec![Twitch], vec![Kick], vec![Youtube], TKY.to_vec(), vec![Twitch]]);
    // a pair someone picked on the ext steps back to all
    let pair = Targets::from_value(Some(&json!({"twitch": true, "kick": true, "youtube": false})));
    assert_eq!(pair.cycled(&TKY).resolve(&TKY), TKY);
    // one feed has nothing to cycle
    assert_eq!(Targets::default().cycled(&[Kick]).resolve(&[Kick]), [Kick]);
  }

  #[test]
  fn written_back_in_the_exts_shape_keeping_other_keys() {
    let mut e = json!({"id": "a", "sendTargets": {"twitch": true, "future": 1}}).as_object().unwrap().clone();
    Targets::from_value(Some(&json!({"twitch": false, "kick": true}))).write(&mut e);
    assert_eq!(e["sendTargets"], json!({"twitch": false, "kick": true, "future": 1}));
    let mut e = json!({"id": "a"}).as_object().unwrap().clone();
    Targets::default().write(&mut e);
    assert!(!e.contains_key("sendTargets"), "untouched writes nothing");
  }

  #[test]
  fn the_tightest_feed_sets_the_limit() {
    assert_eq!(limit(&[Twitch, Kick]), 500);
    assert_eq!(limit(&[Twitch, Youtube]), 200);
    assert_eq!(limit(&[Youtube]), 200);
    assert_eq!(limit(&[]), 500);
  }

  #[test]
  fn a_reply_reaches_youtube_as_an_at_mention() {
    assert_eq!(yt_reply_text("gg", "bob"), "@bob gg");
    assert_eq!(yt_reply_text("gg", "@bob"), "@bob gg", "no doubled @");
    assert_eq!(yt_reply_text("@Bob gg", "bob"), "@Bob gg", "already there");
    assert_eq!(yt_reply_text("@bobby gg", "bob"), "@bob @bobby gg", "a longer name isn't the mention");
    assert_eq!(yt_reply_text("gg", ""), "gg");
    let long = "x".repeat(198);
    assert_eq!(yt_reply_text(&long, "bob"), long, "past the cap the mention is dropped, not the words");
  }
}
