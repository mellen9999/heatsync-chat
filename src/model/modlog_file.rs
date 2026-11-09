// the mod log tab between runs (ext 475e68a9): the bans, timeouts, unbans and deletes it listed, kept on this
// device only in the `modlog` file. a run puts them back as history, oldest first. like every input the file is
// read with caps: a size before reading, a line count, a bounded text, a sane time. the log's rows are
// `<room> · <what>`, so the room (the cap's key) is the words before the first ` · `: a burst in one room can
// never push another room's history out
use serde_json::{Value, json};

use super::message::Platform;

pub const FILE: &str = "modlog";
// rows kept in all (the tab's own cap), and from any one room
pub const KEEP: usize = 300;
pub const PER_ROOM: usize = 100;
// a file bigger than this is not ours
pub const MAX_BYTES: u64 = 256 * 1024;
const MAX_TEXT: usize = 400;

fn tag(p: Platform) -> &'static str {
  match p {
    Platform::Twitch => "t",
    Platform::Kick => "k",
    Platform::Youtube => "y",
  }
}

fn untag(s: &str) -> Option<Platform> {
  Some(match s {
    "t" => Platform::Twitch,
    "k" => Platform::Kick,
    "y" => Platform::Youtube,
    _ => return None,
  })
}

// the room of a row
fn room(p: Platform, text: &str) -> String {
  format!("{}{}", tag(p), text.split_once(" \u{b7} ").map_or("", |x| x.0))
}

// the file for these rows (oldest first): at most PER_ROOM of any room, at most KEEP in all, the newest
pub fn encode<'a>(rows: impl DoubleEndedIterator<Item = (i64, Platform, &'a str)>) -> String {
  let mut per = std::collections::HashMap::<String, usize>::new();
  let mut kept: Vec<String> = Vec::new();
  for (ts, p, text) in rows.rev() {
    if kept.len() == KEEP {
      break;
    }
    let n = per.entry(room(p, text)).or_default();
    if *n == PER_ROOM {
      continue;
    }
    *n += 1;
    let text: String = text.chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect();
    kept.push(json!({"t": ts, "p": tag(p), "x": text}).to_string());
  }
  let mut out = String::new();
  for l in kept.iter().rev() {
    out.push_str(l);
    out.push('\n');
  }
  out
}

// the rows in a file, oldest first, and how many lines were unusable. every field is checked
pub fn decode(text: &str) -> (Vec<(i64, Platform, String)>, usize) {
  let (mut rows, mut junk) = (Vec::new(), 0);
  for l in text.lines().take(KEEP * 2) {
    let v: Option<Value> = serde_json::from_str(l).ok();
    let row = v.as_ref().and_then(|v| {
      let ts = v.get("t")?.as_i64().filter(|&t| t > 0)?;
      let p = untag(v.get("p")?.as_str()?)?;
      let x: String = v.get("x")?.as_str()?.chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect();
      (!x.trim().is_empty()).then_some((ts, p, x))
    });
    match row {
      Some(r) if rows.len() < KEEP => rows.push(r),
      Some(_) => {}
      None => junk += 1,
    }
  }
  (rows, junk)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn rows(spec: &[(&str, usize)]) -> Vec<(i64, Platform, String)> {
    let mut v = Vec::new();
    for (room, n) in spec {
      for i in 0..*n {
        v.push((v.len() as i64 + 1, Platform::Twitch, format!("{room} \u{b7} banned user{i}")));
      }
    }
    v
  }

  fn enc(v: &[(i64, Platform, String)]) -> String {
    encode(v.iter().map(|(t, p, x)| (*t, *p, x.as_str())))
  }

  #[test]
  fn what_goes_in_comes_back_oldest_first() {
    let v = rows(&[("forsen", 3)]);
    let (back, junk) = decode(&enc(&v));
    assert_eq!((back, junk), (v, 0));
  }

  #[test]
  fn a_busy_room_cannot_push_another_rooms_history_out() {
    // 250 in one room, then 5 in another that came first
    let mut v = rows(&[("quiet", 5)]);
    v.extend(rows(&[("busy", 250)]).into_iter().map(|(t, p, x)| (t + 100, p, x)));
    let (back, _) = decode(&enc(&v));
    let quiet = back.iter().filter(|r| r.2.starts_with("quiet")).count();
    let busy = back.iter().filter(|r| r.2.starts_with("busy")).count();
    assert_eq!((quiet, busy), (5, PER_ROOM), "the busy room keeps its newest hundred, the quiet one all five");
    let busy_rows = |n: &str| back.iter().any(|r| r.2.starts_with("busy") && r.2.ends_with(n));
    assert!(busy_rows("user249") && !busy_rows("user0"));
    // and the total never passes the tab's cap
    let many: Vec<_> = (0..10).flat_map(|r| rows(&[(&format!("room{r}"), 100)])).collect();
    assert_eq!(decode(&enc(&many)).0.len(), KEEP);
  }

  #[test]
  fn the_same_name_on_two_platforms_is_two_rooms() {
    let a = (1, Platform::Twitch, "x \u{b7} a".to_owned());
    let b = (2, Platform::Kick, "x \u{b7} b".to_owned());
    assert_ne!(room(a.1, &a.2), room(b.1, &b.2));
  }

  #[test]
  fn hostile_lines_are_junk_never_a_crash() {
    let file = [
      "not json".to_owned(),
      json!({"t": 5, "p": "t", "x": "ok \u{b7} banned bob"}).to_string(),
      json!({"t": -1, "p": "t", "x": "old"}).to_string(),
      json!({"t": "soon", "p": "t", "x": "x"}).to_string(),
      json!({"t": 5, "p": "z", "x": "x"}).to_string(),
      json!({"t": 5, "p": "t", "x": 7}).to_string(),
      json!({"t": 5, "p": "t", "x": "   "}).to_string(),
      json!({"t": 6, "p": "k", "x": format!("a\u{7}b{}", "y".repeat(5000))}).to_string(),
    ]
    .join("\n");
    let (rows, junk) = decode(&file);
    assert_eq!((rows.len(), junk), (2, 6));
    assert!(rows[1].2.starts_with("ab") && rows[1].2.chars().count() == MAX_TEXT, "control bytes out, text cut");
    // a file of a million lines reads only so many
    let big = "{}\n".repeat(1_000_000);
    assert_eq!(decode(&big).1, KEEP * 2);
  }
}
