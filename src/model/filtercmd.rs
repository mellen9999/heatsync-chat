// slash commands for the chat filters, ext names where the ext has them (/mute /unmute /block /hide
// /unhide) and plain switches in the style of /vi and /glow for the rest:
//   /mute <user> [10m|2h|3d|perm]   /unmute <user>   (a day by default, like the ext)
//   /block <user> (toggles)   /unblock <user>
//   /hide <user>   /unhide <user>             this tab only, until restart
//   /banreason [text]  /unbanreason <text>    quick reasons a ban offers; no text lists them
//   /rule [list]                              the hide / highlight rules, in the order they win (words, patterns, users...)
//   /rule add <hide|highlight> <keyword|regex|user|badge|msgtype|expr> <value>
//   /rule edit <number|id> <hide|highlight> [kind] <value>   change it in place: same place, colour, sound
//   /rule notify <number|id> <on|off>          a highlight also lands in the mentions tab
//   /rule sound <number|id> <off|ping|blip|knock|chime|a .wav path>   /rule volume <number|id> <0-100|off>
//   /rule del|on|off|up|down <number|id>
//   /mutes  /blocks                           who is muted / blocked
use crate::i18n::t;
use crate::model::modcmd::name;
use crate::util::time::span;

#[derive(PartialEq, Eq, Debug)]
pub enum Span {
  Default,
  For(i64),
  Forever,
}

#[derive(PartialEq, Eq, Debug)]
pub enum Cmd {
  Mute(String, Span),
  Unmute(String),
  Block(String),
  Unblock(String),
  // user, hide (true) or bring back (false)
  Hide(String, bool),
  // a ban reason: add or remove, and the reason (None = list them)
  Reason { add: bool, text: Option<String> },
  // /rule ...
  Rule(RuleCmd),
  // show who is muted / blocked
  Mutes,
  Blocks,
  Usage(&'static str),
}

#[derive(PartialEq, Eq, Debug)]
pub enum RuleCmd {
  List,
  Add { action: String, ty: String, value: String },
  // the same words as an add, for the rule named: it keeps its place
  Edit { key: String, action: String, ty: String, value: String },
  Notify(String, bool),
  // a preset's name, a wav path, or off
  Sound(String, String),
  // 0..=100, None = the shared mention volume
  Volume(String, Option<u8>),
  Del(String),
  Enable(String, bool),
  Shift(String, bool),
}

const RULE_TYPES: [&str; 6] = ["keyword", "regex", "user", "badge", "msgtype", "expr"];

// `<hide|highlight> [kind] <value>` as an add and an edit both take it: no kind named = a keyword, and a kind
// word with nothing after it is not a keyword
fn spec(tail: &str) -> Option<(String, String, String)> {
  let mut p = tail.splitn(3, char::is_whitespace);
  match (p.next(), p.next(), p.next().map(str::trim)) {
    (Some(a), Some(t), Some(v)) if !v.is_empty() && RULE_TYPES.contains(&t.to_ascii_lowercase().as_str()) => {
      Some((a.to_ascii_lowercase(), t.to_ascii_lowercase(), v.to_owned()))
    }
    (Some(a), Some(t), _) if !RULE_TYPES.contains(&t.to_ascii_lowercase().as_str()) => {
      let value = tail.split_once(char::is_whitespace).map_or("", |x| x.1).trim();
      Some((a.to_ascii_lowercase(), "keyword".into(), value.to_owned()))
    }
    _ => None,
  }
}

fn rule(rest: &str) -> Cmd {
  let (verb, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  let tail = tail.trim();
  let key = || tail.split_whitespace().next().filter(|_| tail.split_whitespace().count() == 1).map(str::to_owned);
  let r = match verb.to_ascii_lowercase().as_str() {
    "" | "list" if tail.is_empty() => RuleCmd::List,
    "add" => match spec(tail) {
      Some((action, ty, value)) => RuleCmd::Add { action, ty, value },
      None => return Cmd::Usage(t("mod.filter.rule_usage")),
    },
    "edit" => {
      let (key, rest) = tail.split_once(char::is_whitespace).unwrap_or((tail, ""));
      match spec(rest.trim()) {
        Some((action, ty, value)) if !key.is_empty() => RuleCmd::Edit { key: key.to_owned(), action, ty, value },
        _ => return Cmd::Usage(t("mod.filter.rule_usage")),
      }
    }
    "notify" => match tail.split_whitespace().collect::<Vec<_>>()[..] {
      [k, "on"] => RuleCmd::Notify(k.to_owned(), true),
      [k, "off"] => RuleCmd::Notify(k.to_owned(), false),
      _ => return Cmd::Usage(t("mod.filter.rule_usage")),
    },
    "sound" => match tail.split_once(char::is_whitespace) {
      Some((k, what)) if !what.trim().is_empty() => RuleCmd::Sound(k.to_owned(), what.trim().to_owned()),
      _ => return Cmd::Usage(t("mod.filter.rule_usage")),
    },
    "volume" => match tail.split_whitespace().collect::<Vec<_>>()[..] {
      [k, "off" | "default"] => RuleCmd::Volume(k.to_owned(), None),
      [k, n] => match n.parse::<u8>().ok().filter(|n| *n <= 100) {
        Some(n) => RuleCmd::Volume(k.to_owned(), Some(n)),
        None => return Cmd::Usage(t("mod.filter.rule_usage")),
      },
      _ => return Cmd::Usage(t("mod.filter.rule_usage")),
    },
    "del" | "rm" | "remove" => match key() {
      Some(k) => RuleCmd::Del(k),
      None => return Cmd::Usage(t("mod.filter.rule_usage")),
    },
    v @ ("on" | "off" | "up" | "down") => match key() {
      Some(k) => match v {
        "on" | "off" => RuleCmd::Enable(k, v == "on"),
        _ => RuleCmd::Shift(k, v == "up"),
      },
      None => return Cmd::Usage(t("mod.filter.rule_usage")),
    },
    _ => return Cmd::Usage(t("mod.filter.rule_usage")),
  };
  Cmd::Rule(r)
}

// the longest mute we accept, in seconds: a year
const MAX_SPAN: u64 = 365 * 86400;

pub fn command(text: &str) -> Option<Cmd> {
  let text = text.trim();
  let (cmd, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
  let rest = rest.trim();
  let cmd = cmd.strip_prefix('/')?.to_ascii_lowercase();
  let (first, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  let user = |usage: &'static str, f: &dyn Fn(String) -> Cmd| name(first).map_or(Cmd::Usage(usage), f);
  let term = (!rest.is_empty()).then(|| rest.to_owned());
  Some(match cmd.as_str() {
    "mute" => match name(first) {
      None => Cmd::Usage(t("mod.filter.mute_usage")),
      Some(u) => match tail.trim() {
        "" => Cmd::Mute(u, Span::Default),
        "perm" | "forever" => Cmd::Mute(u, Span::Forever),
        l => match span(l).filter(|&s| s > 0 && s <= MAX_SPAN) {
          Some(s) => Cmd::Mute(u, Span::For(s as i64 * 1000)),
          None => Cmd::Usage(t("mod.filter.mute_usage")),
        },
      },
    },
    "unmute" => user(t("mod.filter.unmute_usage"), &Cmd::Unmute),
    "block" => user(t("mod.filter.block_usage"), &Cmd::Block),
    "unblock" => user(t("mod.filter.unblock_usage"), &Cmd::Unblock),
    "hide" => user(t("mod.filter.hide_usage"), &|u| Cmd::Hide(u, true)),
    "unhide" => user(t("mod.filter.unhide_usage"), &|u| Cmd::Hide(u, false)),
    "banreason" => Cmd::Reason { add: true, text: term },
    "unbanreason" if term.is_some() => Cmd::Reason { add: false, text: term },
    "unbanreason" => Cmd::Usage(t("mod.filter.unbanreason_usage")),
    "rule" => rule(rest),
    // the lists
    "mutes" if rest.is_empty() => Cmd::Mutes,
    "blocks" if rest.is_empty() => Cmd::Blocks,
    _ => return None,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ban_reasons_keep_spaces() {
    assert_eq!(command("/banreason hate  speech"), Some(Cmd::Reason { add: true, text: Some("hate  speech".into()) }));
    assert_eq!(command("/banreason"), Some(Cmd::Reason { add: true, text: None }));
    assert_eq!(command("/unbanreason x"), Some(Cmd::Reason { add: false, text: Some("x".into()) }));
    assert!(matches!(command("/unbanreason"), Some(Cmd::Usage(_))));
  }

  #[test]
  fn rules() {
    let add =
      |a: &str, t: &str, v: &str| Some(Cmd::Rule(RuleCmd::Add { action: a.into(), ty: t.into(), value: v.into() }));
    assert_eq!(command("/rule"), Some(Cmd::Rule(RuleCmd::List)));
    assert_eq!(command("/rule list"), Some(Cmd::Rule(RuleCmd::List)));
    assert_eq!(command("/rule add hide keyword big spoiler"), add("hide", "keyword", "big spoiler"));
    assert_eq!(command("/rule ADD Highlight expr user:bob && first"), add("highlight", "expr", "user:bob && first"));
    assert_eq!(command("/rule del 2"), Some(Cmd::Rule(RuleCmd::Del("2".into()))));
    assert_eq!(command("/rule off 1"), Some(Cmd::Rule(RuleCmd::Enable("1".into(), false))));
    assert_eq!(command("/rule on abc"), Some(Cmd::Rule(RuleCmd::Enable("abc".into(), true))));
    assert_eq!(command("/rule up 3"), Some(Cmd::Rule(RuleCmd::Shift("3".into(), true))));
    let edit = |k: &str, a: &str, t: &str, v: &str| {
      Some(Cmd::Rule(RuleCmd::Edit { key: k.into(), action: a.into(), ty: t.into(), value: v.into() }))
    };
    assert_eq!(command("/rule edit 2 hide spoilers"), edit("2", "hide", "keyword", "spoilers"));
    assert_eq!(command("/rule EDIT abc highlight user bob"), edit("abc", "highlight", "user", "bob"));
    assert_eq!(command("/rule notify 1 on"), Some(Cmd::Rule(RuleCmd::Notify("1".into(), true))));
    assert_eq!(command("/rule notify 1 off"), Some(Cmd::Rule(RuleCmd::Notify("1".into(), false))));
    assert_eq!(command("/rule sound 3 chime"), Some(Cmd::Rule(RuleCmd::Sound("3".into(), "chime".into()))));
    assert_eq!(
      command("/rule sound 3 ~/My Sounds/ding.wav"),
      Some(Cmd::Rule(RuleCmd::Sound("3".into(), "~/My Sounds/ding.wav".into())))
    );
    assert_eq!(command("/rule volume 3 40"), Some(Cmd::Rule(RuleCmd::Volume("3".into(), Some(40)))));
    assert_eq!(command("/rule volume 3 off"), Some(Cmd::Rule(RuleCmd::Volume("3".into(), None))));
    for bad in [
      "/rule edit",
      "/rule edit 2",
      "/rule edit 2 hide",
      "/rule edit 2 hide user",
      "/rule notify 1",
      "/rule notify 1 maybe",
      "/rule sound 3",
      "/rule volume 3 101",
      "/rule volume 3 -1",
      "/rule volume 3 loud",
      "/rule volume",
    ] {
      assert!(matches!(command(bad), Some(Cmd::Usage(_))), "{bad}");
    }
    assert_eq!(command("/rule down 3"), Some(Cmd::Rule(RuleCmd::Shift("3".into(), false))));
    // no kind named = a keyword; a kind word with nothing after it is still refused
    assert_eq!(command("/rule add hide big spoiler"), add("hide", "keyword", "big spoiler"));
    assert_eq!(command("/rule add highlight spoilers"), add("highlight", "keyword", "spoilers"));
    for bad in [
      "/rule add",
      "/rule add hide",
      "/rule add hide user",
      "/rule add hide regex",
      "/rule add hide keyword",
      "/rule del",
      "/rule del 1 2",
      "/rule frob",
      "/rule list x",
    ] {
      assert!(matches!(command(bad), Some(Cmd::Usage(_))), "{bad}");
    }
  }

  #[test]
  fn mutes() {
    assert_eq!(command("/mute @Bob"), Some(Cmd::Mute("bob".into(), Span::Default)));
    assert_eq!(command("/mute bob 2h"), Some(Cmd::Mute("bob".into(), Span::For(7_200_000))));
    assert_eq!(command("/mute bob perm"), Some(Cmd::Mute("bob".into(), Span::Forever)));
    assert!(matches!(command("/mute bob sometime"), Some(Cmd::Usage(_))));
    assert!(matches!(command("/mute bob 9999d"), Some(Cmd::Usage(_))));
    assert!(matches!(command("/mute"), Some(Cmd::Usage(_))));
    assert_eq!(command("/unmute bob"), Some(Cmd::Unmute("bob".into())));
  }

  #[test]
  fn blocks_and_hides() {
    assert_eq!(command("/block Troll"), Some(Cmd::Block("troll".into())));
    assert_eq!(command("/unblock troll"), Some(Cmd::Unblock("troll".into())));
    assert_eq!(command("/hide troll"), Some(Cmd::Hide("troll".into(), true)));
    assert_eq!(command("/unhide troll"), Some(Cmd::Hide("troll".into(), false)));
  }

  #[test]
  fn the_old_word_and_pattern_commands_are_gone() {
    for c in ["/hideword x", "/unhideword x", "/highlight x", "/unhighlight x", "/automod x", "/unautomod x"] {
      assert_eq!(command(c), None, "{c}");
    }
  }
}
