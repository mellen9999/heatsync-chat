// slash commands for the chat filters, ext names where the ext has them (/mute /unmute /block /hide
// /unhide) and plain switches in the style of /vi and /glow for the rest:
//   /mute <user> [10m|2h|3d|perm]   /unmute <user>   (a day by default, like the ext)
//   /block <user> (toggles)   /unblock <user>
//   /hide <user>   /unhide <user>             this tab only, until restart
//   /hideword [term]  /unhideword <term>      hide lines containing a term; no term lists them
//   /highlight [term] /unhighlight <term>     light up lines containing a whole word
//   /automod [regex]  /unautomod <regex>      hide lines matching a pattern (the ext's filter regex); no arg lists
//   /rule [list]                              the per-rule hide / highlight list, in the order they win
//   /rule add <hide|highlight> <keyword|regex|user|badge|msgtype|expr> <value>
//   /rule del|on|off|up|down <number|id>
//   /mutes  /blocks                           who is muted / blocked
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
  // marks (highlight) or hide list, add or remove, and the term (None = list them)
  Word { marks: bool, add: bool, term: Option<String> },
  // an automod pattern: add or remove, and the pattern (None = list them)
  Pattern { add: bool, pat: Option<String> },
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
  Del(String),
  Enable(String, bool),
  Shift(String, bool),
}

const RULE_USAGE: &str = "usage: /rule [list] | add <hide|highlight> <keyword|regex|user|badge|msgtype|expr> <value> | del|on|off|up|down <number>";

fn rule(rest: &str) -> Cmd {
  let (verb, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  let tail = tail.trim();
  let key = || tail.split_whitespace().next().filter(|_| tail.split_whitespace().count() == 1).map(str::to_owned);
  let r = match verb.to_ascii_lowercase().as_str() {
    "" | "list" if tail.is_empty() => RuleCmd::List,
    "add" => {
      let mut p = tail.splitn(3, char::is_whitespace);
      match (p.next(), p.next(), p.next().map(str::trim)) {
        (Some(a), Some(t), Some(v)) if !v.is_empty() => {
          RuleCmd::Add { action: a.to_ascii_lowercase(), ty: t.to_ascii_lowercase(), value: v.to_owned() }
        }
        _ => return Cmd::Usage(RULE_USAGE),
      }
    }
    "del" | "rm" | "remove" => match key() {
      Some(k) => RuleCmd::Del(k),
      None => return Cmd::Usage(RULE_USAGE),
    },
    v @ ("on" | "off" | "up" | "down") => match key() {
      Some(k) => match v {
        "on" | "off" => RuleCmd::Enable(k, v == "on"),
        _ => RuleCmd::Shift(k, v == "up"),
      },
      None => return Cmd::Usage(RULE_USAGE),
    },
    _ => return Cmd::Usage(RULE_USAGE),
  };
  Cmd::Rule(r)
}

// the longest mute we accept, in seconds: a year
const MAX_SPAN: u64 = 365 * 86400;

pub fn command(text: &str) -> Option<Cmd> {
  let t = text.trim();
  let (cmd, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
  let rest = rest.trim();
  let cmd = cmd.strip_prefix('/')?.to_ascii_lowercase();
  let (first, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  let user = |usage: &'static str, f: &dyn Fn(String) -> Cmd| name(first).map_or(Cmd::Usage(usage), f);
  let term = (!rest.is_empty()).then(|| rest.to_owned());
  Some(match cmd.as_str() {
    "mute" => match name(first) {
      None => Cmd::Usage("usage: /mute <user> [10m|2h|3d|perm]"),
      Some(u) => match tail.trim() {
        "" => Cmd::Mute(u, Span::Default),
        "perm" | "forever" => Cmd::Mute(u, Span::Forever),
        l => match span(l).filter(|&s| s > 0 && s <= MAX_SPAN) {
          Some(s) => Cmd::Mute(u, Span::For(s as i64 * 1000)),
          None => Cmd::Usage("usage: /mute <user> [10m|2h|3d|perm]"),
        },
      },
    },
    "unmute" => user("usage: /unmute <user>", &Cmd::Unmute),
    "block" => user("usage: /block <user> (toggles)", &Cmd::Block),
    "unblock" => user("usage: /unblock <user>", &Cmd::Unblock),
    "hide" => user("usage: /hide <user> (this tab only)", &|u| Cmd::Hide(u, true)),
    "unhide" => user("usage: /unhide <user>", &|u| Cmd::Hide(u, false)),
    "hideword" => Cmd::Word { marks: false, add: true, term },
    "unhideword" if term.is_some() => Cmd::Word { marks: false, add: false, term },
    "highlight" => Cmd::Word { marks: true, add: true, term },
    "unhighlight" if term.is_some() => Cmd::Word { marks: true, add: false, term },
    "unhideword" => Cmd::Usage("usage: /unhideword <term>"),
    "unhighlight" => Cmd::Usage("usage: /unhighlight <term>"),
    "automod" => Cmd::Pattern { add: true, pat: term },
    "unautomod" if term.is_some() => Cmd::Pattern { add: false, pat: term },
    "unautomod" => Cmd::Usage("usage: /unautomod <pattern>"),
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
  fn patterns_keep_spaces() {
    assert_eq!(command("/automod buy \\d+ now"), Some(Cmd::Pattern { add: true, pat: Some("buy \\d+ now".into()) }));
    assert_eq!(command("/automod"), Some(Cmd::Pattern { add: true, pat: None }));
    assert_eq!(command("/unautomod x"), Some(Cmd::Pattern { add: false, pat: Some("x".into()) }));
    assert!(matches!(command("/unautomod"), Some(Cmd::Usage(_))));
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
    assert_eq!(command("/rule down 3"), Some(Cmd::Rule(RuleCmd::Shift("3".into(), false))));
    for bad in [
      "/rule add",
      "/rule add hide",
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
  fn words_keep_spaces() {
    assert_eq!(
      command("/hideword big spoiler"),
      Some(Cmd::Word { marks: false, add: true, term: Some("big spoiler".into()) })
    );
    assert_eq!(command("/highlight"), Some(Cmd::Word { marks: true, add: true, term: None }));
    assert_eq!(command("/unhighlight gg"), Some(Cmd::Word { marks: true, add: false, term: Some("gg".into()) }));
    assert!(matches!(command("/unhideword"), Some(Cmd::Usage(_))));
  }
}
