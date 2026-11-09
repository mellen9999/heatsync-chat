// mention rules (ext multichat/mention-rules.js, server routes/mention-rules.ts): words, phrases or regexes
// the SERVER watches for across every channel; a hit comes back as a `mention:rule-match` frame and is a row
// of the mentions tab like any ping. the list is the account's: read with GET /api/mention-rules, changed
// with POST / PUT {enabled} / DELETE, and `mention-rules:changed` says another client changed it. every byte
// that comes back is hostile: counts, lengths and shapes are held to the server's own bounds here
use crate::i18n::t;
use serde_json::{Value, json};

use super::message::Platform;

// the server's caps: 5 rules free, 50 plus, and 100 for the old flat cap; never trust a longer list
pub const MAX_RULES: usize = 100;
pub const MAX_PATTERN: usize = 200;
const MAX_CHANNELS: usize = 50;
const MAX_CHANNEL: usize = 60;
const MAX_USER: usize = 100;
const MAX_SNIPPET: usize = 300;
const MAX_SAID: usize = 120;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
  Word,
  Phrase,
  Regex,
}

impl Ty {
  pub fn name(self) -> &'static str {
    match self {
      Ty::Word => "word",
      Ty::Phrase => "phrase",
      Ty::Regex => "regex",
    }
  }

  fn parse(s: &str) -> Option<Ty> {
    match s {
      "word" => Some(Ty::Word),
      "phrase" => Some(Ty::Phrase),
      "regex" => Some(Ty::Regex),
      _ => None,
    }
  }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rule {
  pub id: u64,
  pub ty: Ty,
  pub pattern: Box<str>,
  // lowercase channel names; empty = every channel
  pub channels: Vec<Box<str>>,
  // empty = every platform
  pub platforms: Vec<Platform>,
  pub on: bool,
  // over the account's cap: kept by the server, not firing
  pub dormant: bool,
}

// a `mention:rule-match`: who said what where, for a rule of yours
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hit {
  pub pattern: Box<str>,
  pub channel: Box<str>,
  pub platform: Platform,
  pub user: Box<str>,
  pub snippet: Box<str>,
}

// the body of a new rule
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Body {
  pub ty: Ty,
  pub pattern: String,
  pub channels: Vec<String>,
}

// control bytes become spaces, the ends are trimmed, at most `max` characters
fn clean(s: &str, max: usize) -> String {
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().to_owned()
}

// a room name as the server keeps them: lowercase, no `#`, nothing that could be a space or a control
fn channel(s: &str) -> Option<Box<str>> {
  let c = s.trim().trim_start_matches('#').to_lowercase();
  let ok = !c.is_empty()
    && c.chars().count() <= MAX_CHANNEL
    && c.chars().all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '@'));
  ok.then(|| c.into())
}

fn platform(s: &str) -> Option<Platform> {
  match s {
    "twitch" => Some(Platform::Twitch),
    "kick" => Some(Platform::Kick),
    "youtube" => Some(Platform::Youtube),
    _ => None,
  }
}

fn letter(p: Platform) -> &'static str {
  match p {
    Platform::Twitch => "t",
    Platform::Kick => "k",
    Platform::Youtube => "y",
  }
}

fn rule(v: &Value) -> Option<Rule> {
  let id = v.get("id")?.as_u64().filter(|&i| i > 0)?;
  let ty = Ty::parse(v.get("rule_type")?.as_str()?)?;
  let pattern = clean(v.get("pattern")?.as_str().filter(|p| p.chars().count() <= MAX_PATTERN)?, MAX_PATTERN);
  if pattern.is_empty() {
    return None;
  }
  let mut channels: Vec<Box<str>> = Vec::new();
  for c in v.get("channels").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
    if let Some(c) = channel(c).filter(|c| !channels.contains(c)) {
      channels.push(c);
    }
    if channels.len() >= MAX_CHANNELS {
      break;
    }
  }
  let mut platforms = Vec::new();
  for p in v.get("platforms").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
    if let Some(p) = platform(p).filter(|p| !platforms.contains(p)) {
      platforms.push(p);
    }
  }
  let flag = |k: &str| v.get(k).and_then(Value::as_bool) == Some(true);
  Some(Rule { id, ty, pattern: pattern.into(), channels, platforms, on: flag("enabled"), dormant: flag("dormant") })
}

// GET /api/mention-rules: `{rules: [...]}`. None = not a list (the caller keeps what it had)
pub fn list(v: &Value) -> Option<Vec<Rule>> {
  let rows = v.get("rules")?.as_array()?;
  Some(rows.iter().take(MAX_RULES).filter_map(rule).collect())
}

// `mention:rule-match {data: {ruleId, pattern, channel, platform, username, snippet, url?}}` (the server's
// frame; a flat one is read too)
pub fn hit(v: &Value) -> Option<Hit> {
  let b = v.get("data").filter(|d| d.is_object()).unwrap_or(v);
  let s = |k: &str| b.get(k).and_then(Value::as_str);
  let snippet = clean(s("snippet")?, MAX_SNIPPET);
  let user = clean(s("username")?, MAX_USER);
  if snippet.is_empty() || user.is_empty() {
    return None;
  }
  Some(Hit {
    pattern: clean(s("pattern").unwrap_or(""), MAX_PATTERN).into(),
    channel: channel(s("channel")?)?,
    platform: platform(s("platform")?)?,
    user: user.into(),
    snippet: snippet.into(),
  })
}

// a channel token typed after the pattern: `#name`
fn chan_token(t: &str) -> Option<&str> {
  let c = t.strip_prefix('#')?;
  (!c.is_empty() && c.chars().all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-'))).then_some(c)
}

// what is typed on the add row: `[word|phrase|regex] <pattern> [#channel ...]`. more than one word is a phrase,
// one is a word; trailing `#channel`s narrow the rule to those rooms
pub fn parse_add(text: &str) -> Result<Body, &'static str> {
  let mut rest = clean(text, text.len().min(4 * MAX_PATTERN + 1000));
  let mut channels: Vec<String> = Vec::new();
  loop {
    let (head, last) = rest.rsplit_once(char::is_whitespace).unwrap_or(("", rest.as_str()));
    let Some(c) = chan_token(last) else { break };
    let (c, head) = (c.to_lowercase(), head.trim_end().to_owned());
    channels.push(c);
    rest = head;
  }
  // peeled from the end: back in typed order, a repeat folded into its first
  channels.reverse();
  let mut seen = Vec::new();
  channels.retain(|c| {
    !seen.contains(c) && {
      seen.push(c.clone());
      true
    }
  });
  let (ty, pattern) = match rest.split_once(char::is_whitespace) {
    Some((kw, p)) if Ty::parse(kw.to_lowercase().as_str()).is_some() && !p.trim().is_empty() => {
      (Ty::parse(&kw.to_lowercase()).unwrap_or(Ty::Word), p.trim().to_owned())
    }
    Some(_) => (Ty::Phrase, rest.clone()),
    None => (Ty::Word, rest.clone()),
  };
  if pattern.is_empty() {
    return Err(t("mod.mrules.empty"));
  }
  if pattern.chars().count() > MAX_PATTERN {
    return Err(t("mod.mrules.too_long"));
  }
  if channels.len() > MAX_CHANNELS {
    return Err(t("mod.mrules.too_many_channels"));
  }
  if channels.iter().any(|c| c.chars().count() > MAX_CHANNEL) {
    return Err(t("mod.mrules.channel_too_long"));
  }
  Ok(Body { ty, pattern, channels })
}

impl Body {
  // POST /api/mention-rules. channels null = every channel; `web` is the ext's own delivery (the server's
  // default adds browser push, which this window doesn't want)
  pub fn json(&self) -> Value {
    let channels = if self.channels.is_empty() { Value::Null } else { json!(self.channels) };
    json!({ "rule_type": self.ty.name(), "pattern": self.pattern, "channels": channels, "notify_via": ["web"] })
  }
}

// the server's `error` for a refusal, short and plain
pub fn said(v: &Value) -> Option<String> {
  let e = clean(v.get("error")?.as_str()?, MAX_SAID);
  (!e.is_empty()).then_some(e)
}

// one row for a rule in the list: its pattern, and what kind / where / whether it fires
pub fn describe(r: &Rule) -> (String, String) {
  let mut d = vec![r.ty.name().to_owned()];
  if !r.channels.is_empty() {
    d.push(r.channels.iter().map(|c| format!("#{c}")).collect::<Vec<_>>().join(" "));
  } else if r.platforms.is_empty() {
    d.push(t("mod.mrules.everywhere").into());
  }
  if !r.platforms.is_empty() {
    d.push(r.platforms.iter().map(|p| letter(*p)).collect::<Vec<_>>().join(" "));
  }
  if !r.on {
    d.push(t("mod.mrules.off").into());
  } else if r.dormant {
    d.push(t("mod.mrules.dormant").into());
  }
  (r.pattern.to_string(), d.join(" · "))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn rule(id: u64) -> Value {
    json!({
      "id": id, "rule_type": "word", "pattern": "gg", "channels": null, "platforms": null,
      "case_sensitive": false, "enabled": true, "notify_via": ["web"], "cooldown_seconds": 60,
      "last_matched_at": null, "match_count": 3, "dormant": false,
    })
  }

  #[test]
  fn the_servers_list_is_read() {
    let mut a = rule(7);
    a["channels"] = json!(["Forsen", "#xqc"]);
    a["platforms"] = json!(["twitch", "kick"]);
    a["enabled"] = json!(false);
    let mut b = rule(9);
    b["rule_type"] = json!("regex");
    b["pattern"] = json!("^hi");
    b["dormant"] = json!(true);
    let got = list(&json!({ "rules": [a, b], "plus": false, "limit": 5 })).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].id, 7);
    assert_eq!(&*got[0].pattern, "gg");
    assert_eq!(got[0].ty, Ty::Word);
    assert_eq!(got[0].channels, ["forsen".into(), "xqc".into()] as [Box<str>; 2]);
    assert_eq!(got[0].platforms, [Platform::Twitch, Platform::Kick]);
    assert!(!got[0].on && !got[0].dormant);
    assert!(got[1].dormant && got[1].on && got[1].ty == Ty::Regex);
    assert!(got[1].channels.is_empty() && got[1].platforms.is_empty());
  }

  #[test]
  fn a_hostile_list_is_cut_down_to_what_is_safe() {
    // not a list at all: nothing, so the caller keeps what it had
    for bad in
      [json!(null), json!([]), json!({}), json!({"rules": 5}), json!({"rules": "x"}), json!({"rules": {"a": 1}})]
    {
      assert!(list(&bad).is_none(), "{bad}");
    }
    assert_eq!(list(&json!({"rules": []})).unwrap().len(), 0);
    // rules without a usable id / type / pattern are skipped, the rest kept
    let mut no_id = rule(1);
    no_id["id"] = json!("x");
    let mut zero = rule(0);
    zero["id"] = json!(0);
    let mut neg = rule(1);
    neg["id"] = json!(-4);
    let mut float = rule(1);
    float["id"] = json!(1.5);
    let mut ty = rule(2);
    ty["rule_type"] = json!("glob");
    let mut empty = rule(3);
    empty["pattern"] = json!("");
    let mut long = rule(4);
    long["pattern"] = json!("x".repeat(MAX_PATTERN + 1));
    let mut num = rule(5);
    num["pattern"] = json!(5);
    let got =
      list(&json!({"rules": [no_id, zero, neg, float, ty, empty, long, num, json!(5), json!(null), rule(6)]})).unwrap();
    assert_eq!(got.iter().map(|r| r.id).collect::<Vec<_>>(), [6]);
    // too many rules: only the first MAX_RULES
    let many: Vec<Value> = (1..=500).map(rule).collect();
    assert_eq!(list(&json!({ "rules": many })).unwrap().len(), MAX_RULES);
    // odd scopes are cleaned: unknown platforms and blank / long / control channels dropped, ≤50 kept
    let mut odd = rule(8);
    odd["platforms"] = json!(["twitch", "myspace", 5, "kick", "twitch"]);
    odd["channels"] = json!(["ok", "", "  ", "x".repeat(MAX_CHANNEL + 1), "bad\u{7}name", 5, "OK"]);
    let r = &list(&json!({"rules": [odd]})).unwrap()[0];
    assert_eq!(r.platforms, [Platform::Twitch, Platform::Kick]);
    assert_eq!(r.channels, ["ok".into()] as [Box<str>; 1]);
    let mut lots = rule(8);
    lots["channels"] = json!((0..200).map(|i| format!("c{i}")).collect::<Vec<_>>());
    assert_eq!(list(&json!({"rules": [lots]})).unwrap()[0].channels.len(), MAX_CHANNELS);
    // channels / platforms of the wrong type mean none
    let mut wrong = rule(8);
    wrong["channels"] = json!("forsen");
    wrong["platforms"] = json!({"a": 1});
    let r = &list(&json!({"rules": [wrong]})).unwrap()[0];
    assert!(r.channels.is_empty() && r.platforms.is_empty());
    // a pattern is shown without control bytes
    let mut ctl = rule(8);
    ctl["pattern"] = json!("a\u{1b}[31mb\nc");
    assert_eq!(&*list(&json!({"rules": [ctl]})).unwrap()[0].pattern, "a [31mb c");
    // enabled that is not a bool counts as off; dormant that is not a bool as awake
    let mut sloppy = rule(8);
    sloppy["enabled"] = json!("yes");
    sloppy["dormant"] = json!("yes");
    let r = &list(&json!({"rules": [sloppy]})).unwrap()[0];
    assert!(!r.on && !r.dormant);
  }

  #[test]
  fn a_rule_match_frame_is_checked() {
    let frame = json!({"type": "mention:rule-match", "data": {
      "ruleId": 7, "pattern": "gg", "channel": "Forsen", "platform": "twitch", "username": "bob",
      "snippet": "gg wp", "url": "https://heatsync.org/x"}});
    let h = hit(&frame).unwrap();
    assert_eq!(
      h,
      Hit {
        pattern: "gg".into(),
        channel: "forsen".into(),
        platform: Platform::Twitch,
        user: "bob".into(),
        snippet: "gg wp".into()
      }
    );
    // flat, as the server's other pushes are
    let flat = json!({"type": "mention:rule-match", "pattern": "gg", "channel": "xqc", "platform": "kick",
      "username": "al", "snippet": "hi"});
    assert_eq!(hit(&flat).unwrap().platform, Platform::Kick);
    // no snippet, no channel, no name, a platform we don't have: nothing to show
    for key in ["snippet", "channel", "username", "platform"] {
      let mut f = frame.clone();
      f["data"].as_object_mut().unwrap().remove(key);
      assert!(hit(&f).is_none(), "{key}");
    }
    let mut f = frame.clone();
    f["data"]["platform"] = json!("myspace");
    assert!(hit(&f).is_none());
    for bad in [json!({"data": 5}), json!({"data": []}), json!(null), json!("x")] {
      assert!(hit(&bad).is_none(), "{bad}");
    }
    // long fields are cut, control bytes become spaces, a blank snippet is nothing
    let mut f = frame.clone();
    f["data"]["snippet"] = json!(format!("{}\u{7}", "y".repeat(5000)));
    f["data"]["username"] = json!("z".repeat(5000));
    f["data"]["pattern"] = json!(7);
    let h = hit(&f).unwrap();
    assert!(h.snippet.chars().count() <= MAX_SNIPPET && h.user.chars().count() <= MAX_USER);
    assert!(h.pattern.is_empty());
    f["data"]["snippet"] = json!("  \u{7} ");
    assert!(hit(&f).is_none());
    // a channel that is not a room name is dropped
    for c in ["a b", "x/../y", "", &"c".repeat(MAX_CHANNEL + 1)] {
      let mut f = frame.clone();
      f["data"]["channel"] = json!(c);
      assert!(hit(&f).is_none(), "{c}");
    }
  }

  #[test]
  fn a_typed_rule_becomes_a_body() {
    let ok = |t: &str| parse_add(t).unwrap();
    let b = ok("gg");
    assert_eq!((b.ty, b.pattern.as_str(), b.channels.len()), (Ty::Word, "gg", 0));
    // more than one word is a phrase
    assert_eq!(ok("big news").ty, Ty::Phrase);
    // the first word can say which
    assert_eq!(ok("word hello").ty, Ty::Word);
    assert_eq!((ok("phrase hi").ty, ok("phrase hi").pattern.as_str()), (Ty::Phrase, "hi"));
    let r = ok("regex ^hi.*there$");
    assert_eq!((r.ty, r.pattern.as_str()), (Ty::Regex, "^hi.*there$"));
    // a bare "word" / "regex" is itself a word
    assert_eq!((ok("regex").ty, ok("regex").pattern.as_str()), (Ty::Word, "regex"));
    // trailing #channels narrow it; # and case are dropped, repeats folded
    let c = ok("gg #Forsen #xqc #forsen");
    assert_eq!(c.pattern, "gg");
    assert_eq!(c.channels, ["forsen", "xqc"]);
    assert_eq!(ok("phrase big news #xqc").pattern, "big news");
    // a # inside the pattern is the pattern's
    assert_eq!(ok("c#").pattern, "c#");
    assert_eq!(ok("##hashtag").pattern, "##hashtag");
    // refused, with the reason in plain words
    for t in ["", "   ", "#forsen", "#a #b"] {
      assert_eq!(parse_add(t), Err("type a word or phrase"), "{t:?}");
    }
    assert_eq!(parse_add(&"x".repeat(MAX_PATTERN + 1)), Err(t("mod.mrules.too_long")));
    let many = (0..51).map(|i| format!("#c{i}")).collect::<Vec<_>>().join(" ");
    assert_eq!(parse_add(&format!("gg {many}")), Err(t("mod.mrules.too_many_channels")));
    assert_eq!(parse_add(&format!("gg #{}", "c".repeat(61))), Err(t("mod.mrules.channel_too_long")));
    // control bytes in a typed rule are spaces
    assert_eq!(ok("a\u{7}b").pattern, "a b");
  }

  #[test]
  fn the_body_is_what_the_server_wants() {
    let b = parse_add("regex ^hi #xqc").unwrap();
    assert_eq!(b.json(), json!({"rule_type": "regex", "pattern": "^hi", "channels": ["xqc"], "notify_via": ["web"]}));
    // no channels: null, every channel (the ext sends null so an edit can clear them)
    assert_eq!(parse_add("gg").unwrap().json()["channels"], Value::Null);
  }

  #[test]
  fn the_servers_refusal_is_one_plain_line() {
    assert_eq!(
      said(&json!({"error": "free accounts run 5 alerts; plus runs 50"})).unwrap(),
      "free accounts run 5 alerts; plus runs 50"
    );
    assert_eq!(said(&json!({"error": "invalid regex: x\u{7}y"})).unwrap(), "invalid regex: x y");
    assert!(said(&json!({"error": "q".repeat(1000)})).unwrap().chars().count() <= MAX_SAID);
    for none in [json!(null), json!({}), json!({"error": 5}), json!({"error": " "}), json!([1])] {
      assert!(said(&none).is_none(), "{none}");
    }
  }

  #[test]
  fn a_row_says_what_where_and_whether() {
    let mut r = list(&json!({"rules": [rule(1)]})).unwrap().remove(0);
    assert_eq!(describe(&r), ("gg".into(), "word · everywhere".into()));
    r.channels = vec!["forsen".into(), "xqc".into()];
    r.platforms = vec![Platform::Twitch, Platform::Kick];
    r.ty = Ty::Regex;
    assert_eq!(describe(&r).1, "regex · #forsen #xqc · t k");
    r.on = false;
    assert_eq!(describe(&r).1, "regex · #forsen #xqc · t k · off");
    r.on = true;
    r.dormant = true;
    assert_eq!(describe(&r).1, "regex · #forsen #xqc · t k · over your limit, not firing");
  }
}
