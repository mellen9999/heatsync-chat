// a second launch wakes the first: what the two say to each other, the same on every platform. the
// transport is beside it (wake_unix.rs: a socket next to the lock; wake_windows.rs: a named pipe). the newcomer
// says "show" (plus any channels it was given) and exits; the window answers "ok". every byte read is
// hostile: the message is capped, the read has a deadline, and anything that isn't the expected shape is ignored
use std::time::Duration;

pub(super) const MAX_MSG: usize = 2048;
const MAX_OPEN: usize = 8;
const MAX_CHAN: usize = 100;
pub(super) const DEADLINE: Duration = Duration::from_secs(1);

// "show\n" then "open <channel line>\n" for each channel the newcomer was given
pub(super) fn encode(open: &[Box<str>]) -> String {
  let mut s = String::from("show\n");
  for c in open.iter().filter(|c| clean(c)).take(MAX_OPEN) {
    s.push_str(&format!("open {c}\n"));
  }
  s
}

fn clean(c: &str) -> bool {
  !c.is_empty() && c.len() <= MAX_CHAN && !c.chars().any(char::is_control)
}

// the channels asked for, or None when this isn't a "show" message at all
pub(super) fn decode(bytes: &[u8]) -> Option<Vec<Box<str>>> {
  let mut lines = std::str::from_utf8(bytes).ok()?.lines();
  if lines.next()? != "show" {
    return None;
  }
  Some(lines.filter_map(|l| l.strip_prefix("open ")).filter(|c| clean(c)).take(MAX_OPEN).map(Into::into).collect())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_a_show_message_is_a_request() {
    for bad in [&b"hello\n"[..], b"\xff\xfe", b"", b"open xqc\nshow\n"] {
      assert_eq!(decode(bad), None, "{bad:?}");
    }
    // control bytes, empty lines and noise are dropped, not trusted
    let got = decode(b"show\nopen bad\x1b[0m\nopen \nnonsense\nopen ok1\n").unwrap();
    assert_eq!(got, ["ok1".into()] as [Box<str>; 1]);
  }

  #[test]
  fn too_many_channels_are_capped_on_the_way_out_and_in() {
    let many: Vec<Box<str>> = (0..20).map(|n| format!("c{n}").into()).collect();
    assert_eq!(encode(&many).lines().count(), 1 + MAX_OPEN);
    assert_eq!(decode(format!("show\n{}", "open c\n".repeat(20)).as_bytes()).unwrap().len(), MAX_OPEN);
    let long: Box<str> = "x".repeat(MAX_CHAN + 1).into();
    assert_eq!(encode(&[long]), "show\n");
  }

  #[test]
  fn a_message_round_trips() {
    let o: Vec<Box<str>> = vec!["xqc".into(), "kick:amouranth".into()];
    assert_eq!(decode(encode(&o).as_bytes()).unwrap(), o);
    assert!(decode(encode(&[]).as_bytes()).unwrap().is_empty());
  }
}
