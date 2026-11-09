// what you sent from the box between runs (ext 66af7102): up / down walk back through it after a restart too.
// kept on this device only in the `sent` file, one json string per line, the newest 50, never synced. like every
// input the file is read with caps: a size before reading, a line count, each line bounded and free of control
// bytes. `remember_sent` off keeps it in memory only and removes the file
use serde_json::Value;

use super::line::MAX_CHARS;

pub const FILE: &str = "sent";
pub const KEEP: usize = 50;
// a file bigger than this is not ours (50 lines of 500 characters come to a fraction of it)
pub const MAX_BYTES: u64 = 128 * 1024;

// a line as the history keeps it: no control bytes, no more than a chat line holds. None = nothing worth keeping
pub fn clean(s: &str) -> Option<String> {
  let t: String = crate::util::strip(s).take(MAX_CHARS).collect();
  let t = t.trim();
  (!t.is_empty()).then(|| t.to_owned())
}

// the file for `lines` (oldest first): the newest KEEP
pub fn encode<'a>(lines: impl DoubleEndedIterator<Item = &'a str>) -> String {
  let mut kept: Vec<String> = lines.rev().take(KEEP).filter_map(clean).map(|l| Value::String(l).to_string()).collect();
  kept.reverse();
  kept.iter().fold(String::new(), |mut out, l| {
    out.push_str(l);
    out.push('\n');
    out
  })
}

// the lines in a file, oldest first. what is not a clean json string is skipped, a repeat of the line before is not
// kept twice, and only the newest KEEP stay
pub fn decode(text: &str) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  for l in text.lines().take(KEEP * 4) {
    let Some(s) = serde_json::from_str::<Value>(l).ok().and_then(|v| v.as_str().and_then(clean)) else { continue };
    if out.last() != Some(&s) {
      out.push(s);
    }
  }
  let extra = out.len().saturating_sub(KEEP);
  out.drain(..extra);
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn what_goes_in_comes_back_in_order_with_its_quotes_and_unicode() {
    let v = ["gg", "he said \"hi\"\\o/", "日本語 🎉", "a b"];
    assert_eq!(decode(&encode(v.iter().copied())), v);
  }

  #[test]
  fn only_the_newest_fifty_stay_and_long_lines_are_cut() {
    let many: Vec<String> = (0..80).map(|i| format!("line {i}")).collect();
    let back = decode(&encode(many.iter().map(String::as_str)));
    assert_eq!((back.len(), back[0].as_str(), back[49].as_str()), (50, "line 30", "line 79"));
    let long = "x".repeat(900);
    assert_eq!(decode(&encode([long.as_str()].into_iter()))[0].chars().count(), MAX_CHARS);
  }

  #[test]
  fn junk_is_skipped_and_repeats_collapse() {
    let file = "not json\n42\n[\"x\"]\n\"\"\n\"   \"\n\"ok\"\n\"ok\"\n\"a\\u0007b\"\n\"ok\"\n";
    assert_eq!(decode(file), ["ok", "ab", "ok"]);
    // a flood of lines reads only so many
    assert!(decode(&"\"x\"\n".repeat(1_000_000)).len() <= KEEP);
  }
}
