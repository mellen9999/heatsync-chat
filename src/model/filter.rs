// what the chat hides, strips or lights up (ext main.js isMsgFiltered, keywordHighlights, mutedUsers,
// blockedUsers). decided per line at render time and kept off the data: turning a filter off brings the
// lines back. the lists live here, saved as one small `filters` file; the on/off switches are Settings
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::model::automod::{self, Automod};
use crate::model::message::{Message, flag};
use crate::model::reasons::Reasons;
use crate::model::rules::{Line, Rules, Verdict};
use crate::store;

// a line's verdict bits
pub const HIDDEN: u8 = 1;
// the sender is muted: their line stays (name, time) but the words and pictures go
pub const MUTED: u8 = 2;
// a keyword you highlight is in it
pub const MARKED: u8 = 4;

// the ext's _BOT_NAMES
const BOTS: [&str; 12] = [
  "nightbot",
  "streamelements",
  "moobot",
  "fossabot",
  "streamlabs",
  "wizebot",
  "botrix",
  "sery_bot",
  "soundalerts",
  "pokemoncommunitygame",
  "kofistreambot",
  "blerp",
];

// hostile or just huge lists must not grow without bound
const MAX_TERMS: usize = 200;
const MAX_TERM: usize = 64;
const MAX_NAMES: usize = 5000;
pub const DEFAULT_MUTE_MS: i64 = 24 * 3600 * 1000;

#[derive(Default, Clone, Debug)]
pub struct Filters {
  pub bots: bool,
  pub cmds: bool,
  pub dupes: bool,
  // automod: hide lines that are mostly capitals (ext automodAllCaps) or match a pattern (automodRegex)
  pub caps: bool,
  automod: Automod,
  // the ext's ban reasons (modBanReasons): the choices a ban offers
  reasons: Reasons,
  // the ext's per-rule hide / highlight list (chatFilterRules)
  rules: Rules,
  // lowercase terms: a line containing one is hidden / lit up
  words: Vec<Box<str>>,
  marks: Vec<Box<str>>,
  // lowercase login → when the mute ends (None = until you lift it)
  mutes: HashMap<Box<str>, Option<i64>>,
  blocks: HashSet<Box<str>>,
  // entries that came from the account's server lists (not typed here): they leave with the login
  synced_mutes: HashSet<Box<str>>,
  synced_blocks: HashSet<Box<str>>,
  // your own names (lowercase): never hidden by these filters
  me: Vec<Box<str>>,
}

// "@Bob" → "bob"; youtube handles carry a leading @ in chat but not in lists
pub fn key(name: &str) -> Box<str> {
  name.trim().trim_start_matches('@').to_lowercase().into()
}

fn word_char(c: char) -> bool {
  c.is_alphanumeric() || c == '_'
}

// `term` in `text` as a whole word (the ext's \b…\b), both already lowercase
fn has_word(text: &str, term: &str) -> bool {
  let (Some(first), Some(last)) = (term.chars().next(), term.chars().next_back()) else { return false };
  text.match_indices(term).any(|(i, _)| {
    let before = text[..i].chars().next_back();
    let after = text[i + term.len()..].chars().next();
    (!word_char(first) || !before.is_some_and(word_char)) && (!word_char(last) || !after.is_some_and(word_char))
  })
}

fn clean_term(t: &str) -> Option<Box<str>> {
  let t = t.trim().to_lowercase();
  (!t.is_empty() && t.chars().count() <= MAX_TERM && !t.chars().any(char::is_control)).then(|| t.into())
}

impl Filters {
  pub fn set_me(&mut self, names: &[Box<str>]) {
    self.me = names.iter().map(|n| key(n)).collect();
  }

  fn is_me(&self, m: &Message) -> bool {
    m.has(flag::OWN) || self.me.iter().any(|n| **n == *key(&m.login))
  }

  // the verdict for one line. `tab_hidden` = people hidden in this tab only (/hide); `last` = the text
  // of the previous line that was shown, for hide duplicates
  pub fn judge(&self, m: &Message, now: i64, tab_hidden: &HashSet<Box<str>>, last: &mut Box<str>) -> u8 {
    // notices and stream lines are never filtered
    if !m.kind.said() {
      return 0;
    }
    let who = key(&m.login);
    let me = self.is_me(m);
    if !me && (self.blocks.contains(&who) || tab_hidden.contains(&who)) {
      return HIDDEN;
    }
    if !me && self.bots && BOTS.contains(&&*who) {
      return HIDDEN;
    }
    if !me && self.cmds && m.body.starts_with('!') {
      return HIDDEN;
    }
    let low = (!self.words.is_empty() || !self.marks.is_empty()).then(|| m.body.to_lowercase());
    if !me && low.as_deref().is_some_and(|l| self.words.iter().any(|w| l.contains(&**w))) {
      return HIDDEN;
    }
    // automod, then the rules: the ext's order. your own lines are never hidden; a hide rule that
    // matches one also keeps a highlight rule from lighting it
    if !me && ((self.caps && automod::all_caps(&m.body)) || self.automod.hits(&m.body)) {
      return HIDDEN;
    }
    let ruled = if self.rules.is_empty() { Verdict::Pass } else { self.rules.judge(&Line::new(m)) };
    if ruled == Verdict::Hide && !me {
      return HIDDEN;
    }
    if self.dupes && !me && !m.body.is_empty() {
      if **last == *m.body {
        return HIDDEN;
      }
      *last = m.body.clone();
    }
    let mut look = 0;
    if self.muted(&who, now) {
      look |= MUTED;
    }
    if ruled == Verdict::Mark || low.as_deref().is_some_and(|l| self.marks.iter().any(|t| has_word(l, t))) {
      look |= MARKED;
    }
    look
  }

  pub fn muted(&self, who: &str, now: i64) -> bool {
    self.mutes.get(who).is_some_and(|exp| exp.is_none_or(|e| e > now))
  }

  pub fn blocked(&self, who: &str) -> bool {
    self.blocks.contains(who)
  }

  // when the soonest timed mute ends, so the view can be redone then
  pub fn next_expiry(&self, now: i64) -> Option<i64> {
    self.mutes.values().flatten().copied().filter(|&e| e > now).min()
  }

  pub fn mute(&mut self, who: &str, until: Option<i64>) -> bool {
    let k = key(who);
    if k.is_empty() || (self.mutes.len() >= MAX_NAMES && !self.mutes.contains_key(&k)) {
      return false;
    }
    self.synced_mutes.remove(&k);
    self.mutes.insert(k, until);
    true
  }

  // a mute the account's server list holds. one already set by hand stays yours. true = new here
  pub fn mute_synced(&mut self, who: &str, until: Option<i64>) -> bool {
    let k = key(who);
    if self.mutes.contains_key(&k) && !self.synced_mutes.contains(&k) {
      // yours stays yours, but another device may have changed how long it runs
      self.mutes.insert(k, until);
      return false;
    }
    let new = !self.mutes.contains_key(&k);
    if !self.mute_raw(&k, until) {
      return false;
    }
    self.synced_mutes.insert(k);
    new
  }

  fn mute_raw(&mut self, k: &str, until: Option<i64>) -> bool {
    if k.is_empty() || (self.mutes.len() >= MAX_NAMES && !self.mutes.contains_key(k)) {
      return false;
    }
    self.mutes.insert(k.into(), until);
    true
  }

  pub fn block_synced(&mut self, who: &str) -> bool {
    let k = key(who);
    if self.blocks.contains(&k) && !self.synced_blocks.contains(&k) {
      return false;
    }
    let new = self.block_raw(&k);
    if self.blocks.contains(&k) {
      self.synced_blocks.insert(k);
    }
    new
  }

  fn block_raw(&mut self, k: &str) -> bool {
    !k.is_empty() && (self.blocks.len() < MAX_NAMES || self.blocks.contains(k)) && self.blocks.insert(k.into())
  }

  // logged out or another account: its server-origin mutes and blocks go, yours stay. true = changed
  pub fn drop_synced(&mut self) -> bool {
    let n = self.mutes.len() + self.blocks.len();
    let (m, b) = (std::mem::take(&mut self.synced_mutes), std::mem::take(&mut self.synced_blocks));
    self.mutes.retain(|k, _| !m.contains(k));
    self.blocks.retain(|k| !b.contains(k));
    self.mutes.len() + self.blocks.len() != n
  }

  // the account's block list is the whole truth for the blocks it gave us: one it no longer holds goes (yours,
  // made here, stay). true = changed
  pub fn blocks_follow(&mut self, server: &[Box<str>]) -> bool {
    let gone: Vec<Box<str>> = self.synced_blocks.iter().filter(|b| !server.iter().any(|s| s == *b)).cloned().collect();
    for b in &gone {
      self.synced_blocks.remove(b);
      self.blocks.remove(b);
    }
    !gone.is_empty()
  }

  pub fn unmute(&mut self, who: &str) -> bool {
    let k = key(who);
    self.synced_mutes.remove(&k);
    self.mutes.remove(&k).is_some()
  }

  pub fn clear_mutes(&mut self) {
    self.mutes.clear();
    self.synced_mutes.clear();
  }

  // drop mutes that ran out. true = something changed
  pub fn prune(&mut self, now: i64) -> bool {
    let n = self.mutes.len();
    self.mutes.retain(|_, e| e.is_none_or(|e| e > now));
    self.mutes.len() != n
  }

  pub fn block(&mut self, who: &str) -> bool {
    let k = key(who);
    self.synced_blocks.remove(&k);
    self.block_raw(&k)
  }

  pub fn unblock(&mut self, who: &str) -> bool {
    let k = key(who);
    self.synced_blocks.remove(&k);
    self.blocks.remove(&k)
  }

  pub fn blocks_sorted(&self) -> Vec<String> {
    let mut v: Vec<String> = self.blocks.iter().map(|b| b.to_string()).collect();
    v.sort_unstable();
    v
  }

  pub fn mutes(&self) -> Vec<(&str, Option<i64>)> {
    let mut v: Vec<_> = self.mutes.iter().map(|(k, e)| (&**k, *e)).collect();
    v.sort_unstable();
    v
  }

  pub fn words(&self, marks: bool) -> &[Box<str>] {
    if marks { &self.marks } else { &self.words }
  }

  // add / remove a hide or highlight term. false = nothing changed (bad, duplicate, full, not there)
  pub fn word(&mut self, marks: bool, add: bool, term: &str) -> bool {
    let Some(t) = clean_term(term) else { return false };
    let list = if marks { &mut self.marks } else { &mut self.words };
    match (add, list.iter().position(|w| *w == t)) {
      (true, None) if list.len() < MAX_TERMS => {
        list.push(t);
        true
      }
      (false, Some(i)) => {
        list.remove(i);
        true
      }
      _ => false,
    }
  }

  pub fn patterns(&self) -> &[Box<str>] {
    self.automod.patterns()
  }

  // add / remove one automod pattern. false = nothing changed
  pub fn pattern(&mut self, add: bool, p: &str) -> bool {
    self.automod.pattern(add, p)
  }

  // the ext's automodRegex text (one pattern per line) in and out
  pub fn patterns_text(&self) -> String {
    self.automod.text()
  }

  pub fn set_patterns_text(&mut self, t: &str) -> bool {
    self.automod.set_text(t)
  }

  pub fn reasons(&self) -> &[Box<str>] {
    self.reasons.list()
  }

  // add / remove one ban reason. false = nothing changed
  pub fn reason(&mut self, add: bool, r: &str) -> bool {
    self.reasons.reason(add, r)
  }

  // the ext's modBanReasons text (one reason per line) in and out
  pub fn reasons_text(&self) -> String {
    self.reasons.text()
  }

  pub fn set_reasons_text(&mut self, t: &str) -> bool {
    self.reasons.set_text(t)
  }

  pub fn rules(&self) -> &Rules {
    &self.rules
  }

  pub fn rules_mut(&mut self) -> &mut Rules {
    &mut self.rules
  }

  // the ext's chatFilterRules text in. false = unusable, the rules stay as they were
  pub fn set_rules_text(&mut self, t: &str) -> bool {
    Rules::parse(t).map(|r| self.rules = r).is_some()
  }

  pub fn parse(s: &str) -> Self {
    let mut f = Self::default();
    for l in s.lines() {
      let (kind, rest) = l.split_once(' ').unwrap_or((l, ""));
      match kind {
        "hide" => _ = f.word(false, true, rest),
        "mark" => _ = f.word(true, true, rest),
        "automod" => _ = f.pattern(true, rest),
        "banreason" => _ = f.reason(true, rest),
        "rules" => _ = f.set_rules_text(rest),
        "block" => _ = f.block(rest),
        "sblock" => _ = f.block_synced(rest),
        "mute" | "smute" => {
          let (name, until) = rest.rsplit_once(' ').unwrap_or((rest, "-"));
          if kind == "mute" {
            f.mute(name, until.parse().ok());
          } else {
            f.mute_synced(name, until.parse().ok());
          }
        }
        _ => {}
      }
    }
    f
  }

  pub fn render(&self) -> String {
    let mut s = String::new();
    for w in &self.words {
      s.push_str(&format!("hide {w}\n"));
    }
    for w in &self.marks {
      s.push_str(&format!("mark {w}\n"));
    }
    for p in self.automod.patterns() {
      s.push_str(&format!("automod {p}\n"));
    }
    for r in self.reasons.list() {
      s.push_str(&format!("banreason {r}\n"));
    }
    if !self.rules.is_empty() {
      s.push_str(&format!("rules {}\n", self.rules.json()));
    }
    let mut blocks: Vec<_> = self.blocks.iter().collect();
    blocks.sort_unstable();
    for b in blocks {
      let tag = if self.synced_blocks.contains(b) { "sblock" } else { "block" };
      s.push_str(&format!("{tag} {b}\n"));
    }
    for (name, until) in self.mutes() {
      let tag = if self.synced_mutes.contains(name) { "smute" } else { "mute" };
      match until {
        Some(e) => s.push_str(&format!("{tag} {name} {e}\n")),
        None => s.push_str(&format!("{tag} {name} -\n")),
      }
    }
    s
  }

  pub fn load() -> Self {
    store::read("filters").map(|s| Self::parse(&s)).unwrap_or_default()
  }

  pub fn save(&self) {
    if let Err(e) = store::write("filters", &self.render()) {
      eprintln!("save filters: {e}");
    }
  }
}

// the chat views read the rules on every new line; the ui thread owns them, so they live in one place
// here instead of being threaded through every push
thread_local! {
  static RULES: RefCell<Filters> = RefCell::new(Filters::default());
}

pub fn with<R>(f: impl FnOnce(&Filters) -> R) -> R {
  RULES.with(|r| f(&r.borrow()))
}

pub fn edit<R>(f: impl FnOnce(&mut Filters) -> R) -> R {
  RULES.with(|r| f(&mut r.borrow_mut()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::Platform;

  fn line(login: &str, body: &str) -> Message {
    let mut m = Message::system(0, Platform::Twitch, "");
    m.kind = crate::model::message::Kind::Chat;
    m.login = login.into();
    m.nick = login.into();
    m.body = body.into();
    m
  }

  fn judge(f: &Filters, m: &Message) -> u8 {
    f.judge(m, 1000, &HashSet::new(), &mut Box::from(""))
  }

  #[test]
  fn bots_commands_and_words_hide() {
    let mut f = Filters { bots: true, cmds: true, ..Filters::default() };
    f.word(false, true, "Spoiler");
    assert_eq!(judge(&f, &line("nightbot", "hi")), HIDDEN);
    assert_eq!(judge(&f, &line("bob", "!drops")), HIDDEN);
    assert_eq!(judge(&f, &line("bob", "no SPOILERS please")), HIDDEN);
    assert_eq!(judge(&f, &line("bob", "hello")), 0);
    // off by default
    assert_eq!(judge(&Filters::default(), &line("nightbot", "!hi")), 0);
  }

  #[test]
  fn your_own_lines_are_never_hidden() {
    let mut f = Filters { bots: true, cmds: true, ..Filters::default() };
    f.set_me(&["Nightbot".into()]);
    f.block("nightbot");
    assert_eq!(judge(&f, &line("nightbot", "!hi")), 0);
  }

  #[test]
  fn blocks_hide_and_mutes_strip() {
    let mut f = Filters::default();
    f.block("@Troll");
    f.mute("quiet", Some(2000));
    assert_eq!(judge(&f, &line("troll", "x")), HIDDEN);
    assert_eq!(judge(&f, &line("quiet", "x")), MUTED);
    // the mute ran out
    assert_eq!(f.judge(&line("quiet", "x"), 3000, &HashSet::new(), &mut Box::from("")), 0);
    assert_eq!(f.next_expiry(1000), Some(2000));
    assert!(f.prune(3000));
    assert!(f.mutes().is_empty());
  }

  #[test]
  fn per_tab_hide() {
    let f = Filters::default();
    let hid: HashSet<Box<str>> = ["bob".into()].into();
    assert_eq!(f.judge(&line("Bob", "x"), 0, &hid, &mut Box::from("")), HIDDEN);
    assert_eq!(f.judge(&line("al", "x"), 0, &hid, &mut Box::from("")), 0);
  }

  #[test]
  fn duplicates_collapse_to_the_first() {
    let f = Filters { dupes: true, ..Filters::default() };
    let mut last: Box<str> = "".into();
    let none = HashSet::new();
    assert_eq!(f.judge(&line("a", "LUL"), 0, &none, &mut last), 0);
    assert_eq!(f.judge(&line("b", "LUL"), 0, &none, &mut last), HIDDEN);
    assert_eq!(f.judge(&line("c", "hi"), 0, &none, &mut last), 0);
    assert_eq!(f.judge(&line("d", "LUL"), 0, &none, &mut last), 0);
  }

  #[test]
  fn highlights_are_whole_words() {
    let mut f = Filters::default();
    f.word(true, true, "gg");
    f.word(true, true, "!drops");
    assert_eq!(judge(&f, &line("a", "GG wp")), MARKED);
    assert_eq!(judge(&f, &line("a", "oggy")), 0);
    assert_eq!(judge(&f, &line("a", "gg.")), MARKED);
    assert_eq!(judge(&f, &line("a", "got !drops")), MARKED);
  }

  #[test]
  fn special_characters_are_plain_text() {
    let mut f = Filters::default();
    f.word(false, true, "a.c(");
    assert_eq!(judge(&f, &line("x", "abc(")), 0);
    assert_eq!(judge(&f, &line("x", "A.C( yes")), HIDDEN);
  }

  #[test]
  fn file_roundtrip_and_limits() {
    let mut f = Filters::default();
    f.word(false, true, "spoil er");
    f.word(true, true, "gg");
    f.block("troll");
    f.mute("quiet", Some(5));
    f.mute("perm", None);
    let g = Filters::parse(&f.render());
    assert_eq!(g.words(false), f.words(false));
    assert_eq!(g.words(true), f.words(true));
    assert!(g.blocked("troll"));
    assert_eq!(g.mutes(), vec![("perm", None), ("quiet", Some(5))]);
    assert_eq!(Filters::parse("garbage\nmute\nhide \n").mutes().len(), 0);
    assert!(!Filters::default().clone().word(false, true, &"x".repeat(MAX_TERM + 1)));
  }

  #[test]
  fn automod_and_rules_share_the_one_verdict() {
    let mut f = Filters { caps: true, ..Filters::default() };
    f.pattern(true, r"bit\.ly");
    f.rules_mut().add("a".into(), "hide", "keyword", "nope").unwrap();
    f.rules_mut().add("b".into(), "highlight", "keyword", "gg").unwrap();
    let shout = "STOP SHOUTING AT ME PLEASE";
    assert_eq!(judge(&f, &line("a", shout)), HIDDEN);
    assert_eq!(judge(&f, &line("a", "go to BIT.LY now")), HIDDEN);
    assert_eq!(judge(&f, &line("a", "oh nope")), HIDDEN);
    assert_eq!(judge(&f, &line("a", "gg")), MARKED);
    assert_eq!(judge(&f, &line("a", "fine")), 0);
    // a mod is not spared, only you are
    let mut m = line("a", shout);
    m.badges = Box::new([("moderator".into(), "1".into())]);
    assert_eq!(judge(&f, &m), HIDDEN);
    f.set_me(&["me".into()]);
    assert_eq!(judge(&f, &line("me", shout)), 0);
    assert_eq!(judge(&f, &line("me", "bit.ly nope")), 0);
    // a highlight rule still lights your own line, a hide rule that matches it keeps it plain
    assert_eq!(judge(&f, &line("me", "gg")), MARKED);
    assert_eq!(judge(&f, &line("me", "gg nope")), 0);
    // a hide rule beats the highlight rules after it and the keyword highlights
    f.word(true, true, "gg");
    assert_eq!(judge(&f, &line("a", "gg nope")), HIDDEN);
    // turned off, the lines come back
    f.caps = false;
    assert_eq!(judge(&f, &line("a", shout)), 0);
    assert!(f.pattern(false, r"bit\.ly"));
    assert_eq!(judge(&f, &line("a", "bit.ly")), 0);
    assert!(f.rules_mut().remove("1"));
    assert_eq!(judge(&f, &line("a", "oh nope")), 0);
    // system lines are never judged
    f.caps = true;
    assert_eq!(judge(&f, &Message::system(0, Platform::Twitch, shout)), 0);
  }

  #[test]
  fn automod_and_rules_survive_the_file_and_the_account_text() {
    let mut f = Filters::default();
    f.pattern(true, r"free\s+v-?bucks");
    f.pattern(true, "oh (no");
    f.rules_mut().add("a".into(), "hide", "expr", "user:bob && !first").unwrap();
    let g = Filters::parse(&f.render());
    assert_eq!(g.patterns_text(), f.patterns_text());
    assert_eq!(g.rules().json(), f.rules().json());
    assert_eq!(judge(&g, &line("bob", "hi")), HIDDEN);
    assert_eq!(judge(&g, &line("al", "FREE   vbucks")), HIDDEN);
    // the account's text in: unusable text changes nothing
    let mut h = Filters::default();
    assert!(h.set_patterns_text("a\n  b  \n") && h.patterns().len() == 2);
    assert!(!h.set_patterns_text(&"x\n".repeat(100)) && h.patterns().len() == 2);
    assert!(h.set_rules_text(&f.rules().json()) && h.rules().len() == 1);
    assert!(!h.set_rules_text("{\"not\":\"an array\"}") && h.rules().len() == 1);
    // a hostile file line is skipped, not fatal
    let junk = Filters::parse("rules [[[[\nrules {}\nautomod \nautomod x\n");
    assert_eq!((junk.patterns().len(), junk.rules().len()), (1, 0));
  }

  #[test]
  fn logout_drops_server_origin_entries_and_keeps_yours() {
    let mut f = Filters::default();
    f.mute("mine", None);
    f.block("troll");
    assert!(f.mute_synced("theirs", Some(99)));
    assert!(f.block_synced("spam"));
    // a name you already hold by hand stays yours even when the server lists it too
    assert!(!f.mute_synced("mine", None));
    assert!(!f.block_synced("troll"));
    // survives a save + load
    let mut g = Filters::parse(&f.render());
    assert!(g.drop_synced());
    assert!(g.muted("mine", 0) && g.blocked("troll"));
    assert!(!g.muted("theirs", 0) && !g.blocked("spam"));
    assert!(!g.drop_synced());
    // taking one over by hand makes it yours
    f.mute("theirs", None);
    f.drop_synced();
    assert!(f.muted("theirs", 0) && f.muted("mine", 0) && !f.blocked("spam"));
  }
}
