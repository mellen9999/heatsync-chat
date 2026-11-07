// desktop notifications (ext fireNotification): notify-send with an argument list, never a shell. text
// is untrusted (chat lines, channel names), so it is escaped for the daemon's markup, stripped of
// control bytes and capped. best effort: no notify-send is said once on stderr
use std::time::{Duration, Instant};

// the popup itself: notify-send on unix, a tray balloon on windows
#[cfg_attr(unix, path = "notify_unix.rs")]
#[cfg_attr(windows, path = "notify_windows.rs")]
mod os;

// body cap (ext: 200 chars then "...")
pub const BODY_MAX: usize = 200;
const NAME_MAX: usize = 40;

// escape what a notification daemon may read as markup; control bytes become spaces
pub fn clean(s: &str, max: usize) -> String {
  let mut out = String::with_capacity(s.len().min(max * 2));
  for (n, c) in s.chars().enumerate() {
    if n == max {
      out.push_str("...");
      break;
    }
    match c {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      c if c.is_control() => out.push(' '),
      c => out.push(c),
    }
  }
  out
}

// a channel or person name, ready for a title (already escaped: `send` takes titles as given)
pub fn name(s: &str) -> String {
  clean(s, NAME_MAX)
}

// the text above is escaped for a daemon that reads markup; a windows balloon shows plain text, so it takes
// back exactly the three escapes `clean` makes, in one pass ("&amp;lt;" is "&lt;", not "<")
#[cfg(any(windows, test))]
fn unmark(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut rest = s;
  while let Some(i) = rest.find('&') {
    out.push_str(&rest[..i]);
    rest = &rest[i..];
    match [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>')].iter().find(|(e, _)| rest.starts_with(e)) {
      Some((e, c)) => {
        out.push(*c);
        rest = &rest[e.len()..];
      }
      None => {
        out.push('&');
        rest = &rest[1..];
      }
    }
  }
  out.push_str(rest);
  out
}

// `s` cut to at most `max` utf-16 units (what a balloon's fixed buffers count in) without splitting a character
#[cfg(any(windows, test))]
fn fit(s: &str, max: usize) -> &str {
  let mut units = 0;
  for (i, c) in s.char_indices() {
    units += c.len_utf16();
    if units > max {
      return &s[..i];
    }
  }
  s
}

// take the popup down before the process ends (windows: the tray icon; unix has nothing to remove)
pub fn shutdown() {
  os::shutdown();
}

// show one notification. `title` is built from `name()` parts; the body is cleaned here.
// `what` names the feature for the "missing notify-send" hint
pub fn send(what: &str, title: &str, body: &str) {
  os::show(what, title, &clean(body, BODY_MAX));
}

// at most one notification per GAP: a raid of mentions is one popup now and one "N more" when the
// window closes, not a process and a popup each. the owner polls `due` for its wake time
const GAP: Duration = Duration::from_secs(3);

pub struct Coalesce {
  last: Option<Instant>,
  held: u32,
  // what the follow-up counts, singular and plural: "1 more mention", "5 more mentions"
  noun: (&'static str, &'static str),
}

impl Default for Coalesce {
  fn default() -> Self {
    Self::new("mention", "mentions")
  }
}

impl Coalesce {
  pub fn new(one: &'static str, many: &'static str) -> Self {
    Self { last: None, held: 0, noun: (one, many) }
  }

  // true: show this one now. false: it is counted and rides in the follow-up
  pub fn offer(&mut self, now: Instant) -> bool {
    if self.last.is_some_and(|t| now < t + GAP) {
      self.held = self.held.saturating_add(1);
      return false;
    }
    self.last = Some(now);
    true
  }

  // when the follow-up is owed (None: nothing is held, so nothing wakes)
  pub fn due(&self) -> Option<Instant> {
    self.last.filter(|_| self.held > 0).map(|t| t + GAP)
  }

  // the follow-up's text once the window has closed
  pub fn tick(&mut self, now: Instant) -> Option<String> {
    if self.due().is_none_or(|t| t > now) {
      return None;
    }
    let n = std::mem::take(&mut self.held);
    // the follow-up is itself a notification: the next window starts here
    self.last = Some(now);
    Some(if n == 1 { format!("1 more {}", self.noun.0) } else { format!("{n} more {}", self.noun.1) })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_raid_of_mentions_is_one_popup_and_one_follow_up() {
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    let mut c = Coalesce::default();
    assert!(c.offer(t0), "the first shows at once");
    assert!(c.due().is_none(), "nothing held, nothing to wake for");
    for i in 1..=50 {
      assert!(!c.offer(t0 + ms(i * 50)), "inside the window");
    }
    assert_eq!(c.due(), Some(t0 + GAP));
    assert_eq!(c.tick(t0 + ms(2999)), None);
    assert_eq!(c.tick(t0 + GAP).as_deref(), Some("50 more mentions"));
    assert!(c.due().is_none() && c.tick(t0 + GAP * 9).is_none(), "said once");
    // the follow-up opened a new window; a mention in it waits, one after it shows
    assert!(!c.offer(t0 + GAP + ms(10)));
    assert_eq!(c.tick(t0 + GAP * 2).as_deref(), Some("1 more mention"));
    assert!(c.offer(t0 + GAP * 6));
  }

  #[test]
  fn a_burst_of_go_lives_is_one_popup_and_a_count() {
    let t0 = Instant::now();
    let mut c = Coalesce::new("stream update", "stream updates");
    assert!(c.offer(t0));
    for i in 1..=4 {
      assert!(!c.offer(t0 + Duration::from_millis(i * 100)));
    }
    assert_eq!(c.tick(t0 + GAP).as_deref(), Some("4 more stream updates"));
  }

  #[test]
  fn notification_text_is_escaped_and_capped() {
    assert_eq!(clean("a<b>&\n\u{7}c", 50), "a&lt;b&gt;&amp;  c");
    let long = "x".repeat(300);
    let c = clean(&long, BODY_MAX);
    assert_eq!(c.len(), BODY_MAX + 3);
    assert!(c.ends_with("..."));
    assert_eq!(clean("-h", 10), "-h");
    assert_eq!(name(&"y".repeat(90)).len(), NAME_MAX + 3);
  }

  #[test]
  fn a_balloon_buffer_is_filled_by_units_not_bytes() {
    assert_eq!(fit("abcdef", 4), "abcd");
    assert_eq!(fit("abc", 4), "abc");
    // an astral char is two units and is dropped whole rather than cut in half
    assert_eq!(fit("ab😀", 3), "ab");
    assert_eq!(fit("ab😀", 4), "ab😀");
    assert_eq!(fit("日本語", 2), "日本");
    assert_eq!(fit("", 0), "");
  }

  #[test]
  fn a_balloon_gets_back_the_plain_text() {
    for raw in ["a<b>&c", "&lt;", "x &amp; y", "&", "a & b <3", "&&amp;&", "日本 <é>"] {
      assert_eq!(unmark(&clean(raw, 50)), raw, "{raw}");
    }
    assert_eq!(unmark("&unknown; &#39;"), "&unknown; &#39;");
  }
}
