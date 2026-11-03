// the ext's per-rule hide / highlight engine (multichat/filter-rules.js, setting chatFilterRules). the setting
// is a JSON array of rules; it is kept here as the ext wrote it (so fields we don't use survive a round trip
// and a rule we can't read stays in the list) and each rule is compiled once. a line meets the rules in
// order: the first hide wins outright, otherwise the first highlight lights the line up. a rule's color and
// sound are kept but not used (the window has one highlight look); a rule scoped to one ext channel tab is
// kept and listed but never applies here (those ids mean nothing in this window)
use std::cell::OnceCell;

use regex::Regex;
use serde_json::{Value, json};

use crate::model::automod::{safe_regex, scan};
use crate::model::message::{Kind, Message, Platform, flag};

// the ext syncs the setting only up to this size
pub const MAX_JSON: usize = 32 * 1024;
pub const MAX_RULES: usize = 200;
const MAX_EXPR: usize = 512;
const MAX_TOKENS: usize = 256;
const MAX_DEPTH: usize = 40;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
  Pass,
  Hide,
  Mark,
}

// a line as the rules look at it: the first SCAN characters, lowercased once if anything needs it
pub struct Line<'a> {
  m: &'a Message,
  text: &'a str,
  low: OnceCell<String>,
}

impl<'a> Line<'a> {
  pub fn new(m: &'a Message) -> Self {
    Line { m, text: scan(&m.body), low: OnceCell::new() }
  }

  fn low(&self) -> &str {
    self.low.get_or_init(|| self.text.to_lowercase())
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
  First,
  Action,
  Reply,
  Cheer,
}

impl Ty {
  fn parse(v: &str, short_first: bool) -> Option<Ty> {
    match v {
      "first-message" => Some(Ty::First),
      "first" if short_first => Some(Ty::First),
      "action" => Some(Ty::Action),
      "reply" => Some(Ty::Reply),
      "cheer" => Some(Ty::Cheer),
      _ => None,
    }
  }

  fn holds(self, m: &Message) -> bool {
    match self {
      Ty::First => m.has(flag::FIRST),
      Ty::Action => m.kind == Kind::Action,
      Ty::Reply => m.reply_to.is_some(),
      // the window keeps no bits count
      Ty::Cheer => false,
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cmp {
  Gt,
  Ge,
  Lt,
  Le,
  Eq,
}

#[derive(Clone, Debug)]
enum Node {
  Or(Box<Node>, Box<Node>),
  And(Box<Node>, Box<Node>),
  Not(Box<Node>),
  Flag(Ty),
  User(String),
  Badge(String),
  Type(Option<Ty>),
  Contains(String),
  Re(Regex),
  Bits(Cmp, f64),
}

fn bare(u: &str) -> &str {
  u.strip_prefix('@').unwrap_or(u)
}

fn has_badge(m: &Message, v: &str) -> bool {
  m.badges.iter().any(|(set, _)| {
    set.eq_ignore_ascii_case(v)
      || (v == "mod" && m.platform == Platform::Youtube && set.eq_ignore_ascii_case("moderator"))
  })
}

fn is_user(m: &Message, v: &str) -> bool {
  !m.login.is_empty() && (bare(&m.login).to_lowercase() == v || bare(&m.nick).to_lowercase() == v)
}

impl Node {
  fn eval(&self, l: &Line) -> bool {
    match self {
      Node::Or(a, b) => a.eval(l) || b.eval(l),
      Node::And(a, b) => a.eval(l) && b.eval(l),
      Node::Not(x) => !x.eval(l),
      Node::Flag(t) => t.holds(l.m),
      Node::User(v) => is_user(l.m, v),
      Node::Badge(v) => has_badge(l.m, v),
      Node::Type(t) => t.is_some_and(|t| t.holds(l.m)),
      Node::Contains(v) => l.low().contains(v.as_str()),
      Node::Re(re) => re.is_match(l.text),
      // the window keeps no bits count: it is always 0
      Node::Bits(c, n) => match c {
        Cmp::Gt => 0.0 > *n,
        Cmp::Ge => 0.0 >= *n,
        Cmp::Lt => 0.0 < *n,
        Cmp::Le => 0.0 <= *n,
        Cmp::Eq => 0.0 == *n,
      },
    }
  }
}

// ---- the boolean expressions (match type "expr"), the ext's grammar:
//   or := and (('||'|'or') and)*    and := unary (('&&'|'and') unary)*    unary := ('!'|'not') unary | primary
//   primary := '(' or ')' | term    term := first|action|reply|cheer | FIELD ':' VALUE | 'bits' OP NUMBER
//   FIELD := user badge type msgtype contains has regex     VALUE := "quoted" | bareword

#[derive(Clone, PartialEq, Debug)]
enum Tok {
  Open,
  Close,
  Bang,
  Colon,
  And,
  Or,
  Op(Cmp),
  Str(String),
  Word(String),
}

fn tokens(src: &str) -> Option<Vec<Tok>> {
  if src.chars().count() > MAX_EXPR {
    return None;
  }
  let c: Vec<char> = src.chars().collect();
  let (mut i, mut out) = (0, Vec::new());
  while i < c.len() {
    let at = |k: usize| c.get(i + k).copied();
    match c[i] {
      ' ' | '\t' | '\n' => i += 1,
      '(' => (out.push(Tok::Open), i += 1).1,
      ')' => (out.push(Tok::Close), i += 1).1,
      '!' => (out.push(Tok::Bang), i += 1).1,
      ':' => (out.push(Tok::Colon), i += 1).1,
      '&' if at(1) == Some('&') => (out.push(Tok::And), i += 2).1,
      '|' if at(1) == Some('|') => (out.push(Tok::Or), i += 2).1,
      '>' | '<' | '=' => {
        let (op, n) = match (c[i], at(1)) {
          ('>', Some('=')) => (Cmp::Ge, 2),
          ('<', Some('=')) => (Cmp::Le, 2),
          ('=', Some('=')) => (Cmp::Eq, 2),
          ('>', _) => (Cmp::Gt, 1),
          ('<', _) => (Cmp::Lt, 1),
          // a lone = is not an operator
          _ => return None,
        };
        out.push(Tok::Op(op));
        i += n;
      }
      '"' => {
        let end = c[i + 1..].iter().position(|&x| x == '"')?;
        out.push(Tok::Str(c[i + 1..i + 1 + end].iter().collect()));
        i += end + 2;
      }
      _ => {
        let w: String = c[i..].iter().take_while(|x| !" \t\n():!\"<>=&|".contains(**x)).collect();
        // a lone & or | cannot start a word
        if w.is_empty() {
          return None;
        }
        i += w.chars().count();
        out.push(Tok::Word(w));
      }
    }
    if out.len() > MAX_TOKENS {
      return None;
    }
  }
  Some(out)
}

struct Parser {
  toks: Vec<Tok>,
  pos: usize,
}

impl Parser {
  fn peek(&self) -> Option<&Tok> {
    self.toks.get(self.pos)
  }

  fn next(&mut self) -> Option<Tok> {
    let t = self.toks.get(self.pos).cloned();
    self.pos += 1;
    t
  }

  fn word(&self, w: &str) -> bool {
    matches!(self.peek(), Some(Tok::Word(x)) if x.eq_ignore_ascii_case(w))
  }

  fn or(&mut self, d: usize) -> Option<Node> {
    if d > MAX_DEPTH {
      return None;
    }
    let mut n = self.and(d + 1)?;
    while self.peek() == Some(&Tok::Or) || self.word("or") {
      self.next();
      n = Node::Or(Box::new(n), Box::new(self.and(d + 1)?));
    }
    Some(n)
  }

  fn and(&mut self, d: usize) -> Option<Node> {
    if d > MAX_DEPTH {
      return None;
    }
    let mut n = self.unary(d + 1)?;
    while self.peek() == Some(&Tok::And) || self.word("and") {
      self.next();
      n = Node::And(Box::new(n), Box::new(self.unary(d + 1)?));
    }
    Some(n)
  }

  fn unary(&mut self, d: usize) -> Option<Node> {
    if d > MAX_DEPTH {
      return None;
    }
    if self.peek() == Some(&Tok::Bang) || self.word("not") {
      self.next();
      return Some(Node::Not(Box::new(self.unary(d + 1)?)));
    }
    self.primary(d + 1)
  }

  fn primary(&mut self, d: usize) -> Option<Node> {
    if d > MAX_DEPTH {
      return None;
    }
    if self.peek() == Some(&Tok::Open) {
      self.next();
      let e = self.or(d + 1)?;
      return (self.next()? == Tok::Close).then_some(e);
    }
    self.term()
  }

  fn term(&mut self) -> Option<Node> {
    let Some(Tok::Word(w)) = self.next() else { return None };
    let kw = w.to_lowercase();
    if kw == "bits" && matches!(self.peek(), Some(Tok::Op(_))) {
      let Some(Tok::Op(c)) = self.next() else { return None };
      let Some(Tok::Word(n)) = self.next() else { return None };
      return n.parse::<f64>().ok().filter(|x| x.is_finite()).map(|n| Node::Bits(c, n));
    }
    if self.peek() == Some(&Tok::Colon) {
      if !["user", "badge", "type", "msgtype", "contains", "has", "regex"].contains(&kw.as_str()) {
        return None;
      }
      self.next();
      let (Some(Tok::Word(v)) | Some(Tok::Str(v))) = self.next() else { return None };
      return match kw.as_str() {
        "user" => Some(Node::User(bare(&v.to_lowercase()).to_owned())),
        "badge" => Some(Node::Badge(v.to_lowercase())),
        "type" | "msgtype" => Some(Node::Type(Ty::parse(&v.to_lowercase(), true))),
        "contains" | "has" => Some(Node::Contains(v.to_lowercase())),
        _ => safe_regex(&v, true).map(Node::Re),
      };
    }
    match kw.as_str() {
      "first" => Some(Node::Flag(Ty::First)),
      "action" => Some(Node::Flag(Ty::Action)),
      "reply" => Some(Node::Flag(Ty::Reply)),
      "cheer" => Some(Node::Flag(Ty::Cheer)),
      _ => None,
    }
  }
}

fn parse_expr(src: &str) -> Option<Node> {
  let toks = tokens(src)?;
  if toks.is_empty() {
    return None;
  }
  let mut p = Parser { toks, pos: 0 };
  let n = p.or(0)?;
  (p.pos == p.toks.len()).then_some(n)
}

// ---- rules

#[derive(Clone, Debug)]
enum Test {
  // a whole word or phrase, between spaces / punctuation / the line's ends
  Keyword { val: String, cs: bool },
  Re(Regex),
  Expr(Node),
  User { val: String, cs: bool },
  Badge(String),
  Type(Option<Ty>),
}

#[derive(Clone, Debug)]
struct Compiled {
  hide: bool,
  // false = scoped to one of the ext's channel tabs: never applies here
  here: bool,
  test: Test,
}

#[derive(Clone, Debug)]
struct Rule {
  // the rule exactly as the setting holds it
  raw: Value,
  // None = the ext would drop it too (off, no id, unknown type, bad expression): kept, never applied
  c: Option<Compiled>,
}

// js truthiness, for the fields the ext tests with `!x`
fn truthy(v: Option<&Value>) -> bool {
  match v {
    None | Some(Value::Null) => false,
    Some(Value::Bool(b)) => *b,
    Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
    Some(Value::String(s)) => !s.is_empty(),
    Some(_) => true,
  }
}

fn word_at(text: &str, val: &str) -> bool {
  let edge = |c: char| c.is_whitespace() || ",!?.:;'\"".contains(c);
  text.match_indices(val).any(|(i, _)| {
    text[..i].chars().next_back().is_none_or(edge) && text[i + val.len()..].chars().next().is_none_or(edge)
  })
}

fn compile(raw: &Value) -> Option<Compiled> {
  let o = raw.as_object()?;
  if !truthy(o.get("id")) || !truthy(o.get("enabled")) {
    return None;
  }
  let m = o.get("match")?.as_object()?;
  let (ty, val) = (m.get("type")?.as_str()?, m.get("value")?.as_str()?.trim());
  if val.is_empty() {
    return None;
  }
  let hide = match o.get("action")?.as_str()? {
    "hide" => true,
    "highlight" => false,
    _ => return None,
  };
  let here = !truthy(o.get("scope")) || o.get("scope").and_then(Value::as_str) == Some("all");
  let cs = truthy(m.get("caseSensitive"));
  let lower = |s: &str| if cs { s.to_owned() } else { s.to_lowercase() };
  let test = match ty {
    "keyword" => Test::Keyword { val: lower(val), cs },
    "regex" => Test::Re(safe_regex(val, !cs)?),
    "expr" => Test::Expr(parse_expr(val)?),
    "user" => Test::User { val: bare(&lower(val)).to_owned(), cs },
    "badge" => Test::Badge(val.to_lowercase()),
    "msgtype" => Test::Type(Ty::parse(&val.to_lowercase(), false)),
    _ => return None,
  };
  Some(Compiled { hide, here, test })
}

impl Compiled {
  fn hits(&self, l: &Line) -> bool {
    match &self.test {
      Test::Keyword { val, cs } => word_at(if *cs { l.text } else { l.low() }, val),
      Test::Re(re) => re.is_match(l.text),
      Test::Expr(n) => n.eval(l),
      Test::User { val, cs } => {
        let (a, b) = (bare(&l.m.login), bare(&l.m.nick));
        !l.m.login.is_empty()
          && if *cs { a == val || b == val } else { a.to_lowercase() == *val || b.to_lowercase() == *val }
      }
      Test::Badge(v) => has_badge(l.m, v),
      Test::Type(t) => t.is_some_and(|t| t.holds(l.m)),
    }
  }
}

#[derive(Default, Clone, Debug)]
pub struct Rules {
  list: Vec<Rule>,
}

impl Rules {
  pub fn is_empty(&self) -> bool {
    self.list.is_empty()
  }

  pub fn len(&self) -> usize {
    self.list.len()
  }

  // the setting's text in. None = not usable (too big, too many, not a JSON array): the caller keeps what
  // it had. a rule that can't be read is kept as it came, like the ext keeps it in the setting
  pub fn parse(text: &str) -> Option<Rules> {
    let text = text.trim();
    if text.len() > MAX_JSON {
      return None;
    }
    if text.is_empty() {
      return Some(Rules::default());
    }
    let Value::Array(a) = serde_json::from_str(text).ok()? else { return None };
    if a.len() > MAX_RULES {
      return None;
    }
    Some(Rules { list: a.into_iter().map(|raw| Rule { c: compile(&raw), raw }).collect() })
  }

  // the setting's text out, as the ext reads it
  pub fn json(&self) -> String {
    Value::Array(self.list.iter().map(|r| r.raw.clone()).collect()).to_string()
  }

  // the verdict on one line: the first hide wins, else the first highlight
  pub fn judge(&self, l: &Line) -> Verdict {
    let mut mark = false;
    for c in self.list.iter().filter_map(|r| r.c.as_ref()).filter(|c| c.here) {
      if !c.hits(l) {
        continue;
      }
      if c.hide {
        return Verdict::Hide;
      }
      mark = true;
    }
    if mark { Verdict::Mark } else { Verdict::Pass }
  }

  // one line per rule for /rule list: "1 on hide keyword spoiler"
  pub fn describe(&self) -> Vec<String> {
    self
      .list
      .iter()
      .enumerate()
      .map(|(i, r)| {
        let s = |k: &str| r.raw.get(k).and_then(Value::as_str).unwrap_or("?").to_owned();
        let m = r.raw.get("match");
        let ms = |k: &str| m.and_then(|m| m.get(k)).and_then(Value::as_str).unwrap_or("?").to_owned();
        let state = match (&r.c, truthy(r.raw.get("enabled"))) {
          (Some(c), _) if !c.here => "other channel".to_owned(),
          (Some(_), _) => "on".to_owned(),
          (None, false) => "off".to_owned(),
          (None, true) => "invalid".to_owned(),
        };
        let val: String = ms("value").chars().take(60).collect();
        format!("{} {state} {} {} {val}", i + 1, s("action"), ms("type"))
      })
      .collect()
  }

  // /rule add: a new rule at the end. Err = why not
  pub fn add(&mut self, id: String, action: &str, ty: &str, value: &str) -> Result<(), &'static str> {
    if self.list.len() >= MAX_RULES {
      return Err("too many rules");
    }
    if !matches!(action, "hide" | "highlight") {
      return Err("a rule hides or highlights");
    }
    if !matches!(ty, "keyword" | "regex" | "expr" | "user" | "badge" | "msgtype") {
      return Err("type is keyword, regex, user, badge, msgtype or expr");
    }
    let mut raw = json!({
      "id": id,
      "enabled": true,
      "scope": "all",
      "match": { "type": ty, "value": value.trim(), "caseSensitive": false },
      "action": action,
    });
    if action == "highlight" {
      raw["color"] = json!("#ffff00");
    }
    let Some(c) = compile(&raw) else {
      return Err(if ty == "expr" { "that expression doesn't parse" } else { "that rule isn't valid" });
    };
    self.list.push(Rule { raw, c: Some(c) });
    if self.json().len() > MAX_JSON {
      self.list.pop();
      return Err("rules are full");
    }
    Ok(())
  }

  // the rule a user means: its number in /rule list, or its id
  fn find(&self, key: &str) -> Option<usize> {
    match key.parse::<usize>() {
      Ok(n) if (1..=self.list.len()).contains(&n) => Some(n - 1),
      _ => self.list.iter().position(|r| r.raw.get("id").and_then(Value::as_str) == Some(key)),
    }
  }

  pub fn remove(&mut self, key: &str) -> bool {
    self.find(key).map(|i| self.list.remove(i)).is_some()
  }

  pub fn enable(&mut self, key: &str, on: bool) -> bool {
    let Some(i) = self.find(key) else { return false };
    let r = &mut self.list[i];
    if let Some(o) = r.raw.as_object_mut() {
      o.insert("enabled".into(), json!(on));
    }
    r.c = compile(&r.raw);
    true
  }

  // earlier = wins; up / down moves a rule one place
  pub fn shift(&mut self, key: &str, up: bool) -> bool {
    let Some(i) = self.find(key) else { return false };
    let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|&j| j < self.list.len()) };
    j.map(|j| self.list.swap(i, j)).is_some()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::automod::SCAN;

  fn line(login: &str, body: &str) -> Message {
    let mut m = Message::system(0, Platform::Twitch, "");
    m.kind = Kind::Chat;
    m.login = login.into();
    m.nick = login.into();
    m.body = body.into();
    m
  }

  fn rule(action: &str, ty: &str, value: &str) -> Value {
    json!({ "id": "x", "enabled": true, "scope": "all", "match": { "type": ty, "value": value }, "action": action })
  }

  fn rules(rs: &[Value]) -> Rules {
    Rules::parse(&Value::Array(rs.to_vec()).to_string()).unwrap()
  }

  fn judge(r: &Rules, m: &Message) -> Verdict {
    r.judge(&Line::new(m))
  }

  #[test]
  fn keyword_is_a_whole_word_or_phrase() {
    let r = rules(&[rule("hide", "keyword", "Spoiler")]);
    for hit in ["spoiler", "no SPOILER!", "a spoiler, ok", "the 'spoiler'", "x\tspoiler"] {
      assert_eq!(judge(&r, &line("a", hit)), Verdict::Hide, "{hit}");
    }
    for miss in ["spoilers", "unspoiler", "nospoiler here"] {
      assert_eq!(judge(&r, &line("a", miss)), Verdict::Pass, "{miss}");
    }
    let p = rules(&[rule("hide", "keyword", "big news")]);
    assert_eq!(judge(&p, &line("a", "so big news today")), Verdict::Hide);
    // case sensitive
    let mut cs = rule("hide", "keyword", "GG");
    cs["match"]["caseSensitive"] = json!(true);
    let r = rules(&[cs]);
    assert_eq!(judge(&r, &line("a", "gg")), Verdict::Pass);
    assert_eq!(judge(&r, &line("a", "GG wp")), Verdict::Hide);
  }

  #[test]
  fn regex_user_badge_msgtype() {
    let r = rules(&[rule("hide", "regex", r"^\d{3,}$")]);
    assert_eq!(judge(&r, &line("a", "12345")), Verdict::Hide);
    assert_eq!(judge(&r, &line("a", "12")), Verdict::Pass);
    let r = rules(&[rule("hide", "user", "@Troll")]);
    assert_eq!(judge(&r, &line("troll", "x")), Verdict::Hide);
    assert_eq!(judge(&r, &line("other", "x")), Verdict::Pass);
    let mut m = line("a", "x");
    m.badges = Box::new([("subscriber".into(), "12".into())]);
    let r = rules(&[rule("highlight", "badge", "Subscriber")]);
    assert_eq!(judge(&r, &m), Verdict::Mark);
    assert_eq!(judge(&r, &line("a", "x")), Verdict::Pass);
    let mut first = line("a", "hi");
    first.flags |= flag::FIRST;
    let mut act = line("a", "waves");
    act.kind = Kind::Action;
    let mut rep = line("a", "yes");
    rep.reply_to = Some("bob".into());
    for (ty, m) in [("first-message", &first), ("action", &act), ("reply", &rep)] {
      let r = rules(&[rule("highlight", "msgtype", ty)]);
      assert_eq!(judge(&r, m), Verdict::Mark, "{ty}");
      assert_eq!(judge(&r, &line("a", "plain")), Verdict::Pass, "{ty}");
    }
    // cheer needs bits, which the window doesn't keep; an unknown type matches nothing
    for ty in ["cheer", "bogus"] {
      assert_eq!(judge(&rules(&[rule("hide", "msgtype", ty)]), &line("a", "x")), Verdict::Pass);
    }
  }

  #[test]
  fn expressions() {
    let ok = |e: &str, m: &Message| judge(&rules(&[rule("hide", "expr", e)]), m) == Verdict::Hide;
    let mut sub = line("bob", "Hello there");
    sub.badges = Box::new([("subscriber".into(), "1".into())]);
    sub.flags |= flag::FIRST;
    assert!(ok("user:bob && badge:subscriber", &sub));
    assert!(ok("user:bob and not reply", &sub));
    assert!(!ok("user:bob && !first", &sub));
    assert!(ok("first || contains:\"never\"", &sub));
    assert!(ok("contains:\"HELLO THERE\"", &sub));
    assert!(ok("has:hello && type:first-message", &sub));
    assert!(ok("regex:^hel", &sub));
    assert!(ok("(reply || action) or user:@BOB", &sub));
    assert!(!ok("(reply || action)", &sub));
    assert!(ok("bits < 1 && bits == 0 && bits >= 0", &sub));
    assert!(!ok("bits > 100", &sub));
    assert!(ok("!cheer", &sub));
  }

  #[test]
  fn bad_expressions_drop_the_rule() {
    let bad = [
      "",
      "user:",
      "user",
      "frobnicate:x",
      "first &&",
      "&& first",
      "(first",
      "first)",
      "bits >",
      "bits 5",
      "a = b",
      "contains:\"open",
      "first first",
      "regex:",
      "!",
      "user:bob &",
      "bits > nan",
    ];
    for e in bad {
      let r = rules(&[rule("hide", "expr", e)]);
      assert_eq!(judge(&r, &line("bob", "x")), Verdict::Pass, "{e:?}");
      assert!(r.describe()[0].contains("invalid"), "{e:?}");
    }
    // nesting too deep, too long, too many tokens
    let deep = format!("{}first{}", "(".repeat(60), ")".repeat(60));
    let long = vec!["first"; 400].join(" || ");
    let toks = vec!["a"; 300].join(" ");
    for e in [deep, long, toks] {
      assert!(parse_expr(&e).is_none());
    }
    // a generous but legal nesting still works
    assert!(parse_expr(&format!("{}first{}", "(".repeat(8), ")".repeat(8))).is_some());
  }

  #[test]
  fn first_hide_wins_then_first_highlight() {
    let r = rules(&[rule("highlight", "keyword", "gg"), rule("hide", "keyword", "gg"), rule("hide", "keyword", "wp")]);
    assert_eq!(judge(&r, &line("a", "gg wp")), Verdict::Hide);
    assert_eq!(judge(&r, &line("a", "gg")), Verdict::Hide);
    let r = rules(&[rule("highlight", "keyword", "a"), rule("highlight", "keyword", "b")]);
    assert_eq!(judge(&r, &line("x", "b")), Verdict::Mark);
    assert_eq!(judge(&r, &line("x", "c")), Verdict::Pass);
  }

  #[test]
  fn what_the_ext_drops_is_kept_but_never_applies() {
    let mut off = rule("hide", "keyword", "x");
    off["enabled"] = json!(false);
    let mut noid = rule("hide", "keyword", "x");
    noid["id"] = json!("");
    let mut scoped = rule("hide", "keyword", "x");
    scoped["scope"] = json!("tab-9");
    let mut extra = rule("hide", "keyword", "x");
    extra["color"] = json!("#ff0000");
    extra["future"] = json!({ "a": [1, 2] });
    let r = rules(&[
      off,
      noid,
      scoped,
      json!(5),
      json!(null),
      rule("hide", "nope", "x"),
      rule("zap", "keyword", "x"),
      rule("hide", "keyword", "  "),
      extra.clone(),
    ]);
    assert_eq!(r.len(), 9);
    assert_eq!(judge(&r, &line("a", "x")), Verdict::Hide);
    let only = rules(&[extra]);
    assert!(only.json().contains("future"));
    let d = r.describe();
    assert!(d[0].contains("off") && d[2].contains("other channel") && d[5].contains("invalid"));
    // round trip loses nothing
    let again = Rules::parse(&r.json()).unwrap();
    assert_eq!(again.json(), r.json());
    // a scoped rule alone means nothing applies
    let mut sc = rule("hide", "keyword", "x");
    sc["scope"] = json!("c1");
    assert_eq!(judge(&rules(&[sc]), &line("a", "x")), Verdict::Pass);
  }

  #[test]
  fn hostile_json() {
    assert!(Rules::parse("").unwrap().is_empty());
    assert!(Rules::parse("[]").unwrap().is_empty());
    for bad in ["{}", "null", "5", "\"x\"", "[", "not json", "[1,2,", "{\"a\":[]}"] {
      assert!(Rules::parse(bad).is_none(), "{bad}");
    }
    // wrong types everywhere: kept, never applied, never a panic
    let weird = r#"[{"id":1,"enabled":"yes","match":{"type":5,"value":[]},"action":{}},
      {"id":"a","enabled":true,"match":"x","action":"hide"},
      {"id":"a","enabled":true,"match":{"type":"keyword","value":9},"action":"hide"},
      [[[[]]]], "str", 3.5, true]"#;
    let r = Rules::parse(weird).unwrap();
    assert_eq!(r.len(), 7);
    assert_eq!(judge(&r, &line("a", "anything 9")), Verdict::Pass);
    // too many / too big / nested to the parser's own limit: refused whole
    let many = Value::Array(vec![rule("hide", "keyword", "x"); MAX_RULES + 1]).to_string();
    assert!(Rules::parse(&many).is_none());
    assert!(Rules::parse(&format!("[\"{}\"]", "x".repeat(MAX_JSON))).is_none());
    assert!(Rules::parse(&format!("{}{}", "[".repeat(100_000), "]".repeat(100_000))).is_none());
    // a huge regex value does not hang or panic: it is matched as text
    let r = rules(&[rule("hide", "regex", &format!("(a+)+{}", "b".repeat(600)))]);
    assert_eq!(judge(&r, &line("a", &"a".repeat(300))), Verdict::Pass);
    let r = rules(&[rule("hide", "regex", "(a+)+$")]);
    assert_eq!(judge(&r, &line("a", &format!("{}!", "a".repeat(250)))), Verdict::Pass);
    assert_eq!(judge(&r, &line("a", "aaa")), Verdict::Hide);
    // js-only syntax is text, not a crash, not dropped
    let r = rules(&[rule("hide", "regex", "(?<=a)b")]);
    assert_eq!(judge(&r, &line("a", "x (?<=a)b y")), Verdict::Hide);
  }

  #[test]
  fn only_the_first_256_characters_are_read() {
    let r = rules(&[rule("hide", "keyword", "needle")]);
    assert_eq!(judge(&r, &line("a", &format!("{} needle", "x".repeat(SCAN)))), Verdict::Pass);
    assert_eq!(judge(&r, &line("a", &format!("needle {}", "é".repeat(500)))), Verdict::Hide);
  }

  #[test]
  fn editing() {
    let mut r = Rules::default();
    assert!(r.add("a".into(), "hide", "keyword", "one").is_ok());
    assert!(r.add("b".into(), "highlight", "expr", "first").is_ok());
    assert!(r.add("c".into(), "hide", "expr", "first &&").is_err());
    assert!(r.add("c".into(), "zap", "keyword", "x").is_err());
    assert!(r.add("c".into(), "hide", "frob", "x").is_err());
    assert!(r.add("c".into(), "hide", "keyword", " ").is_err());
    assert_eq!(r.len(), 2);
    assert!(r.json().contains("#ffff00"));
    assert!(r.enable("1", false));
    assert_eq!(judge(&r, &line("a", "one")), Verdict::Pass);
    assert!(r.enable("a", true));
    assert_eq!(judge(&r, &line("a", "one")), Verdict::Hide);
    assert!(r.shift("2", true));
    assert!(r.describe()[0].contains("highlight"));
    assert!(!r.shift("1", true) && !r.shift("2", false));
    assert!(r.remove("b") && !r.remove("b") && !r.remove("9"));
    assert_eq!(r.len(), 1);
    for i in 0..MAX_RULES {
      let _ = r.add(format!("i{i}"), "hide", "keyword", "w");
    }
    assert_eq!(r.add("z".into(), "hide", "keyword", "w"), Err("too many rules"));
  }

  // 10k lines x 50 rules of every kind: the number goes in the commit message
  #[test]
  fn perf() {
    let mut rs: Vec<Value> = Vec::new();
    for i in 0..10 {
      rs.push(rule("hide", "keyword", &format!("spoilerword{i}")));
      rs.push(rule("highlight", "keyword", &format!("hype{i}")));
      rs.push(rule("hide", "regex", &format!(r"buy \d+ f{i}llowers")));
      rs.push(rule("hide", "user", &format!("troll{i}")));
      rs.push(rule("highlight", "expr", &format!("badge:vip{i} && contains:\"gg{i}\" || user:nobody{i}")));
    }
    let r = rules(&rs);
    assert_eq!(r.len(), 50);
    let lines: Vec<Message> = (0..10_000)
      .map(|i| {
        line(
          &format!("user{}", i % 300),
          &format!("this is chat line number {i} with some ordinary words in it, nothing to see"),
        )
      })
      .collect();
    let t = std::time::Instant::now();
    let mut hits = 0;
    for m in &lines {
      hits += usize::from(judge(&r, m) != Verdict::Pass);
    }
    let us = t.elapsed().as_secs_f64() * 1e6 / lines.len() as f64;
    eprintln!("rules: 50 rules x 10k lines = {us:.2} us/line ({hits} hits)");
    assert!(us < 100.0, "{us}");
  }
}
