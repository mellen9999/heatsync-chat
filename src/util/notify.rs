// desktop notifications (ext fireNotification): notify-send with an argument list, never a shell. text
// is untrusted (chat lines, channel names), so it is escaped for the daemon's markup, stripped of
// control bytes and capped. best effort: no notify-send is said once on stderr
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

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

// show one notification. `title` is built from `name()` parts; the body is cleaned here.
// `what` names the feature for the "missing notify-send" hint
pub fn send(what: &str, title: &str, body: &str) {
  static MISSING: AtomicBool = AtomicBool::new(false);
  let spawned = Command::new("notify-send")
    .args(["-a", "heatsync", "-t", "8000", "--"])
    .arg(title)
    .arg(clean(body, BODY_MAX))
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn();
  match spawned {
    // reap it so a long session leaves no zombies
    Ok(mut child) => {
      std::thread::spawn(move || {
        let _ = child.wait();
      });
    }
    Err(e) => {
      if !MISSING.swap(true, Ordering::Relaxed) {
        eprintln!("{what} notifications need notify-send: {e}");
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
}
