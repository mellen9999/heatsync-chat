// what the chat hides, strips or lights up (ext main.js isMsgFiltered, keywordHighlights, mutedUsers,
// blockedUsers). decided per line at render time and kept off the data: turning a filter off brings the
// lines back. the lists live here, saved as one small `filters` file; the on/off switches are Settings
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::model::message::{Message, flag};
use crate::model::reasons::Reasons;
use crate::model::rules::{INKS, Judged, Line, Old, Rules, Verdict};
use crate::store;
use crate::ui::theme;
use crate::util::sound::{Preset, Voice};

// a line's verdict bits
pub const HIDDEN: u8 = 1;
// the sender is muted: their line stays (name, time) but the words and pictures go
pub const MUTED: u8 = 2;
// a highlight rule matched it
pub const MARKED: u8 = 4;
// a highlight rule's colour: 1 + its place in rules::INKS, in these three bits (0 = the plain highlight)
const INK_SHIFT: u32 = 3;
const INK_MASK: u8 = 7 << INK_SHIFT;

// the ink a marked line's edge is drawn in
pub fn mark_ink(look: u8) -> u32 {
  match (look & INK_MASK) >> INK_SHIFT {
    0 => theme::CYAN,
    n => INKS.get(usize::from(n) - 1).copied().unwrap_or(theme::CYAN),
  }
}

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
// an old file is read for this many words and patterns, no more
const MAX_OLD: usize = 1024;
const MAX_NAMES: usize = 5000;
// a rule's wav file: the longest path kept, and a rule id the file is kept under
const MAX_PATH: usize = crate::model::settings::PATH_MAX;
const MAX_RULE_ID: usize = 64;
pub const DEFAULT_MUTE_MS: i64 = 24 * 3600 * 1000;

#[derive(Default, Clone, Debug)]
pub struct Filters {
  pub bots: bool,
  pub cmds: bool,
  pub dupes: bool,
  // the ext's ban reasons (modBanReasons): the choices a ban offers
  reasons: Reasons,
  // the ext's per-rule hide / highlight list (chatFilterRules): hidden words, highlights and patterns live here
  rules: Rules,
  // words and patterns from an older file that did not fit in the rules yet: kept in the file, tried again on load
  old: Vec<Old>,
  // this load turned old words into rules: the file is written again without them
  folded: bool,
  // lowercase login → when the mute ends (None = until you lift it)
  mutes: HashMap<Box<str>, Option<i64>>,
  blocks: HashSet<Box<str>>,
  // entries that came from the account's server lists (not typed here): they leave with the login
  synced_mutes: HashSet<Box<str>>,
  synced_blocks: HashSet<Box<str>>,
  // your own names (lowercase): never hidden by these filters
  me: Vec<Box<str>>,
  // a highlight rule's own wav, by the rule's id. the rule only carries the file's name (it syncs), the path
  // stays on this device, in this file
  rule_files: HashMap<Box<str>, Box<str>>,
}

// what a highlight rule gives a live line: a sound (with its own loudness, else the shared one) and a ping
// into the mentions tab
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Cue {
  pub sound: Option<(Voice, Option<u8>)>,
  pub notify: bool,
}

// "@Bob" → "bob"; youtube handles carry a leading @ in chat but not in lists
pub fn key(name: &str) -> Box<str> {
  name.trim().trim_start_matches('@').to_lowercase().into()
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
  // `chan` is the channel tab the line is in, for the rules scoped to one
  pub fn judge(&self, m: &Message, chan: &str, now: i64, tab_hidden: &HashSet<Box<str>>, last: &mut Box<str>) -> u8 {
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
    // the rules. your own lines are never hidden; a hide rule that matches one also keeps a highlight
    // rule from lighting it
    let ruled = self.ruled(m, chan);
    if ruled.verdict == Verdict::Hide && !me {
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
    if ruled.verdict == Verdict::Mark {
      let at = ruled.ink.and_then(|c| INKS.iter().position(|&i| i == c)).map_or(0, |i| i as u8 + 1);
      look |= MARKED | at << INK_SHIFT;
    }
    look
  }

  fn ruled(&self, m: &Message, chan: &str) -> Judged {
    if self.rules.is_empty() {
      return Judged::default();
    }
    self.rules.judge(&Line::new(m, chan))
  }

  // the sound a verdict plays, with its own volume. a file the rule names that is not on this device plays the
  // ping instead (the ext does the same), so a synced rule is never silent
  fn voice(&self, j: &Judged) -> Option<(Voice, Option<u8>)> {
    let voice = match (j.sound, j.custom) {
      (Some(p), _) => Voice::Preset(p),
      (None, true) => self
        .rules
        .id_at(j.at)
        .and_then(|id| self.rule_files.get(id))
        .map_or(Voice::Preset(Preset::Ping), |p| Voice::File(p.to_string())),
      (None, false) => return None,
    };
    Some((voice, j.volume))
  }

  // what a highlight rule gives a new said line in `chan`
  pub fn rule_cue(&self, m: &Message, chan: &str) -> Cue {
    if !m.kind.said() {
      return Cue::default();
    }
    let j = self.ruled(m, chan);
    Cue { sound: self.voice(&j), notify: j.notify }
  }

  // what the rule at `at` plays (to hear it as it is set)
  pub fn rule_voice_at(&self, at: usize) -> Option<(Voice, Option<u8>)> {
    self.voice(&self.rules.judged(at)?)
  }

  pub fn rule_file(&self, id: &str) -> Option<&str> {
    self.rule_files.get(id).map(|p| &**p)
  }

  // keep (Some) or forget (None) where a rule's wav is. false = not kept: an id or path that is no good
  pub fn set_rule_file(&mut self, id: &str, path: Option<&str>) -> bool {
    let ok_id =
      (1..=MAX_RULE_ID).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !ok_id {
      return false;
    }
    let Some(p) = path else {
      self.rule_files.remove(id);
      return true;
    };
    let ok_path = !p.is_empty() && p.len() <= MAX_PATH && !p.chars().any(char::is_control);
    if !ok_path || (self.rule_files.len() >= crate::model::rules::MAX_RULES && !self.rule_files.contains_key(id)) {
      return false;
    }
    self.rule_files.insert(id.into(), p.into());
    true
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

  // the account's rules just replaced ours (`mine`, as text): rules folded here from old words that it lacks are put back
  pub fn keep_old_rules_of(&mut self, mine: &str) -> bool {
    Rules::parse(mine).is_some_and(|m| self.rules.keep_old_of(&m))
  }

  // the ext's chatFilterRules text in. false = unusable, the rules stay as they were
  pub fn set_rules_text(&mut self, t: &str) -> bool {
    Rules::parse(t).map(|r| self.rules = r).is_some()
  }

  // the file in. lines for hidden words, highlight words and automod patterns are from before the one rules list:
  // they are folded into the rules here (the same entry never twice), and the file is written again without them
  pub fn parse(s: &str) -> Self {
    let mut f = Self::default();
    let mut old = Vec::new();
    for l in s.lines() {
      let (kind, rest) = l.split_once(' ').unwrap_or((l, ""));
      match kind {
        "hide" | "mark" | "automod" | "caps" => {
          if old.len() < MAX_OLD {
            old.extend(Old::from_line(kind, rest));
          }
        }
        "banreason" => _ = f.reason(true, rest),
        "rules" => _ = f.set_rules_text(rest),
        "rulewav" => {
          if let Some((id, path)) = rest.split_once(' ') {
            f.set_rule_file(id, Some(path));
          }
        }
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
    f.folded = !old.is_empty();
    f.old = f.rules.fold(old);
    f
  }

  pub fn render(&self) -> String {
    let mut s = String::new();
    for o in &self.old {
      s.push_str(&o.line());
      s.push('\n');
    }
    for r in self.reasons.list() {
      s.push_str(&format!("banreason {r}\n"));
    }
    if !self.rules.is_empty() {
      s.push_str(&format!("rules {}\n", self.rules.json()));
    }
    // a file for a rule that is gone is not written again
    let mut files: Vec<_> = self.rule_files.iter().filter(|(id, _)| self.rules.has_id(id)).collect();
    files.sort_unstable();
    for (id, path) in files {
      s.push_str(&format!("rulewav {id} {path}\n"));
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

  // an older file is written again at once, so its words are rules from now on
  pub fn load() -> Self {
    let f = store::read("filters").map(|s| Self::parse(&s)).unwrap_or_default();
    if f.folded {
      f.save();
    }
    f
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
  use serde_json::json;

  fn line(login: &str, body: &str) -> Message {
    let mut m = Message::system(0, Platform::Twitch, "");
    m.kind = crate::model::message::Kind::Chat;
    m.login = login.into();
    m.nick = login.into();
    m.body = body.into();
    m
  }

  fn judge(f: &Filters, m: &Message) -> u8 {
    f.judge(m, "", 1000, &HashSet::new(), &mut Box::from(""))
  }

  #[test]
  fn bots_commands_and_words_hide() {
    let mut f = Filters::parse("hide Spoiler\n");
    (f.bots, f.cmds) = (true, true);
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
    assert_eq!(f.judge(&line("quiet", "x"), "", 3000, &HashSet::new(), &mut Box::from("")), 0);
    assert_eq!(f.next_expiry(1000), Some(2000));
    assert!(f.prune(3000));
    assert!(f.mutes().is_empty());
  }

  #[test]
  fn per_tab_hide() {
    let f = Filters::default();
    let hid: HashSet<Box<str>> = ["bob".into()].into();
    assert_eq!(f.judge(&line("Bob", "x"), "", 0, &hid, &mut Box::from("")), HIDDEN);
    assert_eq!(f.judge(&line("al", "x"), "", 0, &hid, &mut Box::from("")), 0);
  }

  #[test]
  fn duplicates_collapse_to_the_first() {
    let f = Filters { dupes: true, ..Filters::default() };
    let mut last: Box<str> = "".into();
    let none = HashSet::new();
    assert_eq!(f.judge(&line("a", "LUL"), "", 0, &none, &mut last), 0);
    assert_eq!(f.judge(&line("b", "LUL"), "", 0, &none, &mut last), HIDDEN);
    assert_eq!(f.judge(&line("c", "hi"), "", 0, &none, &mut last), 0);
    assert_eq!(f.judge(&line("d", "LUL"), "", 0, &none, &mut last), 0);
  }

  #[test]
  fn highlights_are_whole_words() {
    let f = Filters::parse("mark gg\nmark !drops\n");
    // a word that starts or ends in punctuation has no \b edge there, in the ext either: the shape is the ext's
    assert_eq!(judge(&f, &line("a", "GG wp")) & MARKED, MARKED);
    assert_eq!(judge(&f, &line("a", "oggy")), 0);
    assert_eq!(judge(&f, &line("a", "gg.")) & MARKED, MARKED);
    assert_eq!(judge(&f, &line("a", "got !drops")), 0);
    assert_eq!(judge(&f, &line("a", "got!drops")) & MARKED, MARKED);
  }

  #[test]
  fn special_characters_are_plain_text() {
    let f = Filters::parse("hide a.c(\n");
    assert_eq!(judge(&f, &line("x", "abc(")), 0);
    assert_eq!(judge(&f, &line("x", "A.C( yes")), HIDDEN);
  }

  #[test]
  fn file_roundtrip_and_limits() {
    let mut f = Filters::default();
    f.rules_mut().add("a".into(), "hide", "keyword", "spoil er").unwrap();
    f.block("troll");
    f.mute("quiet", Some(5));
    f.mute("perm", None);
    let g = Filters::parse(&f.render());
    assert_eq!(g.rules().json(), f.rules().json());
    assert!(g.blocked("troll"));
    assert_eq!(g.mutes(), vec![("perm", None), ("quiet", Some(5))]);
    assert_eq!(Filters::parse("garbage\nmute\nhide \n").mutes().len(), 0);
  }

  #[test]
  fn a_rules_colour_and_channel_reach_the_look() {
    let mut f = Filters::default();
    let rule = |color: &str, scope: &str| {
      json!([{ "id": "r", "enabled": true, "scope": scope, "color": color, "action": "highlight",
        "match": { "type": "keyword", "value": "gg" } }])
      .to_string()
    };
    assert!(f.set_rules_text(&rule("#ff0000", "forsen")));
    let at = |f: &Filters, chan: &str| f.judge(&line("a", "gg"), chan, 0, &HashSet::new(), &mut Box::from(""));
    assert_eq!(mark_ink(at(&f, "forsen")), theme::RED);
    assert_eq!(at(&f, "forsen") & MARKED, MARKED);
    assert_eq!(at(&f, "xqc"), 0, "another channel: no mark");
    // a rule with no colour of its own is the plain highlight
    assert_eq!(mark_ink(MARKED), theme::CYAN);
    // every palette colour comes back as itself
    for (i, c) in INKS.iter().enumerate() {
      assert_eq!(mark_ink(MARKED | (i as u8 + 1) << INK_SHIFT), *c);
    }
    // the sound is the rule's, for a said line in its channel only
    assert!(f.set_rules_text(&rule("#ff0000", "all").replace("\"color\"", "\"sound\":\"chime\",\"color\"")));
    let sound = |f: &Filters, m: &Message, c: &str| f.rule_cue(m, c).sound.map(|s| s.0);
    assert_eq!(sound(&f, &line("a", "gg"), "xqc"), Some(Voice::Preset(Preset::Chime)));
    assert_eq!(sound(&f, &line("a", "hi"), "xqc"), None);
    assert_eq!(sound(&f, &Message::system(0, Platform::Twitch, "gg"), "xqc"), None);
  }

  #[test]
  fn a_rules_wav_is_kept_here_and_never_outlives_the_rule() {
    let mut f = Filters::default();
    f.rules_mut().add("r1".into(), "highlight", "keyword", "gg").unwrap();
    f.rules_mut().set_sound("1", &crate::model::rules::Pick::File("ding.wav".into())).unwrap();
    let said = |f: &Filters| f.rule_cue(&line("a", "gg"), "").sound;
    // no file on this device yet: the ping, never silence
    assert_eq!(said(&f), Some((Voice::Preset(Preset::Ping), None)));
    assert!(f.set_rule_file("r1", Some("/home/me/ding.wav")));
    assert_eq!(said(&f), Some((Voice::File("/home/me/ding.wav".into()), None)));
    // it survives a restart, in the local file
    let g = Filters::parse(&f.render());
    assert_eq!(g.rule_file("r1"), Some("/home/me/ding.wav"));
    assert!(f.render().contains("rulewav r1 /home/me/ding.wav\n"));
    // bounds: a bad id, a control byte, a path too long, an empty one
    for (id, p) in
      [("a b", "/x.wav"), ("", "/x.wav"), ("r1", ""), ("r1", "/x\n.wav"), ("r1", &"y".repeat(MAX_PATH + 1)[..])]
    {
      assert!(!f.set_rule_file(id, Some(p)), "{id:?} {p:?}");
    }
    assert_eq!(f.rule_file("r1"), Some("/home/me/ding.wav"), "a refusal keeps what was there");
    // the rule goes, and its file is not written again
    f.rules_mut().remove("1");
    assert!(!f.render().contains("rulewav"));
    // hostile lines in the file
    let h = Filters::parse("rulewav\nrulewav only-id\nrulewav bad!id /x.wav\nrulewav ok /x.wav\n");
    assert_eq!(h.rule_files.len(), 1);
  }

  #[test]
  fn a_rule_can_ask_for_the_mentions_tab() {
    let mut f = Filters::default();
    f.rules_mut().add("r".into(), "highlight", "keyword", "gg").unwrap();
    assert!(!f.rule_cue(&line("a", "gg"), "").notify);
    f.rules_mut().set_notify("1", true).unwrap();
    assert!(f.rule_cue(&line("a", "gg"), "").notify);
    assert!(!f.rule_cue(&line("a", "hello"), "").notify);
    assert!(!f.rule_cue(&Message::system(0, Platform::Twitch, "gg"), "").notify);
  }

  #[test]
  fn the_rules_share_the_one_verdict() {
    let mut f = Filters::parse("caps\n");
    f.rules_mut().add("p".into(), "hide", "regex", r"bit\.ly").unwrap();
    f.rules_mut().add("a".into(), "hide", "keyword", "nope").unwrap();
    f.rules_mut().add("b".into(), "highlight", "keyword", "gg").unwrap();
    let shout = "STOP SHOUTING AT ME PLEASE";
    assert_eq!(judge(&f, &line("a", shout)), HIDDEN);
    assert_eq!(judge(&f, &line("a", "go to BIT.LY now")), HIDDEN);
    assert_eq!(judge(&f, &line("a", "oh nope")), HIDDEN);
    assert_eq!(judge(&f, &line("a", "gg")), MARKED | 3 << 3);
    assert_eq!(judge(&f, &line("a", "fine")), 0);
    // a mod is not spared, only you are
    let mut m = line("a", shout);
    m.badges = Box::new([("moderator".into(), "1".into())]);
    assert_eq!(judge(&f, &m), HIDDEN);
    f.set_me(&["me".into()]);
    assert_eq!(judge(&f, &line("me", shout)), 0);
    assert_eq!(judge(&f, &line("me", "bit.ly nope")), 0);
    // a highlight rule still lights your own line, a hide rule that matches it keeps it plain
    assert_eq!(judge(&f, &line("me", "gg")), MARKED | 3 << 3);
    assert_eq!(judge(&f, &line("me", "gg nope")), 0);
    // a hide rule beats the highlight rules
    assert_eq!(judge(&f, &line("a", "gg nope")), HIDDEN);
    // turned off, the lines come back
    assert!(f.rules_mut().remove("1"));
    assert_eq!(judge(&f, &line("a", shout)), 0);
    assert!(f.rules_mut().remove("p"));
    assert_eq!(judge(&f, &line("a", "bit.ly")), 0);
    assert!(f.rules_mut().remove("1"));
    assert_eq!(judge(&f, &line("a", "oh nope")), 0);
    // system lines are never judged
    assert_eq!(judge(&f, &Message::system(0, Platform::Twitch, shout)), 0);
  }

  #[test]
  fn rules_survive_the_file_and_the_account_text() {
    let mut f = Filters::default();
    f.rules_mut().add("p".into(), "hide", "regex", r"free\s+v-?bucks").unwrap();
    f.rules_mut().add("a".into(), "hide", "expr", "user:bob && !first").unwrap();
    let g = Filters::parse(&f.render());
    assert_eq!(g.rules().json(), f.rules().json());
    assert_eq!(judge(&g, &line("bob", "hi")), HIDDEN);
    assert_eq!(judge(&g, &line("al", "FREE   vbucks")), HIDDEN);
    // the account's text in: unusable text changes nothing
    let mut h = Filters::default();
    assert!(h.set_rules_text(&f.rules().json()) && h.rules().len() == 2);
    assert!(!h.set_rules_text("{\"not\":\"an array\"}") && h.rules().len() == 2);
    // a hostile file line is skipped, not fatal
    let junk = Filters::parse("rules [[[[\nrules {}\nautomod \nhide \nmark \u{7}\n");
    assert_eq!(junk.rules().len(), 0);
  }

  // hidden words, highlight words and automod patterns became rules: all three kinds in one old file, kept
  // behaving the way they did, the file written again once, and a second load adding nothing
  #[test]
  fn old_words_and_patterns_fold_into_rules_once_and_lose_nothing() {
    let old = "hide spoil\nmark gg\nautomod buy \\d+ followers\nautomod oh(no\nblock troll\nrules []\n";
    let f = Filters::parse(old);
    assert!(f.folded && f.old.is_empty());
    assert_eq!(f.rules().len(), 4);
    // a hidden word hides anywhere in a line, a highlight word is a whole word, a pattern is a regex
    assert_eq!(judge(&f, &line("a", "no SPOILERS here")), HIDDEN);
    assert_eq!(judge(&f, &line("a", "gg wp")) & MARKED, MARKED);
    assert_eq!(judge(&f, &line("a", "oggy")), 0);
    assert_eq!(judge(&f, &line("a", "BUY 500 Followers")), HIDDEN);
    assert_eq!(judge(&f, &line("a", "well OH(NO then")), HIDDEN, "a pattern that is not a regex is text, as before");
    assert!(f.blocked("troll"), "everything else in the file stays");
    // rewritten without the old lines; loading that, or the old file again, adds no second copy
    let now = f.render();
    assert!(!now.contains("hide spoil") && !now.contains("mark gg") && !now.contains("automod"));
    let again = Filters::parse(&now);
    assert!(!again.folded);
    assert_eq!(again.rules().json(), f.rules().json());
    let both = Filters::parse(&format!("{old}{now}"));
    assert_eq!(both.rules().json(), f.rules().json(), "a file with both (a crash between) stays one set");
    // the same entry typed twice is one rule
    assert_eq!(Filters::parse("hide x\nhide X\n").rules().len(), 1);
  }

  #[test]
  fn the_filters_file_is_written_again_without_its_old_lines_on_load() {
    store::write("filters", "hide spoil\nmark gg\nautomod bit\\.ly\n").unwrap();
    let f = Filters::load();
    assert_eq!(f.rules().len(), 3);
    let on_disk = store::read("filters").unwrap();
    assert!(on_disk.starts_with("rules ") && !on_disk.contains("hide spoil"));
    // a second start finds the rules and nothing to fold
    assert_eq!(Filters::load().rules().json(), f.rules().json());
  }

  // a full rules list keeps the rest in the file, never drops them, and tries again on the next load
  #[test]
  fn what_does_not_fit_in_the_rules_stays_in_the_file() {
    let words: String = (0..260).map(|i| format!("hide word{i}\n")).collect();
    let f = Filters::parse(&words);
    assert_eq!(f.rules().len(), crate::model::rules::MAX_RULES);
    assert_eq!(f.old.len(), 60);
    let out = f.render();
    assert_eq!(out.lines().filter(|l| l.starts_with("hide ")).count(), 60);
    let g = Filters::parse(&out);
    assert_eq!((g.rules().len(), g.old.len()), (crate::model::rules::MAX_RULES, 60));
    // room made: they fold in on the next load
    let mut h = g.clone();
    for i in 0..60 {
      assert!(h.rules_mut().remove("1"), "{i}");
    }
    let h = Filters::parse(&h.render());
    assert_eq!((h.rules().len(), h.old.len()), (crate::model::rules::MAX_RULES, 0));
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
