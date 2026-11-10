// mod slash commands (ext input.js): /ban <user> [reason], /timeout <user> [secs] [reason] (default 10
// minutes), /unban <user> (also /untimeout, /unto), /delete <msg-id>, plus /purge <user> (the ext's
// purge button: a 1 second timeout, which clears their lines), and /nuke <term> [seconds]

use crate::i18n::{f, t};

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
      Verb::Delete => t("mod.verb.deleted").into(),
      Verb::Timeout(s) => f("mod.verb.timed_out", &[("n", &s.to_string())]),
      Verb::Purge => t("mod.verb.purged").into(),
      Verb::Ban => t("mod.verb.banned").into(),
      Verb::Unban => t("mod.verb.unbanned").into(),
    }
  }

  // the action as a word in a status line (the slash command stays `name`)
  pub fn word(self) -> &'static str {
    match self {
      Verb::Delete => t("mod.verb.delete"),
      Verb::Timeout(_) => t("mod.verb.timeout"),
      Verb::Purge => t("mod.verb.purge"),
      Verb::Ban => t("mod.verb.ban"),
      Verb::Unban => t("mod.verb.unban"),
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
  // /nuke: the term and the lookback in seconds; None = the usage line (said after the channel checks, as the ext does)
  Nuke(Option<(String, u32)>),
}

// /nuke: a term shorter than this is refused
pub const NUKE_MIN_TERM: usize = 2;
// never more than this many deletes in one go
pub const NUKE_MAX: usize = 100;
pub const NUKE_SECS: u32 = 30;
pub const NUKE_MAX_SECS: u32 = 300;

// "<term> [seconds]": the ext's /^(.+?)(?:\s+(\d+))?$/ - a trailing number is the lookback, the rest the term
fn nuke(rest: &str) -> Option<(String, u32)> {
  let rest = rest.trim();
  let (term, secs) = match rest.rsplit_once(char::is_whitespace) {
    Some((t, n)) if !t.trim().is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => (t.trim(), Some(n)),
    _ => (rest, None),
  };
  if term.chars().count() < NUKE_MIN_TERM {
    return None;
  }
  // a number too long to parse is just "as far back as allowed"
  let secs = secs.map_or(NUKE_SECS, |n| n.parse::<u32>().unwrap_or(u32::MAX).clamp(1, NUKE_MAX_SECS));
  Some((term.to_owned(), secs))
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
  let text = text.trim();
  let (cmd, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
  let rest = rest.trim();
  let cmd = cmd.strip_prefix('/')?.to_ascii_lowercase();
  let (first, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  Some(match cmd.as_str() {
    "ban" | "b" => match name(first) {
      Some(u) => Cmd::Do(Verb::Ban, u, reason(tail)),
      None => Cmd::Usage(t("mod.cmd.ban_usage")),
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
      None => Cmd::Usage(t("mod.cmd.timeout_usage")),
    },
    "purge" => match name(first) {
      Some(u) => Cmd::Do(Verb::Purge, u, None),
      None => Cmd::Usage(t("mod.cmd.purge_usage")),
    },
    "unban" | "untimeout" | "unto" => match name(first) {
      Some(u) => Cmd::Do(Verb::Unban, u, None),
      None => Cmd::Usage(t("mod.cmd.unban_usage")),
    },
    "delete" | "del" => {
      // twitch and kick ids are uuids; youtube's are base64url (`_` and `-`)
      if !first.is_empty()
        && first.len() <= 128
        && first.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
      {
        Cmd::Do(Verb::Delete, first.to_owned(), None)
      } else {
        Cmd::Usage(t("mod.cmd.delete_usage"))
      }
    }
    "nuke" => Cmd::Nuke(nuke(rest)),
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
    assert_eq!(command("/delete ChwK_Gk-Nd"), d(Verb::Delete, "ChwK_Gk-Nd", None));
    assert!(matches!(command("/delete a;b"), Some(Cmd::Usage(_))));
    assert!(matches!(command("/delete"), Some(Cmd::Usage(_))));
    assert_eq!(command("/purge Bob"), d(Verb::Purge, "bob", None));
    assert_eq!(command("/banana bob"), None);
    assert_eq!(command("hello"), None);
  }

  #[test]
  fn nuke_takes_a_term_and_an_optional_lookback() {
    let n = |t: &str, s: u32| Some(Cmd::Nuke(Some((t.to_owned(), s))));
    assert_eq!(command("/nuke spam"), n("spam", 30));
    assert_eq!(command("/nuke spam 60"), n("spam", 60));
    // the term may hold spaces and stays as typed; only a trailing number is the lookback
    assert_eq!(command("/nuke Buy Followers   90"), n("Buy Followers", 90));
    assert_eq!(command("/nuke ab"), n("ab", 30));
    // capped at five minutes, floored at a second, never an overflow
    assert_eq!(command("/nuke spam 9999"), n("spam", 300));
    assert_eq!(command("/nuke spam 0"), n("spam", 1));
    assert_eq!(command("/nuke spam 99999999999999999999"), n("spam", 300));
    // under two characters, or nothing but a number, is the usage line
    for bad in ["/nuke", "/nuke a", "/nuke   a  ", "/nuke 5", "/nuke \u{e9}"] {
      assert_eq!(command(bad), Some(Cmd::Nuke(None)), "{bad}");
    }
    // "ab 5" is the term ab for five seconds, and "5 ab" the term "5 ab"
    assert_eq!(command("/nuke 5 ab"), n("5 ab", 30));
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
