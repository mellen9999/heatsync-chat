// the channel commands heatsync's server runs for twitch (ext input.js): /announce [color] <text> (and
// /announceblue ...), /vip /unvip /mod /unmod <user>, /color <name|#hex>, /poll and /prediction
// "title | a | b [| secs]", /endpoll and the prediction verbs. announce is for mods; roles, polls and
// predictions are the broadcaster's. heatsync's server is the real check
use super::ident;
use super::modsuite::login;

pub const MAX_ANNOUNCE: usize = 500;
pub const MAX_POLL_TITLE: usize = 60;
pub const MAX_PRED_TITLE: usize = 45;
pub const MAX_CHOICE: usize = 25;
pub const MAX_CHOICES: usize = 5;
pub const MAX_OUTCOMES: usize = 10;
pub const DEFAULT_SECS: u32 = 120;
pub const MAX_SECS: u32 = 1800;

pub const ANNOUNCE_COLORS: [&str; 5] = ["primary", "blue", "green", "orange", "purple"];

// twitch's fifteen free chat colors, as the server wants them
const NAMED: [&str; 15] = [
  "blue",
  "blue_violet",
  "cadet_blue",
  "chocolate",
  "coral",
  "dodger_blue",
  "firebrick",
  "golden_rod",
  "green",
  "hot_pink",
  "orange_red",
  "red",
  "sea_green",
  "spring_green",
  "yellow_green",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
  Announce { color: &'static str, text: String },
  Vip(Box<str>, bool),
  Mod(Box<str>, bool),
  // a twitch name (blue_violet) or #rrggbb
  Color(String),
  Poll { title: String, choices: Vec<String>, secs: u32 },
  EndPoll,
  Prediction { title: String, outcomes: Vec<String>, secs: u32 },
  Lock,
  // the winner's place in the prediction's outcomes, from 1
  Resolve(usize),
  Cancel,
}

impl Cmd {
  // the twitch permission pack that lets heatsync's server do it
  pub fn pack(&self) -> &'static str {
    match self {
      Cmd::Announce { .. } => "announce",
      Cmd::Color(_) => "chatcolor",
      _ => "channeltools",
    }
  }

  // runs in no particular room (your color is yours)
  pub fn roomless(&self) -> bool {
    matches!(self, Cmd::Color(_))
  }
}

// "a | b | c | 60s" -> title, items, secs. a last number is the time only after a title and two items,
// so a numeric last choice stays a choice (the ext's rule)
fn pieces(arg: &str) -> (String, Vec<String>, Option<u32>) {
  let mut segs: Vec<String> = arg.split('|').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
  let mut secs = None;
  if segs.len() >= 4 {
    let last = segs[segs.len() - 1].trim_end_matches(['s', 'S']);
    if !last.is_empty()
      && last.len() <= 6
      && let Ok(n) = last.parse::<u32>()
    {
      secs = Some(n);
      segs.pop();
    }
  }
  let title = if segs.is_empty() { String::new() } else { segs.remove(0) };
  (title, segs, secs)
}

fn clean(s: &str) -> bool {
  !s.chars().any(char::is_control)
}

fn color(arg: &str) -> Option<String> {
  let low = arg.trim().to_ascii_lowercase();
  let key: String = low.chars().filter(|c| !matches!(c, ' ' | '_' | '-')).collect();
  if let Some(n) = NAMED.iter().find(|n| n.replace('_', "") == key) {
    return Some((*n).to_owned());
  }
  let hex = low.strip_prefix('#').unwrap_or(&low);
  if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
    return None;
  }
  match hex.len() {
    6 => Some(format!("#{hex}")),
    3 => Some(format!("#{}", hex.chars().flat_map(|c| [c, c]).collect::<String>())),
    _ => None,
  }
}

// Some(Ok) / Some(Err(usage)) for ours, None for anything else
pub fn command(text: &str) -> Option<Result<Cmd, &'static str>> {
  let rest = text.trim().strip_prefix('/')?;
  let (word, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  let arg = arg.trim();
  let word = word.to_ascii_lowercase();
  Some(match word.as_str() {
    "announce" | "announceblue" | "announcegreen" | "announceorange" | "announcepurple" => {
      let named = |w: &str| ANNOUNCE_COLORS.iter().copied().find(|c| *c == w.to_ascii_lowercase());
      let suffix = word.strip_prefix("announce").filter(|s| !s.is_empty()).and_then(named);
      // `/announce blue` alone is the word "blue"
      let lead = arg.split_once(char::is_whitespace).and_then(|(first, tail)| Some((named(first)?, tail.trim())));
      let (color, body) = match (suffix, lead) {
        (Some(c), _) => (c, arg),
        (None, Some((c, tail))) if !tail.is_empty() => (c, tail),
        _ => ("primary", arg),
      };
      if body.is_empty() || !clean(body) {
        Err("usage: /announce [primary|blue|green|orange|purple] <message>")
      } else if body.chars().count() > MAX_ANNOUNCE {
        Err("announcements are 500 characters at most")
      } else {
        Ok(Cmd::Announce { color, text: body.to_owned() })
      }
    }
    "vip" | "unvip" | "mod" | "unmod" => {
      let usage = match word.as_str() {
        "vip" => "usage: /vip <user>",
        "unvip" => "usage: /unvip <user>",
        "mod" => "usage: /mod <user>",
        _ => "usage: /unmod <user>",
      };
      match login(arg) {
        Some(u) if !arg.contains(char::is_whitespace) => {
          let add = !word.starts_with("un");
          Ok(if word.ends_with("vip") { Cmd::Vip(u, add) } else { Cmd::Mod(u, add) })
        }
        _ => Err(usage),
      }
    }
    "color" => color(arg).map(Cmd::Color).ok_or("usage: /color <name|#hex>, like /color coral or /color #ff8800"),
    "poll" | "prediction" => {
      let poll = word == "poll";
      let usage = if poll {
        "usage: /poll title | choice | choice [| up to 5] [| seconds]"
      } else {
        "usage: /prediction title | outcome | outcome [| up to 10] [| seconds]"
      };
      let (title, items, secs) = pieces(arg);
      let (max_title, max_items, min_secs) =
        if poll { (MAX_POLL_TITLE, MAX_CHOICES, 15) } else { (MAX_PRED_TITLE, MAX_OUTCOMES, 30) };
      if title.is_empty() || items.len() < 2 || items.len() > max_items || !clean(arg) {
        Err(usage)
      } else if title.chars().count() > max_title {
        Err(if poll { "a poll title is 60 characters at most" } else { "a prediction title is 45 characters at most" })
      } else if items.iter().any(|i| i.chars().count() > MAX_CHOICE) {
        Err("each choice is 25 characters at most")
      } else {
        let secs = secs.unwrap_or(DEFAULT_SECS).clamp(min_secs, MAX_SECS);
        Ok(if poll {
          Cmd::Poll { title, choices: items, secs }
        } else {
          Cmd::Prediction { title, outcomes: items, secs }
        })
      }
    }
    "endpoll" => Ok(Cmd::EndPoll),
    "lockprediction" | "lockpred" => Ok(Cmd::Lock),
    "cancelprediction" | "cancelpred" => Ok(Cmd::Cancel),
    "resolveprediction" | "resolvepred" => match arg.parse::<usize>() {
      Ok(n) if (1..=MAX_OUTCOMES).contains(&n) => Ok(Cmd::Resolve(n)),
      _ => Err("usage: /resolveprediction <n>, the winning outcome's number"),
    },
    _ => return None,
  })
}

// a hostile string made safe to show: no control characters, trimmed, at most `max` characters
pub fn tidy(s: &str, max: usize) -> Box<str> {
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().into()
}

// an id twitch gave us, before it goes back in a request
pub fn id_ok(s: &str) -> bool {
  ident::slug(s, 64)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ok(s: &str) -> Cmd {
    command(s).unwrap().unwrap()
  }
  fn bad(s: &str) -> &'static str {
    command(s).unwrap().unwrap_err()
  }

  #[test]
  fn announce_takes_a_color_word_or_a_suffix() {
    assert_eq!(ok("/announce hello all"), Cmd::Announce { color: "primary", text: "hello all".into() });
    assert_eq!(ok("/announce Blue hello all"), Cmd::Announce { color: "blue", text: "hello all".into() });
    assert_eq!(ok("/announcegreen go go"), Cmd::Announce { color: "green", text: "go go".into() });
    assert_eq!(ok("/announce blue"), Cmd::Announce { color: "primary", text: "blue".into() });
    assert_eq!(ok("/announce primary x"), Cmd::Announce { color: "primary", text: "x".into() });
    assert!(bad("/announce").starts_with("usage: /announce"));
    assert!(bad("/announceblue  ").starts_with("usage: /announce"));
    assert!(bad(&format!("/announce {}", "x".repeat(501))).contains("500"));
    assert!(command(&format!("/announce {}", "x".repeat(500))).unwrap().is_ok());
  }

  #[test]
  fn roles_need_one_login() {
    assert_eq!(ok("/vip @Bob"), Cmd::Vip("bob".into(), true));
    assert_eq!(ok("/unvip bob"), Cmd::Vip("bob".into(), false));
    assert_eq!(ok("/MOD bob"), Cmd::Mod("bob".into(), true));
    assert_eq!(ok("/unmod bob"), Cmd::Mod("bob".into(), false));
    assert_eq!(bad("/vip"), "usage: /vip <user>");
    assert_eq!(bad("/unmod a b"), "usage: /unmod <user>");
    assert_eq!(bad("/mod b.o.b"), "usage: /mod <user>");
  }

  #[test]
  fn color_reads_names_and_hex() {
    assert_eq!(ok("/color Coral"), Cmd::Color("coral".into()));
    assert_eq!(ok("/color orangered"), Cmd::Color("orange_red".into()));
    assert_eq!(ok("/color blue violet"), Cmd::Color("blue_violet".into()));
    assert_eq!(ok("/color #FF8800"), Cmd::Color("#ff8800".into()));
    assert_eq!(ok("/color f80"), Cmd::Color("#ff8800".into()));
    assert!(bad("/color").starts_with("usage: /color"));
    assert!(bad("/color mauve").starts_with("usage: /color"));
    assert!(bad("/color #12345").starts_with("usage: /color"));
  }

  #[test]
  fn poll_and_prediction_parse() {
    assert_eq!(
      ok("/poll best | a | b"),
      Cmd::Poll { title: "best".into(), choices: vec!["a".into(), "b".into()], secs: 120 }
    );
    assert_eq!(
      ok("/poll best | a | b | c | 60s"),
      Cmd::Poll { title: "best".into(), choices: vec!["a".into(), "b".into(), "c".into()], secs: 60 }
    );
    // too short / too long clamp into twitch's range
    assert!(matches!(ok("/poll t | a | b | 5"), Cmd::Poll { secs: 15, .. }));
    assert!(matches!(ok("/poll t | a | b | 99999"), Cmd::Poll { secs: 1800, .. }));
    // a number last with only two choices is a choice
    assert!(matches!(ok("/poll t | a | 2"), Cmd::Poll { ref choices, secs: 120, .. } if choices == &["a", "2"]));
    assert!(matches!(ok("/prediction t | a | b | 10"), Cmd::Prediction { secs: 30, .. }));
    assert!(
      matches!(ok("/prediction t | a | b | c | d | e | f | g | h | i | j"), Cmd::Prediction { ref outcomes, .. } if outcomes.len() == 10)
    );
    assert!(bad("/poll").starts_with("usage: /poll"));
    assert!(bad("/poll t | a").starts_with("usage: /poll"));
    assert!(bad("/poll t | a | b | c | d | e | f").starts_with("usage: /poll"));
    assert!(bad("/prediction t | a | b | c | d | e | f | g | h | i | j | k").starts_with("usage: /prediction"));
    assert!(bad(&format!("/poll {} | a | b", "t".repeat(61))).contains("60"));
    assert!(bad(&format!("/prediction {} | a | b", "t".repeat(46))).contains("45"));
    assert!(bad(&format!("/poll t | {} | b", "a".repeat(26))).contains("25"));
  }

  #[test]
  fn prediction_verbs() {
    assert_eq!(ok("/endpoll"), Cmd::EndPoll);
    assert_eq!(ok("/lockprediction"), Cmd::Lock);
    assert_eq!(ok("/cancelpred"), Cmd::Cancel);
    assert_eq!(ok("/resolveprediction 2"), Cmd::Resolve(2));
    assert!(bad("/resolveprediction").starts_with("usage: /resolveprediction"));
    assert!(bad("/resolveprediction 0").starts_with("usage: /resolveprediction"));
    assert!(bad("/resolveprediction 11").starts_with("usage: /resolveprediction"));
    assert!(command("/polls x").is_none() && command("/modtools").is_none() && command("vip bob").is_none());
  }

  #[test]
  fn packs_follow_the_server() {
    assert_eq!(ok("/announce x").pack(), "announce");
    assert_eq!(ok("/color red").pack(), "chatcolor");
    assert_eq!(ok("/vip bob").pack(), "channeltools");
    assert_eq!(ok("/endpoll").pack(), "channeltools");
    assert!(ok("/color red").roomless() && !ok("/vip bob").roomless());
  }

  #[test]
  fn hostile_text_is_tamed() {
    assert_eq!(&*tidy("a\u{0}b\nc  ", 10), "a b c");
    assert_eq!(tidy(&"é".repeat(500), 120).chars().count(), 120);
    assert!(id_ok("abc-1_2") && !id_ok("a b") && !id_ok("") && !id_ok(&"a".repeat(65)));
  }
}
