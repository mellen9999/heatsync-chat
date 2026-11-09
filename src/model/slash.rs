// every /command the message box takes, with its arguments and what it does: tab completes from
// here and the / list shows it (ext slash-registry). settings switches come from the schema
use super::schema::{self, Ctl};
#[cfg(test)]
use crate::i18n::t;

pub struct Slash {
  pub name: &'static str,
  pub args: &'static str,
  // the i18n key of what it does, shown through `t`
  pub desc: &'static str,
}

const fn s(name: &'static str, args: &'static str, desc: &'static str) -> Slash {
  Slash { name, args, desc }
}

const FIXED: [Slash; 83] = [
  s("/op", "<text>", "keys.cmd.op"),
  s("/re", "<text>", "keys.cmd.re"),
  s("/me", "<text>", "keys.cmd.me"),
  s("/w", "<user> [text]", "keys.cmd.w"),
  s("/dm", "<user> [text]", "keys.cmd.dm"),
  s("/r", "<text>", "keys.cmd.r"),
  s("/join", "<channel>", "keys.cmd.join"),
  s("/part", "", "keys.cmd.part"),
  s("/keep", "", "keys.cmd.keep"),
  s("/sharelayout", "", "keys.cmd.sharelayout"),
  s("/layout", "<link>", "keys.cmd.layout"),
  s("/watch", "", "keys.cmd.watch"),
  s("/ban", "<user> [reason]", "keys.cmd.ban"),
  s("/timeout", "<user> [seconds] [reason]", "keys.cmd.timeout"),
  s("/unban", "<user>", "keys.cmd.unban"),
  s("/delete", "<message-id>", "keys.cmd.delete"),
  s("/purge", "<user>", "keys.cmd.purge"),
  s("/nuke", "<term> [seconds]", "keys.cmd.nuke"),
  s("/warn", "<user> <reason>", "keys.cmd.warn"),
  s("/shoutout", "<user>", "keys.cmd.shoutout"),
  s("/so", "<user>", "keys.cmd.so"),
  s("/shield", "[on|off]", "keys.cmd.shield"),
  s("/announce", "[blue|green|orange|purple] <text>", "keys.cmd.announce"),
  s("/vip", "<user>", "keys.cmd.vip"),
  s("/unvip", "<user>", "keys.cmd.unvip"),
  s("/mod", "<user>", "keys.cmd.mod"),
  s("/unmod", "<user>", "keys.cmd.unmod"),
  s("/color", "<name|#hex>", "keys.cmd.color"),
  s("/poll", "<title> | <a> | <b> [| secs]", "keys.cmd.poll"),
  s("/endpoll", "", "keys.cmd.endpoll"),
  s("/prediction", "<title> | <a> | <b> [| secs]", "keys.cmd.prediction"),
  s("/lockprediction", "", "keys.cmd.lockprediction"),
  s("/resolveprediction", "<n>", "keys.cmd.resolveprediction"),
  s("/cancelprediction", "", "keys.cmd.cancelprediction"),
  s("/modtools", "", "keys.cmd.modtools"),
  s("/unbanrequests", "", "keys.cmd.unbanrequests"),
  s("/blocked", "", "keys.cmd.blocked"),
  s("/summary", "", "keys.cmd.summary"),
  s("/status", "", "keys.cmd.status"),
  s("/predictions", "", "keys.cmd.predictions"),
  s("/slow", "[seconds|off]", "keys.cmd.slow"),
  s("/followers", "[10m|1h|1d|off]", "keys.cmd.followers"),
  s("/emoteonly", "[off]", "keys.cmd.emoteonly"),
  s("/subscribers", "[off]", "keys.cmd.subscribers"),
  s("/unique", "[off]", "keys.cmd.unique"),
  s("/modes", "", "keys.cmd.modes"),
  s("/mute", "<user> [10m|2h|3d|perm]", "keys.cmd.mute"),
  s("/unmute", "<user>", "keys.cmd.unmute"),
  s("/block", "<user>", "keys.cmd.block"),
  s("/unblock", "<user>", "keys.cmd.unblock"),
  s("/hide", "<user|tab>", "keys.cmd.hide"),
  s("/unhide", "<user>", "keys.cmd.unhide"),
  s("/show", "<tab>", "keys.cmd.show"),
  s("/note", "<user> [text]", "keys.cmd.note"),
  s("/delnote", "<user>", "keys.cmd.delnote"),
  s("/reset", "", "keys.cmd.reset"),
  s("/banreason", "[reason]", "keys.cmd.banreason"),
  s("/modbuttons", "[x 1m 10m 1h 24h 7d purge ban unban | off]", "keys.cmd.modbuttons"),
  s("/unbanreason", "<reason>", "keys.cmd.unbanreason"),
  s("/rule", "[add|edit|notify|sound|volume|del|on|off|up|down|list] ...", "keys.cmd.rule"),
  s("/mutes", "", "keys.cmd.mutes"),
  s("/blocks", "", "keys.cmd.blocks"),
  s("/login", "[twitch|kick|youtube]", "keys.cmd.login"),
  s("/logout", "", "keys.cmd.logout"),
  s("/account", "", "keys.cmd.account"),
  s("/following", "", "keys.cmd.following"),
  s("/live", "", "keys.cmd.live"),
  s("/import", "", "keys.cmd.import"),
  s("/emotes", "", "keys.cmd.emotes"),
  s("/settings", "", "keys.cmd.settings"),
  s("/keys", "", "keys.cmd.keys"),
  s("/help", "", "keys.cmd.help"),
  s("/tour", "", "keys.cmd.tour"),
  s("/set", "<setting> <value>", "keys.cmd.set"),
  s("/tab", "<name>", "keys.cmd.tab"),
  s("/user", "<name>", "keys.cmd.user"),
  s("/follow", "<name>", "keys.cmd.follow"),
  s("/unfollow", "<name>", "keys.cmd.unfollow"),
  s("/clearsent", "", "keys.cmd.clearsent"),
  s("/lclear", "", "keys.cmd.lclear"),
  s("/shrug", "[text]", "keys.cmd.shrug"),
  s("/tableflip", "[text]", "keys.cmd.tableflip"),
  s("/unflip", "[text]", "keys.cmd.unflip"),
];

pub fn all() -> Vec<Slash> {
  let switches = schema::DEFS.iter().filter_map(|d| match d.ctl {
    Ctl::Pill(..) => Some(s(d.cmd?, "", d.label_key())),
    Ctl::Zoom => Some(s(d.cmd?, "[percent]", d.label_key())),
    Ctl::Side => Some(s(d.cmd?, "<top|bottom|left|right>", d.label_key())),
    Ctl::Counter => Some(s(d.cmd?, "<unread|heat|off>", d.label_key())),
    Ctl::Pick(c, ..) => Some(s(d.cmd?, c.usage, d.label_key())),
    Ctl::Text(w) => Some(s(d.cmd?, w.usage, d.label_key())),
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
      assert!(c.name.starts_with('/') && !c.desc.is_empty(), "{}", c.name);
    }
    // the fixed rows each read as lowercase english (the settings switches take their schema label's key)
    for c in &FIXED {
      let d = t(c.desc);
      assert!(!d.is_empty() && d == d.to_lowercase(), "{}", c.name);
    }
    for c in ["/vi", "/zoom", "/glow", "/ban", "/mute", "/op", "/re", "/w", "/join", "/keys"] {
      assert!(names.contains(&c), "{c}");
    }
  }
}
