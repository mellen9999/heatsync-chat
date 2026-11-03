// every /command the message box takes, with its arguments and what it does: tab completes from
// here and the / list shows it (ext slash-registry). settings switches come from the schema
use super::schema::{self, Ctl};

pub struct Slash {
  pub name: &'static str,
  pub args: &'static str,
  pub desc: &'static str,
}

const fn s(name: &'static str, args: &'static str, desc: &'static str) -> Slash {
  Slash { name, args, desc }
}

const FIXED: [Slash; 27] = [
  s("/me", "<text>", "say it as an action"),
  s("/w", "<user> [text]", "whisper someone"),
  s("/r", "<text>", "answer the last whisper"),
  s("/join", "<channel>", "open a channel tab"),
  s("/part", "", "close this tab"),
  s("/watch", "", "watch this channel's stream (mpv, else the browser)"),
  s("/ban", "<user> [reason]", "ban"),
  s("/timeout", "<user> [seconds] [reason]", "time out"),
  s("/unban", "<user>", "lift a ban or timeout"),
  s("/delete", "<message-id>", "delete a line (or select it, press m)"),
  s("/purge", "<user>", "clear someone's lines"),
  s("/mute", "<user> [10m|2h|3d|perm]", "hide someone, a day by default"),
  s("/unmute", "<user>", "show them again"),
  s("/block", "<user>", "block, or unblock"),
  s("/unblock", "<user>", "unblock"),
  s("/hide", "<user|tab>", "hide someone here, or a tab"),
  s("/unhide", "<user>", "bring them back"),
  s("/show", "<tab>", "show a hidden tab"),
  s("/note", "<user> [text]", "a private note on someone; no text deletes it"),
  s("/hideword", "[term]", "hide lines with a term; alone, list them"),
  s("/unhideword", "<term>", "stop hiding a term"),
  s("/highlight", "[term]", "light up lines with a word; alone, list them"),
  s("/unhighlight", "<term>", "stop lighting a word"),
  s("/mutes", "", "who is muted"),
  s("/blocks", "", "who is blocked"),
  s("/settings", "", "open settings"),
  s("/keys", "", "every key"),
];

pub fn all() -> Vec<Slash> {
  let switches = schema::DEFS.iter().filter_map(|d| match d.ctl {
    Ctl::Pill(..) => Some(s(d.cmd?, "", d.label)),
    Ctl::Zoom => Some(s(d.cmd?, "[percent]", d.label)),
    Ctl::Side => Some(s(d.cmd?, "<top|bottom|left|right>", d.label)),
    Ctl::Counter => Some(s(d.cmd?, "<unread|heat|off>", d.label)),
    Ctl::Pick(c, ..) => Some(s(d.cmd?, c.usage, d.label)),
    _ => None,
  });
  FIXED.into_iter().chain(switches).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_command_is_handled_and_listed_once() {
    let all = all();
    let mut names: Vec<&str> = all.iter().map(|c| c.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all.len());
    for c in &all {
      assert!(c.name.starts_with('/') && !c.desc.is_empty() && c.desc == c.desc.to_lowercase(), "{}", c.name);
    }
    for c in ["/vi", "/zoom", "/glow", "/ban", "/mute", "/w", "/join", "/keys"] {
      assert!(names.contains(&c), "{c}");
    }
  }
}
