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

const FIXED: [Slash; 87] = [
  s("/op", "<text>", "post a thread to your heatsync feed"),
  s("/re", "<text>", "answer the last op shown in chat"),
  s("/me", "<text>", "say it as an action"),
  s("/w", "<user> [text]", "whisper someone"),
  s("/dm", "<user> [text]", "heatsync dm (needs a follow both ways)"),
  s("/r", "<text>", "answer the last whisper or dm"),
  s("/join", "<channel>", "open a channel tab"),
  s("/part", "", "close this tab"),
  s("/keep", "", "save this temporary tab (a channel opened from following)"),
  s("/sharelayout", "", "copy your tabs as a heatsync.org/m/ link (/sl)"),
  s("/layout", "<link>", "open the tabs of a heatsync.org/m/ link"),
  s("/watch", "", "watch this channel's stream (mpv, else the browser)"),
  s("/ban", "<user> [reason]", "ban"),
  s("/timeout", "<user> [seconds] [reason]", "time out"),
  s("/unban", "<user>", "lift a ban or timeout"),
  s("/delete", "<message-id>", "delete a line (or select it, press m)"),
  s("/purge", "<user>", "clear someone's lines"),
  s("/nuke", "<term> [seconds]", "delete recent lines containing a term (mods)"),
  s("/warn", "<user> <reason>", "a twitch warning they must accept to chat"),
  s("/shoutout", "<user>", "twitch shoutout (live only)"),
  s("/so", "<user>", "same as /shoutout"),
  s("/shield", "[on|off]", "shield mode; alone, flips it"),
  s("/announce", "[blue|green|orange|purple] <text>", "highlighted announcement in chat (mods)"),
  s("/vip", "<user>", "give vip (broadcaster)"),
  s("/unvip", "<user>", "take vip away (broadcaster)"),
  s("/mod", "<user>", "make a moderator (broadcaster)"),
  s("/unmod", "<user>", "remove a moderator (broadcaster)"),
  s("/color", "<name|#hex>", "your twitch chat color; hex needs turbo or prime"),
  s("/poll", "<title> | <a> | <b> [| secs]", "start a poll, 2-5 choices (broadcaster)"),
  s("/endpoll", "", "end the running poll (broadcaster)"),
  s("/prediction", "<title> | <a> | <b> [| secs]", "start a prediction, 2-10 outcomes (broadcaster)"),
  s("/lockprediction", "", "lock the open prediction (broadcaster)"),
  s("/resolveprediction", "<n>", "pick the winning outcome (broadcaster)"),
  s("/cancelprediction", "", "cancel and refund the prediction (broadcaster)"),
  s("/modtools", "", "shield mode, automod levels, unban requests, chatters (mods)"),
  s("/unbanrequests", "", "same as /modtools"),
  s("/blocked", "", "this channel's twitch blocked terms (mods)"),
  s("/summary", "", "this tab's stream stats: lines, chatters, top emotes"),
  s("/status", "", "this twitch channel's stream and chat modes (mods change them)"),
  s("/predictions", "", "this twitch channel's prediction: odds, points, bettors, the clock (read only)"),
  s("/slow", "[seconds|off]", "slow mode, 30s by default (mods)"),
  s("/followers", "[10m|1h|1d|off]", "followers-only chat (mods)"),
  s("/emoteonly", "[off]", "emote-only chat (mods)"),
  s("/subscribers", "[off]", "subs-only chat (mods)"),
  s("/unique", "[off]", "unique chat: no repeated lines (mods)"),
  s("/modes", "", "same as /status"),
  s("/mute", "<user> [10m|2h|3d|perm]", "hide someone, a day by default"),
  s("/unmute", "<user>", "show them again"),
  s("/block", "<user>", "block, or unblock"),
  s("/unblock", "<user>", "unblock"),
  s("/hide", "<user|tab>", "hide someone here, or a tab"),
  s("/unhide", "<user>", "bring them back"),
  s("/show", "<tab>", "show a hidden tab"),
  s("/note", "<user> [text]", "a private note on someone; no text deletes it"),
  s("/delnote", "<user>", "remove your note"),
  s("/reset", "", "put every look and behaviour setting back to its default (asks first)"),
  s("/hideword", "[term]", "hide lines with a term; alone, list them"),
  s("/unhideword", "<term>", "stop hiding a term"),
  s("/highlight", "[term]", "light up lines with a word; alone, list them"),
  s("/unhighlight", "<term>", "stop lighting a word"),
  s("/automod", "[pattern]", "hide lines matching a regex; alone, list them"),
  s("/unautomod", "<pattern>", "stop hiding a pattern"),
  s("/banreason", "[reason]", "a quick reason your bans offer; alone, list them"),
  s("/modbuttons", "[x 1m 10m 1h 24h 7d purge ban unban | off]", "the hover buttons on a line you can moderate"),
  s("/unbanreason", "<reason>", "stop offering a reason"),
  s("/rule", "[add|del|on|off|up|down|list] ...", "per-rule hide / highlight filters"),
  s("/mutes", "", "who is muted"),
  s("/blocks", "", "who is blocked"),
  s("/login", "[twitch|kick|youtube]", "log in; when in, link a platform"),
  s("/logout", "", "log out"),
  s("/account", "", "who you are logged in as, and how to log out"),
  s("/following", "", "who you follow that is live: game, viewers, uptime; enter opens a tab, w watches"),
  s("/live", "", "same as /following"),
  s("/import", "", "add the channels you follow on twitch as tabs: pick, enter adds"),
  s("/emotes", "", "your heatsync emotes: rename, reorder, remove, undo, saved sets, share (ctrl-p)"),
  s("/settings", "", "open settings"),
  s("/keys", "", "every key"),
  s("/help", "", "same as /keys"),
  s("/set", "<setting> <value>", "change any setting: /set timestamps on"),
  s("/tab", "<name>", "go to a tab: a channel, feed, mentions, whispers, modlog"),
  s("/user", "<name>", "their card"),
  s("/follow", "<name>", "follow them on heatsync: their posts fill your feed"),
  s("/unfollow", "<name>", "stop following them on heatsync"),
  s("/lclear", "", "empty this tab's lines (only here, not for anyone else)"),
  s("/shrug", "[text]", "say it with ¯\\_(ツ)_/¯"),
  s("/tableflip", "[text]", "say it with (╯°□°)╯︵ ┻━┻"),
  s("/unflip", "[text]", "say it with ┬─┬ノ( ゜-゜ノ)"),
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
    for c in ["/vi", "/zoom", "/glow", "/ban", "/mute", "/op", "/re", "/w", "/join", "/keys"] {
      assert!(names.contains(&c), "{c}");
    }
  }
}
