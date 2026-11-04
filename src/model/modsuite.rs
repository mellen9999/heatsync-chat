// twitch's mod suite commands (ext input.js mod suite): /warn <user> <reason>, /shoutout <user> (/so)
// and /shield [on|off] (bare flips it). heatsync's server makes the helix calls (/api/mod/warn,
// /shoutout, /shield) and is the real moderator check
use super::ident;

pub const MAX_REASON: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
  Warn { user: Box<str>, reason: String },
  Shoutout(Box<str>),
  // None = flip what it is now
  Shield(Option<bool>),
}

// a twitch login as typed: `@` dropped, lowercased, 1-25 of a-z 0-9 _
pub fn login(s: &str) -> Option<Box<str>> {
  let l = s.trim().trim_start_matches('@').to_ascii_lowercase();
  ident::twitch_login(&l).then(|| l.into())
}

// Some(Ok) / Some(Err(usage)) for ours, None for anything else
pub fn command(text: &str) -> Option<Result<Cmd, &'static str>> {
  let rest = text.trim().strip_prefix('/')?;
  let (word, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  let arg = arg.trim();
  Some(match word.to_ascii_lowercase().as_str() {
    "warn" => {
      let (who, why) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
      let reason: String = why.trim().chars().take(MAX_REASON).collect();
      match login(who) {
        Some(user) if !reason.is_empty() => Ok(Cmd::Warn { user, reason }),
        _ => Err("usage: /warn <user> <reason>"),
      }
    }
    "shoutout" | "so" => match login(arg) {
      Some(u) if !arg.contains(char::is_whitespace) => Ok(Cmd::Shoutout(u)),
      _ => Err("usage: /shoutout <user>"),
    },
    "shield" => match arg.to_ascii_lowercase().as_str() {
      "" => Ok(Cmd::Shield(None)),
      "on" => Ok(Cmd::Shield(Some(true))),
      "off" => Ok(Cmd::Shield(Some(false))),
      _ => Err("usage: /shield [on|off]"),
    },
    _ => return None,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_like_the_ext() {
    assert_eq!(
      command("/warn @Bob stop  spamming "),
      Some(Ok(Cmd::Warn { user: "bob".into(), reason: "stop  spamming".into() }))
    );
    assert_eq!(command("/warn bob"), Some(Err("usage: /warn <user> <reason>")));
    assert_eq!(command("/warn b.o.b why"), Some(Err("usage: /warn <user> <reason>")));
    let long = format!("/warn bob {}", "x".repeat(900));
    assert!(matches!(command(&long), Some(Ok(Cmd::Warn { reason, .. })) if reason.len() == MAX_REASON));
    assert_eq!(command("/so forsen"), Some(Ok(Cmd::Shoutout("forsen".into()))));
    assert_eq!(command("/shoutout a b"), Some(Err("usage: /shoutout <user>")));
    assert_eq!(command("/shield"), Some(Ok(Cmd::Shield(None))));
    assert_eq!(command("/SHIELD off"), Some(Ok(Cmd::Shield(Some(false)))));
    assert_eq!(command("/shield maybe"), Some(Err("usage: /shield [on|off]")));
    assert!(command("/warning x y").is_none() && command("warn bob x").is_none());
    assert_eq!(login(&"a".repeat(26)), None);
  }
}
