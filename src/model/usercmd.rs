// your own slash commands (chatterino-style): `/gg` → `good game {1}`. kept one per line, `name expansion`, in
// the `commands` file. the placeholders: {1} {2} … the typed words, {1+} the rest from the nth word on,
// {input} everything after the command, {channel} this tab's channel, {user} your own name. an expansion
// is made exactly once, from what you typed, and the result is never looked at again for placeholders, so
// there is nothing to loop (app/usercmd.rs also refuses a result that starts another of your commands)
use crate::i18n::t;
use crate::store;

pub const MAX_CMDS: usize = 200;
pub const MAX_NAME: usize = 32;
// what a chat line may be (twitch's cap): a longer result is refused, never cut
pub const MAX_EXP: usize = 500;
const FILE: &str = "commands";
// a file past this is not ours: 200 full lines and some room
const MAX_FILE: u64 = (MAX_CMDS * (MAX_NAME + MAX_EXP + 4) + 1024) as u64;

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Cmds {
  list: Vec<(String, String)>,
}

// a name as typed: `/Gg` or `gg`; None when it is not 1-32 of a-z 0-9 _ -
pub fn name(s: &str) -> Option<String> {
  let n = s.strip_prefix('/').unwrap_or(s).to_ascii_lowercase();
  let ok = !n.is_empty()
    && n.len() <= MAX_NAME
    && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
  ok.then_some(n)
}

// one line of text for a file: no control chars, trimmed
fn clean(s: &str) -> String {
  let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
  flat.trim().to_owned()
}

// `/gg good game {1}` → (gg, good game {1})
pub fn entry(s: &str) -> Result<(String, String), &'static str> {
  let s = s.trim();
  let (n, rest) = s.split_once(char::is_whitespace).unwrap_or((s, ""));
  let Some(n) = name(n) else { return Err(t("chat2.usercmd.bad_name")) };
  let exp = clean(rest);
  if exp.is_empty() {
    return Err(t("chat2.usercmd.empty"));
  }
  if exp.chars().count() > MAX_EXP {
    return Err(t("chat2.usercmd.too_long"));
  }
  Ok((n, exp))
}

// the command a line starts with, and what follows it: `/gg  hi there` → (gg, hi there)
pub fn split(line: &str) -> Option<(String, &str)> {
  let rest = line.trim_start().strip_prefix('/')?;
  let (w, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  Some((name(w)?, tail.trim()))
}

impl Cmds {
  pub fn load() -> Self {
    let Some(text) = store::read_capped(FILE, MAX_FILE) else { return Self::default() };
    let (c, junk) = Self::parse(&text);
    if junk > 0 {
      store::set_aside(FILE, Some(junk));
      c.save();
    }
    c
  }

  // the good lines, and how many were not (bad name, no text, a repeat, past the cap)
  pub fn parse(text: &str) -> (Self, usize) {
    let mut c = Self::default();
    let mut junk = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
      match entry(line) {
        Ok((n, e)) if c.list.len() < MAX_CMDS && c.get(&n).is_none() => c.list.push((n, e)),
        _ => junk += 1,
      }
    }
    (c, junk)
  }

  pub fn save(&self) {
    let s: String = self.list.iter().map(|(n, e)| format!("{n} {e}\n")).collect();
    let _ = store::write(FILE, &s);
  }

  pub fn get(&self, name: &str) -> Option<&str> {
    self.list.iter().find(|(n, _)| n == name).map(|(_, e)| &**e)
  }

  pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
    self.list.iter().map(|(n, e)| (&**n, &**e))
  }

  #[cfg(test)]
  pub fn len(&self) -> usize {
    self.list.len()
  }

  #[cfg(test)]
  pub fn is_empty(&self) -> bool {
    self.list.is_empty()
  }

  // a new one (or a new text for a name you have): the name it was kept under
  pub fn add(&mut self, typed: &str) -> Result<String, &'static str> {
    let (n, e) = entry(typed)?;
    if let Some(slot) = self.list.iter_mut().find(|(x, _)| *x == n) {
      slot.1 = e;
    } else if self.list.len() >= MAX_CMDS {
      return Err("200 commands at most: remove one first");
    } else {
      self.list.push((n.clone(), e));
    }
    Ok(n)
  }

  pub fn remove(&mut self, name: &str) -> bool {
    let n = self.list.len();
    self.list.retain(|(x, _)| x != name);
    self.list.len() != n
  }
}

// what the placeholders can name besides the typed words
pub struct Ctx<'a> {
  pub channel: &'a str,
  pub user: Option<&'a str>,
}

// the words of `input` with where each starts, for {n+}
fn words(input: &str) -> Vec<(usize, &str)> {
  let mut out = Vec::new();
  let mut start = None;
  for (i, c) in input.char_indices().chain(std::iter::once((input.len(), ' '))) {
    match (start, c.is_whitespace()) {
      (None, false) => start = Some(i),
      (Some(s), true) => {
        out.push((s, &input[s..i]));
        start = None;
      }
      _ => {}
    }
  }
  out
}

// the text a line of `/name input` becomes, or a plain sentence saying what is missing. one pass over the
// template: what a placeholder puts in is never read for placeholders again, and an unknown `{thing}` stays
pub fn expand(template: &str, name: &str, input: &str, ctx: &Ctx) -> Result<String, String> {
  let ws = words(input);
  let mut out = String::with_capacity(template.len() + input.len());
  let mut rest = template;
  while let Some(open) = rest.find('{') {
    out.push_str(&rest[..open]);
    let after = &rest[open + 1..];
    let Some(close) = after.find('}').filter(|&c| c <= 8) else {
      out.push('{');
      rest = after;
      continue;
    };
    let key = &after[..close];
    let plus = key.strip_suffix('+');
    let num = plus.unwrap_or(key);
    let n = (!num.is_empty() && num.len() <= 3 && num.bytes().all(|b| b.is_ascii_digit()))
      .then(|| num.parse::<usize>().ok())
      .flatten()
      .filter(|&n| n >= 1);
    let value: Option<String> = match (key, n) {
      ("input", _) => {
        if input.is_empty() {
          return Err(format!("/{name} needs something after it: /{name} …"));
        }
        Some(input.to_owned())
      }
      ("channel", _) => {
        if ctx.channel.is_empty() {
          return Err(format!("/{name} needs a channel tab: open one first"));
        }
        Some(ctx.channel.to_owned())
      }
      ("user", _) => match ctx.user {
        Some(u) => Some(u.to_owned()),
        None => return Err(format!("/{name} needs your name: log in first (ctrl-l)")),
      },
      (_, Some(n)) => match (ws.get(n - 1), plus.is_some()) {
        (Some((_, w)), false) => Some((*w).to_owned()),
        (Some((at, _)), true) => Some(input[*at..].to_owned()),
        (None, plus) => {
          let more = if plus { "at least " } else { "" };
          let s = if n == 1 { "" } else { "s" };
          return Err(format!("/{name} needs {more}{n} word{s} after it"));
        }
      },
      _ => None,
    };
    match value {
      Some(v) => out.push_str(&v),
      None => {
        out.push('{');
        out.push_str(key);
        out.push('}');
      }
    }
    rest = &after[close + 1..];
  }
  out.push_str(rest);
  let out = out.trim().to_owned();
  if out.is_empty() {
    return Err(format!("/{name} came out empty"));
  }
  if out.chars().count() > MAX_EXP {
    return Err(format!("/{name} came out over {MAX_EXP} characters: say less"));
  }
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  const CTX: Ctx = Ctx { channel: "forsen", user: Some("me") };

  fn go(t: &str, input: &str) -> Result<String, String> {
    expand(t, "x", input, &CTX)
  }

  #[test]
  fn placeholders_fill_in() {
    assert_eq!(go("hi {1}, {2}!", "bob  amy"), Ok("hi bob, amy!".into()));
    assert_eq!(go("{1} then {2+}", "a b  c d"), Ok("a then b  c d".into()));
    assert_eq!(go("say: {input}", "a  b"), Ok("say: a  b".into()));
    assert_eq!(go("{user} in {channel}", ""), Ok("me in forsen".into()));
    assert_eq!(go("{1+}", "a b"), Ok("a b".into()));
  }

  #[test]
  fn unknown_braces_stay_and_nothing_is_read_twice() {
    assert_eq!(go("{0} {foo} {} { {1", "a"), Ok("{0} {foo} {} { {1".into()));
    // what you typed is data: a placeholder inside it is not filled in
    assert_eq!(go("{1} {2}", "{2} {input}"), Ok("{2} {input}".into()));
    assert_eq!(go("{input}", "{user}"), Ok("{user}".into()));
  }

  #[test]
  fn missing_words_are_a_plain_error() {
    assert_eq!(go("hi {2}", "bob"), Err("/x needs 2 words after it".into()));
    assert_eq!(go("hi {1}", ""), Err("/x needs 1 word after it".into()));
    assert_eq!(go("hi {3+}", "a b"), Err("/x needs at least 3 words after it".into()));
    assert!(go("{input}", "").is_err());
    assert!(expand("{user}", "x", "", &Ctx { channel: "c", user: None }).is_err());
    assert!(expand("{channel}", "x", "", &Ctx { channel: "", user: None }).is_err());
  }

  #[test]
  fn a_result_too_long_or_empty_is_refused() {
    assert!(go("{input}", &"a".repeat(MAX_EXP + 1)).unwrap_err().contains("over 500"));
    assert_eq!(go("{input}", &"a".repeat(MAX_EXP)).map(|s| s.len()), Ok(MAX_EXP));
    assert!(go("   ", "").is_err());
    // a huge number is no index, not a panic
    assert_eq!(go("{99999999999999999999}", "a"), Ok("{99999999999999999999}".into()));
  }

  #[test]
  fn names_and_entries() {
    assert_eq!(name("/GG"), Some("gg".into()));
    assert_eq!(name("a_b-1"), Some("a_b-1".into()));
    assert!(name("").is_none() && name("/").is_none() && name("a b").is_none() && name("é").is_none());
    assert!(name(&"a".repeat(33)).is_none() && name(&"a".repeat(32)).is_some());
    assert_eq!(entry("/gg good game {1}"), Ok(("gg".into(), "good game {1}".into())));
    assert!(entry("/gg").is_err() && entry("/gg   ").is_err() && entry("/bad! x").is_err());
    assert!(entry(&format!("gg {}", "a".repeat(501))).is_err());
    assert_eq!(split("  /Gg  hi there "), Some(("gg".into(), "hi there")));
    assert_eq!(split("/gg"), Some(("gg".into(), "")));
    assert!(split("hi /gg").is_none() && split("/g!g").is_none());
  }

  #[test]
  fn add_replaces_caps_and_removes() {
    let mut c = Cmds::default();
    assert_eq!(c.add("/gg one"), Ok("gg".into()));
    assert_eq!(c.add("gg two"), Ok("gg".into()));
    assert_eq!((c.len(), c.get("gg")), (1, Some("two")));
    for i in 1..MAX_CMDS {
      c.add(&format!("c{i} x")).unwrap();
    }
    assert!(c.add("one-more x").is_err());
    assert!(c.add("gg three").is_ok(), "changing one is fine at the cap");
    assert!(c.remove("gg") && !c.remove("gg"));
    assert!(c.add("one-more x").is_ok());
  }

  #[test]
  fn a_hostile_file_is_cut_down_to_the_good_lines() {
    let mut text = String::from("ok fine {1}\nbad!name x\nnoexpansion\n\nok again\nUP caps\x07bell\n");
    for i in 0..500 {
      text.push_str(&format!("n{i} x\n"));
    }
    let (c, junk) = Cmds::parse(&text);
    assert_eq!(c.len(), MAX_CMDS);
    assert_eq!(c.get("ok"), Some("fine {1}"));
    assert_eq!(c.get("up"), Some("caps bell"));
    assert_eq!(junk, 3 + 500 - (MAX_CMDS - 2));
  }

  #[test]
  fn the_file_round_trips_and_an_oversized_one_is_set_aside() {
    let mut c = Cmds::default();
    c.add("gg good game {1}").unwrap();
    c.save();
    assert_eq!(Cmds::load(), c);
    store::write(FILE, &"x y\n".repeat(MAX_FILE as usize)).unwrap();
    assert!(Cmds::load().is_empty());
  }
}
