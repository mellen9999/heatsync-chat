// what a chat log says, as pure text: which folder a channel gets, and one line per message. every
// name and word in here came off the wire, so nothing reaches a path or a line unchecked
use crate::model::message::{Kind, Message, Platform};
use crate::util::time::Local;

// a channel folder is at most this many characters
const MAX_CHAN: usize = 64;
// a logged line keeps this many characters of a message (a platform allows 500; the rest is hostile)
const MAX_LINE: usize = 1200;

// the folder name a channel gets: letters, digits, `_` and `-` stay; everything else (`/`, `.`, NUL,
// control characters, spaces) becomes `_`, so it can't climb out of the logs folder or hide. never empty
pub fn chan_dir(chan: &str) -> String {
  let s: String =
    chan.chars().take(MAX_CHAN).map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
  let s = s.to_lowercase();
  if s.is_empty() { "_".into() } else { s }
}

// one line from any text: control characters, line breaks and the marks that reorder text become
// visible escapes, and a very long one is cut
pub fn escape(text: &str, out: &mut String) {
  for (n, c) in text.chars().enumerate() {
    if n >= MAX_LINE {
      out.push('…');
      return;
    }
    match c {
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      c if c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') =>
      {
        out.push_str(&format!("\\u{{{:x}}}", c as u32));
      }
      c => out.push(c),
    }
  }
}

fn who(m: &Message) -> &str {
  if m.nick.is_empty() { &m.login } else { &m.nick }
}

// `[hh:mm:ss] name: text`; a /me is `* name text`; a system line is its words alone; a notice that
// carries a person's words joins them with ` | `. None for what nobody said or announced
pub fn text(m: &Message, at: Local) -> Option<String> {
  let mut out = format!("[{}] ", at.hms());
  let mut any = false;
  if let Some(sys) = m.system.as_deref().filter(|s| !s.is_empty()) {
    escape(sys, &mut out);
    any = true;
  }
  match m.kind {
    Kind::Event => return None,
    Kind::System => {
      if any && !m.body.is_empty() {
        out.push_str(" | ");
      }
      escape(&m.body, &mut out);
      any |= !m.body.is_empty();
    }
    Kind::Chat | Kind::Action => {
      if m.body.is_empty() && any {
        return Some(out);
      }
      if any {
        out.push_str(" | ");
      }
      let action = m.kind == Kind::Action;
      if action {
        out.push_str("* ");
      }
      escape(who(m), &mut out);
      out.push_str(if action { " " } else { ": " });
      escape(&m.body, &mut out);
      any = true;
    }
  }
  any.then_some(out)
}

// where a line goes: platform / channel / day. built on the caller's thread, so the writer needs no
// clock and a message near midnight lands in the file of its own day
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
  pub platform: &'static str,
  pub chan: String,
  pub day: String,
  pub line: String,
}

pub fn entry(p: Platform, chan: &str, m: &Message, now_ms: i64) -> Option<Entry> {
  let at = Local::of(if m.ts > 0 { m.ts } else { now_ms })?;
  let mut line = text(m, at)?;
  line.push('\n');
  Some(Entry { platform: p.name(), chan: chan_dir(chan), day: at.day(), line })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn at() -> Local {
    Local { y: 2026, mo: 11, d: 6, h: 9, mi: 5, s: 3 }
  }

  fn chat(nick: &str, body: &str) -> Message {
    let mut m = Message::system(1, Platform::Twitch, "");
    m.kind = Kind::Chat;
    m.nick = nick.into();
    m.body = body.into();
    m
  }

  #[test]
  fn a_channel_cannot_climb_out_of_its_folder() {
    for bad in ["..", "../..", "a/b", ".hidden", "x\0y", "a\nb", "", "  ", "\\..\\", "a/../../etc"] {
      let d = chan_dir(bad);
      assert!(!d.is_empty() && !d.contains(['/', '.', '\0', '\n', '\\']), "{bad:?} -> {d:?}");
    }
    assert_eq!(chan_dir("Forsen"), "forsen");
    assert_eq!(chan_dir("some_chan-1"), "some_chan-1");
    assert_eq!(chan_dir(&"x".repeat(500)).chars().count(), MAX_CHAN);
  }

  #[test]
  fn one_message_is_one_line() {
    let l = text(&chat("bob", "hi\nthere\r\n\u{0}\u{1b}[31m\u{202e}x\ty"), at()).unwrap();
    assert_eq!(l, "[09:05:03] bob: hi\\nthere\\r\\n\\u{0}\\u{1b}[31m\\u{202e}x\\ty");
    assert!(!l.contains(['\n', '\r', '\0']));
    let mut long = String::new();
    escape(&"a".repeat(5000), &mut long);
    assert_eq!(long.chars().count(), MAX_LINE + 1);
    let n = text(&chat("bo\nb", "x"), at()).unwrap();
    assert_eq!(n, "[09:05:03] bo\\nb: x");
  }

  #[test]
  fn actions_system_lines_and_notices() {
    let mut a = chat("bob", "waves");
    a.kind = Kind::Action;
    assert_eq!(text(&a, at()).unwrap(), "[09:05:03] * bob waves");
    let s = Message::system(1, Platform::Twitch, "bob timed out for 60s");
    assert_eq!(text(&s, at()).unwrap(), "[09:05:03] bob timed out for 60s");
    let mut n = chat("bob", "gg");
    n.system = Some("bob subscribed".into());
    assert_eq!(text(&n, at()).unwrap(), "[09:05:03] bob subscribed | bob: gg");
    n.body = "".into();
    assert_eq!(text(&n, at()).unwrap(), "[09:05:03] bob subscribed");
    let mut e = Message::system(1, Platform::Twitch, "went live");
    e.kind = Kind::Event;
    assert!(text(&e, at()).is_none());
    assert!(text(&Message::system(1, Platform::Twitch, ""), at()).is_none());
  }

  #[test]
  fn a_line_lands_in_its_own_day() {
    let mut m = chat("bob", "hi");
    m.ts = 1_700_000_000_000;
    let e = entry(Platform::Kick, "../Chan", &m, 0).unwrap();
    assert_eq!((e.platform, e.chan.as_str()), ("kick", "___chan"));
    assert_eq!(e.day.len(), 10);
    assert!(e.line.ends_with("bob: hi\n"));
    // two days apart is a different calendar day wherever the clock is
    m.ts += 2 * 86_400_000;
    let l = entry(Platform::Kick, "c", &m, 0).unwrap();
    assert_ne!(l.day, e.day);
  }
}
