// 7tv personal emotes: the sets 7tv says a chatter owns (a subscriber's own set, or an event set they were
// given), kept per OWNER's twitch id. same privacy invariant as a heatsync inventory: a personal emote draws
// only on a line THAT chatter sent, is never shown to anyone else, and is never offered to anyone else in
// completion or the picker. its only inputs are 7tv's entitlements (`entitle`); a search or a catalog never
// reaches this store. bounded in every direction: owners (lru), sets, and emotes across all of them
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::entitle::MAX_SET_EMOTES;
use super::live::{Delta, patch_set};
use super::{Emote, Set};

// owners kept, least recently seen out first (a busy channel shows a few hundred a minute)
const MAX_USERS: usize = 1000;
const MAX_SETS: usize = 1500;
// every emote of every set held: ~150 bytes each, so about 600KB at the most
const MAX_EMOTES: usize = 4000;
// sets one owner can have (their own and the event sets they were given)
const MAX_USER_SETS: usize = 6;
// owners one entitlement can name
const MAX_OWNERS: usize = 4;
// a set heard about again after this is read again; one that failed waits this long to be asked again
const TTL: Duration = Duration::from_secs(6 * 3600);
const RETRY: Duration = Duration::from_secs(60);

// what the net side tells us (net/seventv.rs), already parsed and bounded by `entitle`
#[derive(Debug)]
pub enum Ev {
  // these twitch accounts own this set / no longer do
  Grant { users: Vec<u64>, set: Box<str> },
  Revoke { users: Vec<u64>, set: Box<str> },
  // a set read whole
  Set(Box<str>, Set),
  // a live edit of a set
  Delta(Box<str>, Delta),
  // we were deaf for a while: every set may have moved on
  Stale,
}

enum Ask {
  // no content, or content that is due a read
  Need,
  Asked(Instant),
  Have(Instant),
}

struct Entry {
  emotes: Option<Set>,
  // owners that have it
  refs: u32,
  ask: Ask,
}

struct User {
  sets: Vec<Box<str>>,
  seen: u64,
}

#[derive(Default)]
pub struct Personal {
  on: bool,
  users: HashMap<u64, User>,
  sets: HashMap<Box<str>, Entry>,
  total: usize,
  clock: u64,
  wanted: Vec<Box<str>>,
}

fn same(a: &Set, b: &Set) -> bool {
  a.len() == b.len()
    && a.iter().all(|(k, x)| {
      b.get(k).is_some_and(|y| x.url == y.url && (x.w, x.h) == (y.w, y.h) && x.zero_width == y.zero_width)
    })
}

impl Personal {
  // the switch (`personal_emotes`). off holds nothing and shows nothing
  pub fn set_on(&mut self, on: bool) {
    self.on = on;
    if !on {
      *self = Self::default();
    }
  }

  // `word` as `user` owns it, for a line they sent. the owner is the only key: nobody else's line asks this.
  // a set without content (or past its time) is queued to be read (`take_wanted`)
  pub fn lookup(&mut self, user: u64, word: &str) -> Option<&Emote> {
    let Self { on, users, sets, clock, wanted, .. } = self;
    if !*on {
      return None;
    }
    let u = users.get_mut(&user)?;
    *clock += 1;
    u.seen = *clock;
    for id in &u.sets {
      let Some(e) = sets.get_mut(id) else { continue };
      let due = match e.ask {
        Ask::Have(_) => false,
        Ask::Need => true,
        Ask::Asked(t) => t.elapsed() > RETRY,
      };
      if due {
        e.ask = Ask::Asked(Instant::now());
        wanted.push(id.clone());
      }
    }
    let sets: &HashMap<Box<str>, Entry> = sets;
    u.sets.iter().find_map(|id| sets.get(id)?.emotes.as_ref()?.get(word))
  }

  // the same without asking for anything or marking the owner seen (completion, the box)
  pub fn peek(&self, user: u64, word: &str) -> Option<&Emote> {
    if !self.on {
      return None;
    }
    self.users.get(&user)?.sets.iter().find_map(|id| self.sets.get(id)?.emotes.as_ref()?.get(word))
  }

  // everything `user` owns, a name once, in name order (their own completion and picker only)
  pub fn entries(&self, user: u64) -> Vec<(&str, &Emote)> {
    let Some(u) = self.users.get(&user).filter(|_| self.on) else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<(&str, &Emote)> = u
      .sets
      .iter()
      .filter_map(|id| self.sets.get(id)?.emotes.as_ref())
      .flatten()
      .map(|(k, v)| (&**k, v))
      .filter(|(k, _)| seen.insert(*k))
      .collect();
    out.sort_by_cached_key(|(k, _)| k.to_lowercase());
    out
  }

  // sets to read now (net: `Cmd::PersonalSets`)
  pub fn take_wanted(&mut self) -> Vec<Box<str>> {
    let mut v = std::mem::take(&mut self.wanted);
    v.sort_unstable();
    v.dedup();
    v
  }

  // an event from the net side. the owners whose lines may now draw differently
  pub fn apply(&mut self, ev: Ev) -> Vec<u64> {
    if !self.on {
      return Vec::new();
    }
    match ev {
      Ev::Grant { users, set } => self.grant(&users, &set),
      Ev::Revoke { users, set } => self.revoke(&users, &set),
      Ev::Set(set, emotes) => self.fill(&set, emotes),
      Ev::Delta(set, d) => self.edit(&set, &d),
      Ev::Stale => {
        for e in self.sets.values_mut() {
          if matches!(e.ask, Ask::Have(_)) {
            e.ask = Ask::Need;
          }
        }
        Vec::new()
      }
    }
  }

  fn grant(&mut self, users: &[u64], set: &str) -> Vec<u64> {
    let mut touched = Vec::new();
    for &id in users.iter().take(MAX_OWNERS) {
      self.clock += 1;
      let u = self.users.entry(id).or_insert(User { sets: Vec::new(), seen: 0 });
      u.seen = self.clock;
      if u.sets.iter().any(|s| &**s == set) {
        continue;
      }
      let dropped = (u.sets.len() >= MAX_USER_SETS).then(|| u.sets.remove(0));
      u.sets.push(set.into());
      self.sets.entry(set.into()).or_insert(Entry { emotes: None, refs: 0, ask: Ask::Need }).refs += 1;
      if let Some(old) = dropped {
        self.unref(&old);
      }
      touched.push(id);
    }
    // heard again after its time: read again (what it held keeps drawing meanwhile)
    if let Some(e) = self.sets.get_mut(set)
      && matches!(e.ask, Ask::Have(t) if t.elapsed() > TTL)
    {
      e.ask = Ask::Need;
    }
    self.trim();
    // lines have nothing new to show until the set has something in it
    if self.sets.get(set).and_then(|e| e.emotes.as_ref()).is_some_and(|s| !s.is_empty()) { touched } else { Vec::new() }
  }

  fn revoke(&mut self, users: &[u64], set: &str) -> Vec<u64> {
    let mut touched = Vec::new();
    for &id in users.iter().take(MAX_OWNERS) {
      let Some(u) = self.users.get_mut(&id) else { continue };
      let Some(at) = u.sets.iter().position(|s| &**s == set) else { continue };
      u.sets.remove(at);
      if u.sets.is_empty() {
        self.users.remove(&id);
      }
      self.unref(set);
      touched.push(id);
    }
    touched
  }

  fn fill(&mut self, set: &str, emotes: Set) -> Vec<u64> {
    // a set nobody here owns (an answer that came late, or one we never asked for) is not kept
    let Some(e) = self.sets.get_mut(set) else { return Vec::new() };
    let emotes: Set = emotes.into_iter().take(MAX_SET_EMOTES).collect();
    e.ask = Ask::Have(Instant::now());
    let unchanged = e.emotes.as_ref().is_some_and(|old| same(old, &emotes));
    let now = emotes.len();
    let old = e.emotes.replace(emotes).map_or(0, |s| s.len());
    self.total = self.total - old + now;
    self.trim();
    if unchanged { Vec::new() } else { self.owners(set) }
  }

  fn edit(&mut self, set: &str, d: &Delta) -> Vec<u64> {
    // no content yet: the read that is coming brings the whole set
    let Some(content) = self.sets.get_mut(set).and_then(|e| e.emotes.as_mut()) else { return Vec::new() };
    let before = content.len();
    let changed = patch_set(content, d, MAX_SET_EMOTES);
    self.total = self.total - before + content.len();
    self.trim();
    if changed { self.owners(set) } else { Vec::new() }
  }

  fn owners(&self, set: &str) -> Vec<u64> {
    self.users.iter().filter(|(_, u)| u.sets.iter().any(|s| &**s == set)).map(|(&id, _)| id).collect()
  }

  fn unref(&mut self, set: &str) {
    let Some(e) = self.sets.get_mut(set) else { return };
    e.refs = e.refs.saturating_sub(1);
    if e.refs == 0 {
      self.total -= e.emotes.as_ref().map_or(0, Set::len);
      self.sets.remove(set);
    }
  }

  // back under every bound, the owner seen longest ago first
  fn trim(&mut self) {
    while self.users.len() > MAX_USERS || self.sets.len() > MAX_SETS || self.total > MAX_EMOTES {
      let Some(lru) = self.users.iter().min_by_key(|(_, u)| u.seen).map(|(&id, _)| id) else { break };
      if let Some(u) = self.users.remove(&lru) {
        u.sets.iter().for_each(|s| self.unref(s));
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn emote(id: &str) -> Emote {
    Emote { url: format!("https://cdn.7tv.app/emote/{id}/1x.webp").into(), w: 28, h: 28, zero_width: false }
  }

  fn set(names: &[&str]) -> Set {
    names.iter().map(|n| ((*n).into(), emote(&format!("id{n}")))).collect()
  }

  fn grant(users: &[u64], set: &str) -> Ev {
    Ev::Grant { users: users.to_vec(), set: set.into() }
  }

  fn world(on: bool) -> Personal {
    let mut p = Personal::default();
    p.set_on(on);
    p
  }

  fn url(p: &mut Personal, user: u64, word: &str) -> Option<String> {
    p.lookup(user, word).map(|e| e.url.to_string())
  }

  #[test]
  fn a_set_is_its_owners_and_nobodys_else() {
    let mut p = world(true);
    p.apply(grant(&[5], "S1"));
    p.apply(Ev::Set("S1".into(), set(&["DonkMonk"])));
    assert_eq!(url(&mut p, 5, "DonkMonk").as_deref(), Some("https://cdn.7tv.app/emote/idDonkMonk/1x.webp"));
    assert!(url(&mut p, 6, "DonkMonk").is_none(), "another sender");
    assert!(url(&mut p, 5, "donkmonk").is_none(), "exact case");
    assert!(p.peek(6, "DonkMonk").is_none());
    assert!(p.entries(6).is_empty(), "never offered to anyone else");
    assert_eq!(p.entries(5).len(), 1);
  }

  #[test]
  fn off_holds_and_shows_nothing() {
    let mut p = world(false);
    assert!(p.apply(grant(&[5], "S1")).is_empty());
    p.apply(Ev::Set("S1".into(), set(&["Kek"])));
    assert!(url(&mut p, 5, "Kek").is_none() && p.peek(5, "Kek").is_none() && p.entries(5).is_empty());
    assert!(p.users.is_empty() && p.sets.is_empty());
    // turning it off drops what was held, and on again starts empty
    p.set_on(true);
    p.apply(grant(&[5], "S1"));
    p.apply(Ev::Set("S1".into(), set(&["Kek"])));
    assert!(url(&mut p, 5, "Kek").is_some());
    p.set_on(false);
    p.set_on(true);
    assert!(url(&mut p, 5, "Kek").is_none() && p.total == 0);
  }

  #[test]
  fn a_set_is_read_when_missing_and_again_after_a_failure() {
    let mut p = world(true);
    p.apply(grant(&[5], "S1"));
    assert!(p.take_wanted().is_empty(), "nothing asked until a line of theirs is drawn");
    p.lookup(5, "x");
    p.lookup(5, "y");
    assert_eq!(p.take_wanted(), [Box::<str>::from("S1")], "once, however many words");
    p.lookup(5, "z");
    assert!(p.take_wanted().is_empty(), "in flight");
    // no answer: asked again after the wait, not before
    if let Some(e) = p.sets.get_mut("S1") {
      e.ask = Ask::Asked(Instant::now() - RETRY - Duration::from_secs(1));
    }
    p.lookup(5, "z");
    assert_eq!(p.take_wanted().len(), 1);
    // answered: not asked again
    p.apply(Ev::Set("S1".into(), set(&["Kek"])));
    p.lookup(5, "z");
    assert!(p.take_wanted().is_empty());
    // a reconnect makes every set due; what it held keeps drawing meanwhile
    p.apply(Ev::Stale);
    assert!(url(&mut p, 5, "Kek").is_some());
    assert_eq!(p.take_wanted().len(), 1);
  }

  #[test]
  fn an_answer_nobody_asked_for_is_dropped() {
    let mut p = world(true);
    assert!(p.apply(Ev::Set("S9".into(), set(&["Kek"]))).is_empty());
    assert!(p.sets.is_empty() && p.total == 0);
    p.apply(grant(&[5], "S1"));
    p.apply(Ev::Revoke { users: vec![5], set: "S1".into() });
    assert!(p.apply(Ev::Set("S1".into(), set(&["Kek"]))).is_empty(), "the owner is gone");
    assert!(p.sets.is_empty());
  }

  #[test]
  fn lines_are_laid_out_again_only_when_something_new_can_draw() {
    let mut p = world(true);
    assert!(p.apply(grant(&[5], "S1")).is_empty(), "no content yet");
    assert_eq!(p.apply(Ev::Set("S1".into(), set(&["Kek"]))), [5]);
    assert!(p.apply(Ev::Set("S1".into(), set(&["Kek"]))).is_empty(), "the same set again");
    assert_eq!(p.apply(grant(&[6], "S1")), [6], "a second owner of a set we hold");
    let mut owners = p.apply(Ev::Set("S1".into(), set(&["Kek", "Pog"])));
    owners.sort_unstable();
    assert_eq!(owners, [5, 6]);
    assert_eq!(p.apply(Ev::Revoke { users: vec![6], set: "S1".into() }), [6]);
    assert!(url(&mut p, 6, "Kek").is_none());
    assert!(url(&mut p, 5, "Kek").is_some());
  }

  fn delta(add: &[(&str, &str)], remove: &[&str]) -> Delta {
    Delta {
      add: add.iter().map(|(id, n)| ((*id).into(), (*n).into(), emote(id))).collect(),
      remove: remove.iter().map(|id| (*id).into()).collect(),
      rename: Vec::new(),
    }
  }

  #[test]
  fn live_edits_change_a_held_set_and_are_ignored_for_one_we_do_not_hold() {
    let mut p = world(true);
    assert!(p.apply(Ev::Delta("S1".into(), delta(&[("A", "Kek")], &[]))).is_empty(), "no such set");
    p.apply(grant(&[5], "S1"));
    assert!(
      p.apply(Ev::Delta("S1".into(), delta(&[("A", "Kek")], &[]))).is_empty(),
      "not read yet: the read brings it all"
    );
    assert!(p.sets["S1"].emotes.is_none());
    p.apply(Ev::Set("S1".into(), set(&["Old"])));
    assert_eq!(p.apply(Ev::Delta("S1".into(), delta(&[("A", "Kek")], &[]))), [5]);
    assert!(url(&mut p, 5, "Kek").is_some());
    assert_eq!(p.total, 2);
    assert_eq!(p.apply(Ev::Delta("S1".into(), delta(&[], &["A"]))), [5]);
    assert!(url(&mut p, 5, "Kek").is_none());
    assert_eq!(p.total, 1);
    assert!(p.apply(Ev::Delta("S1".into(), delta(&[], &["nope"]))).is_empty(), "removing what is not there");
  }

  #[test]
  fn a_set_shared_by_owners_is_held_once_and_goes_with_the_last() {
    let mut p = world(true);
    p.apply(grant(&[5, 6, 7], "EVENT"));
    p.apply(Ev::Set("EVENT".into(), set(&["a", "b", "c"])));
    assert_eq!((p.sets.len(), p.total), (1, 3));
    for u in [5, 6] {
      p.apply(Ev::Revoke { users: vec![u], set: "EVENT".into() });
      assert_eq!(p.total, 3);
    }
    p.apply(Ev::Revoke { users: vec![7], set: "EVENT".into() });
    assert_eq!((p.sets.len(), p.users.len(), p.total), (0, 0, 0));
  }

  #[test]
  fn the_first_set_of_a_name_wins_for_an_owner_with_several() {
    let mut p = world(true);
    p.apply(grant(&[5], "OWN"));
    p.apply(grant(&[5], "EVENT"));
    p.apply(Ev::Set("OWN".into(), [("Kek".into(), emote("mine"))].into()));
    p.apply(Ev::Set("EVENT".into(), [("Kek".into(), emote("theirs")), ("Pog".into(), emote("pog"))].into()));
    assert_eq!(url(&mut p, 5, "Kek").as_deref(), Some("https://cdn.7tv.app/emote/mine/1x.webp"));
    assert!(url(&mut p, 5, "Pog").is_some());
    let names: Vec<&str> = p.entries(5).iter().map(|e| e.0).collect();
    assert_eq!(names, ["Kek", "Pog"], "a name once");
  }

  #[test]
  fn owners_are_evicted_least_recently_seen_first() {
    let mut p = world(true);
    for u in 1..=MAX_USERS as u64 {
      p.apply(grant(&[u], &format!("S{u}")));
      p.apply(Ev::Set(format!("S{u}").into(), set(&["Kek"])));
    }
    assert_eq!(p.users.len(), MAX_USERS);
    // user 1 speaks again, so user 2 is the one that goes when a new owner arrives
    assert!(url(&mut p, 1, "Kek").is_some());
    p.apply(grant(&[100_000], "NEW"));
    assert_eq!(p.users.len(), MAX_USERS);
    assert!(p.users.contains_key(&1) && !p.users.contains_key(&2) && p.users.contains_key(&100_000));
    assert!(!p.sets.contains_key("S2"), "their set went with them");
    assert!(url(&mut p, 2, "Kek").is_none());
  }

  #[test]
  fn emotes_held_stay_under_the_cap() {
    let mut p = world(true);
    let names: Vec<String> = (0..MAX_SET_EMOTES).map(|i| format!("e{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    for u in 1..=200u64 {
      p.apply(grant(&[u], &format!("S{u}")));
      // a set bigger than the per-set cap is cut first
      let mut big = set(&refs);
      big.extend((0..50).map(|i| (format!("x{i}").into(), emote("x"))));
      p.apply(Ev::Set(format!("S{u}").into(), big));
      assert!(p.total <= MAX_EMOTES, "{u}: {}", p.total);
      assert_eq!(p.total, p.sets.values().map(|e| e.emotes.as_ref().map_or(0, Set::len)).sum::<usize>());
    }
    assert!(p.users.len() < 200, "the oldest owners made room: {}", p.users.len());
    assert!(p.users.contains_key(&200), "the newest stays");
    assert!(p.sets.len() <= MAX_SETS);
  }

  #[test]
  fn sets_per_owner_and_owners_per_event_are_bounded() {
    let mut p = world(true);
    for i in 0..20 {
      p.apply(grant(&[5], &format!("S{i}")));
    }
    assert_eq!(p.users[&5].sets.len(), MAX_USER_SETS);
    assert_eq!(p.sets.len(), MAX_USER_SETS, "a set dropped by its last owner is gone");
    let many: Vec<u64> = (100..200).collect();
    p.apply(grant(&many, "BIG"));
    assert_eq!(p.sets["BIG"].refs as usize, MAX_OWNERS);
  }
}
