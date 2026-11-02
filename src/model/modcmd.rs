// mod slash commands (ext input.js): /ban <user> [reason], /timeout <user> [secs] [reason] (default 10
// minutes), /unban <user> (also /untimeout, /unto), /delete <msg-id>, plus /purge <user> (the ext's
// purge button: a 1 second timeout, which clears their lines)

// what a toolbar button does, and its label in the "did it" line (ext mc_mod_label_*)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
  Delete,
  Timeout(u32),
  // a 1 second timeout: clears their lines without a real mute
  Purge,
  Ban,
  Unban,
}

// the ext's timeout buttons: 1m 10m 1h 24h 7d
pub const DURATIONS: [(&str, u32); 5] = [("1m", 60), ("10m", 600), ("1h", 3600), ("24h", 86400), ("7d", 604800)];
pub const DEFAULT_TIMEOUT: u32 = 600;
// twitch caps a timeout at 14 days
pub const MAX_TIMEOUT: u32 = 14 * 24 * 3600;
const MAX_REASON: usize = 500;

impl Verb {
  // the server's action word
  pub fn action(self) -> &'static str {
    match self {
      Verb::Delete => "delete_message",
      Verb::Timeout(_) | Verb::Purge => "timeout",
      Verb::Ban => "ban",
      Verb::Unban => "unban",
    }
  }

  pub fn seconds(self) -> Option<u32> {
    match self {
      Verb::Timeout(s) => Some(s),
      Verb::Purge => Some(1),
      _ => None,
    }
  }

  // past tense, as the ext toasts it ("banned bob", "timed out 600s bob")
  pub fn done(self) -> String {
    match self {
      Verb::Delete => "deleted".into(),
      Verb::Timeout(s) => format!("timed out {s}s"),
      Verb::Purge => "purged".into(),
      Verb::Ban => "banned".into(),
      Verb::Unban => "unbanned".into(),
    }
  }

  pub fn name(self) -> &'static str {
    match self {
      Verb::Delete => "delete",
      Verb::Timeout(_) => "timeout",
      Verb::Purge => "purge",
      Verb::Ban => "ban",
      Verb::Unban => "unban",
    }
  }
}

#[derive(PartialEq, Eq, Debug)]
pub enum Cmd {
  // verb, the user (or, for delete, the message id), a reason when given
  Do(Verb, String, Option<String>),
  Usage(&'static str),
}

// "90", "10m", "2h", "1d" → seconds, clamped to what twitch allows. None = not a duration
pub fn seconds(s: &str) -> Option<u32> {
  let n = crate::util::time::span(s)?;
  Some(u32::try_from(n).unwrap_or(u32::MAX).clamp(1, MAX_TIMEOUT))
}

// a user name as typed after a command: "@Bob" → "bob". None = not a plausible login
pub fn name(w: &str) -> Option<String> {
  let n = w.trim_start_matches('@').to_ascii_lowercase();
  (!n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_alphanumeric() || "_-.".contains(c))).then_some(n)
}

fn reason(r: &str) -> Option<String> {
  let r = r.trim();
  (!r.is_empty()).then(|| r.chars().take(MAX_REASON).collect())
}

pub fn command(text: &str) -> Option<Cmd> {
  let t = text.trim();
  let (cmd, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
  let rest = rest.trim();
  let cmd = cmd.strip_prefix('/')?.to_ascii_lowercase();
  let (first, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  Some(match cmd.as_str() {
    "ban" | "b" => match name(first) {
      Some(u) => Cmd::Do(Verb::Ban, u, reason(tail)),
      None => Cmd::Usage("usage: /ban <user> [reason]"),
    },
    "timeout" | "to" => match name(first) {
      Some(u) => {
        // an optional length, then the reason
        let (len, why) = tail.trim().split_once(char::is_whitespace).unwrap_or((tail.trim(), ""));
        match seconds(len) {
          Some(s) => Cmd::Do(Verb::Timeout(s), u, reason(why)),
          None => Cmd::Do(Verb::Timeout(DEFAULT_TIMEOUT), u, reason(tail)),
        }
      }
      None => Cmd::Usage("usage: /timeout <user> [seconds] [reason]"),
    },
    "purge" => match name(first) {
      Some(u) => Cmd::Do(Verb::Purge, u, None),
      None => Cmd::Usage("usage: /purge <user>"),
    },
    "unban" | "untimeout" | "unto" => match name(first) {
      Some(u) => Cmd::Do(Verb::Unban, u, None),
      None => Cmd::Usage("usage: /unban <user>"),
    },
    "delete" | "del" => {
      if !first.is_empty() && first.len() <= 128 && first.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        Cmd::Do(Verb::Delete, first.to_owned(), None)
      } else {
        Cmd::Usage("usage: /delete <message-id> (select a line and press m)")
      }
    }
    _ => return None,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn d(v: Verb, u: &str, r: Option<&str>) -> Option<Cmd> {
    Some(Cmd::Do(v, u.into(), r.map(Into::into)))
  }

  #[test]
  fn ban_and_unban() {
    assert_eq!(command("/ban @Bob spamming links"), d(Verb::Ban, "bob", Some("spamming links")));
    assert_eq!(command("/b bob"), d(Verb::Ban, "bob", None));
    assert_eq!(command("/unban bob"), d(Verb::Unban, "bob", None));
    assert_eq!(command("/unto bob"), d(Verb::Unban, "bob", None));
    assert!(matches!(command("/ban"), Some(Cmd::Usage(_))));
    assert!(matches!(command("/ban bad;name"), Some(Cmd::Usage(_))));
  }

  #[test]
  fn timeout_length_is_optional() {
    assert_eq!(command("/timeout bob"), d(Verb::Timeout(600), "bob", None));
    assert_eq!(command("/timeout bob 30"), d(Verb::Timeout(30), "bob", None));
    assert_eq!(command("/to bob 2h too loud"), d(Verb::Timeout(7200), "bob", Some("too loud")));
    // no length: the rest is the reason
    assert_eq!(command("/timeout bob too loud"), d(Verb::Timeout(600), "bob", Some("too loud")));
    assert_eq!(command("/timeout bob 99d"), d(Verb::Timeout(MAX_TIMEOUT), "bob", None));
  }

  #[test]
  fn delete_and_purge() {
    assert_eq!(command("/delete 0a1b-22"), d(Verb::Delete, "0a1b-22", None));
    assert!(matches!(command("/delete"), Some(Cmd::Usage(_))));
    assert_eq!(command("/purge Bob"), d(Verb::Purge, "bob", None));
    assert_eq!(command("/banana bob"), None);
    assert_eq!(command("hello"), None);
  }

  #[test]
  fn lengths() {
    assert_eq!(seconds("90"), Some(90));
    assert_eq!(seconds("10m"), Some(600));
    assert_eq!(seconds("1d"), Some(86400));
    assert_eq!(seconds("0"), Some(1));
    assert_eq!(seconds("abc"), None);
    assert_eq!(seconds(""), None);
  }

  #[test]
  fn labels() {
    assert_eq!(Verb::Timeout(600).done(), "timed out 600s");
    assert_eq!(Verb::Purge.seconds(), Some(1));
    assert_eq!(Verb::Delete.action(), "delete_message");
  }
}
