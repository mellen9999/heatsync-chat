// your channel tabs follow your heatsync account, like the ext's multichat config: ws `multichat:config
// {channels, rev}` in (right after sign-in, on `multichat:pull`, and whenever any of your clients saves),
// `multichat:sync {channels}` out. the account's list is the ext's channel objects `{id, twitch, kick,
// youtube, …}`; this file maps them onto tabs and back, and holds the rules:
// - every frame is hostile: one whose list isn't a list is dropped whole; an entry the ext's own check
//   (isValidSyncedChannel) refuses is dropped, as it is there; counts and sizes are bounded
// - a frame older than the newest already applied for the same account is dropped (the ext's isStaleConfigRev)
// - the account's list wins: on sign-in it replaces the tabs you had, like the ext's (never merged)
// - entries go back as they came: fields this app has no use for, and feeds it can't open, ride along
//   untouched; an entry with nothing this app can open keeps its place in the list without a tab
use std::collections::VecDeque;

use serde_json::{Map, Value, json};

use crate::model::chan::{self, Spec};

// the server refuses a longer list (handleMultichatSync)
pub const MAX_CHANNELS: usize = 50;
// an id is a channel name or `yt-<ms>`; the whole entry a few dozen bytes plus whatever another client adds
const MAX_ID: usize = 64;
const MAX_ENTRY: usize = 2048;
// lists sent and not echoed yet
const MAX_PENDING: usize = 8;

pub type Raw = Map<String, Value>;

// the newest rev applied, and for which account (the ext's mcConfigRevSeen)
#[derive(Debug, PartialEq)]
pub struct Seen {
  user: Option<Box<str>>,
  rev: f64,
}

impl Default for Seen {
  fn default() -> Self {
    Seen { user: None, rev: -1.0 }
  }
}

// the ext's isStaleConfigRev: a server read races the write of a push, so a frame read before a write can land
// after its echo and lay the old list back. older than the newest applied for this account = stale. no stamp
// (an older server) always applies and leaves the mark alone. marks the rev seen when it isn't stale
pub fn is_stale(seen: &mut Seen, user: &str, rev: Option<f64>) -> bool {
  let Some(rev) = rev.filter(|r| r.is_finite()) else { return false };
  if seen.user.as_deref() != Some(user) {
    seen.user = Some(user.into());
    seen.rev = -1.0;
  }
  if rev < seen.rev {
    return true;
  }
  seen.rev = rev;
  false
}

// what a `multichat:config` frame carries once checked
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
  pub list: Vec<Raw>,
  pub rev: Option<f64>,
}

// None = no list in it: dropped whole, nothing changes
pub fn frame(v: &Value) -> Option<Frame> {
  let list = v.get("channels")?.as_array()?;
  let list = list.iter().filter(|c| valid(c)).filter_map(Value::as_object).take(MAX_CHANNELS).cloned().collect();
  Some(Frame { list, rev: v.get("rev").and_then(Value::as_f64) })
}

// js truthiness, which the ext's checks lean on: a leg that is null, false, 0 or "" is just absent
fn truthy(v: Option<&Value>) -> bool {
  match v {
    None | Some(Value::Null) => false,
    Some(Value::Bool(b)) => *b,
    Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
    Some(Value::String(s)) => !s.is_empty(),
    Some(_) => true,
  }
}

fn word(s: &str, max: usize, extra: &[u8]) -> bool {
  (1..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || extra.contains(&b))
}

// the ext's isValidSyncedChannel (twitch is sent to irc: login-shaped; kick adds `-`; youtube a bare handle
// or UC id, or a full https youtube url; no line breaks), plus the server's rule that every entry has a
// string id, and our size bounds
pub fn valid(ch: &Value) -> bool {
  let Some(o) = ch.as_object() else { return false };
  let id_ok = o
    .get("id")
    .and_then(Value::as_str)
    .is_some_and(|i| (1..=MAX_ID).contains(&i.len()) && !i.contains(char::is_control));
  if !id_ok || serde_json::to_string(o).map_or(true, |s| s.len() > MAX_ENTRY) {
    return false;
  }
  let leg = |k: &str, ok: &dyn Fn(&str) -> bool| !truthy(o.get(k)) || o.get(k).and_then(Value::as_str).is_some_and(ok);
  leg("twitch", &|s| word(s, 25, b""))
    && leg("kick", &|s| word(s, 25, b"-"))
    && leg("youtube", &|s| {
      let bare = word(s.strip_prefix('@').unwrap_or(s), 100, b".-");
      let lower = s.get(..24).unwrap_or(s).to_ascii_lowercase();
      let url = lower.starts_with("https://youtube.com/") || lower.starts_with("https://www.youtube.com/");
      !s.contains(['\r', '\n']) && (bare || url)
    })
}

fn text<'a>(o: &'a Raw, k: &str) -> &'a str {
  o.get(k).and_then(Value::as_str).unwrap_or("")
}

// the feeds of an entry this app can open (a leg it can't, a UC id say, is left out here and kept in the entry)
pub fn spec(o: &Raw) -> Spec {
  Spec::lenient(text(o, "twitch"), text(o, "kick"), text(o, "youtube"))
}

fn has_feed(s: &Spec) -> bool {
  !s.name().is_empty()
}

// one entry of the account's list. `tab`: it has a tab here (false: nothing this app can open, or a feed an
// earlier entry already has; it rides along in place)
#[derive(Debug, Clone, PartialEq)]
struct Entry {
  raw: Raw,
  tab: bool,
}

// the account's tab list as this window last agreed with it, and the sends not echoed yet
#[derive(Default)]
pub struct TabSync {
  seen: Seen,
  list: Vec<Entry>,
  pending: VecDeque<Vec<Value>>,
  // a change made while the socket was down: it goes up when the socket signs in again
  pub owed: bool,
}

impl TabSync {
  fn set_list(&mut self, list: Vec<Raw>) {
    let mut taken: Vec<Spec> = Vec::with_capacity(list.len());
    self.list = list
      .into_iter()
      .map(|raw| {
        let s = spec(&raw);
        let tab = has_feed(&s) && s.check(&taken).is_ok();
        if tab {
          taken.push(s);
        }
        Entry { raw, tab }
      })
      .collect();
  }

  fn values(&self) -> Vec<Value> {
    self.list.iter().map(|e| Value::Object(e.raw.clone())).collect()
  }

  // the channel tabs the account's list makes, in its order
  pub fn tabs(&self) -> Vec<Spec> {
    self.list.iter().filter(|e| e.tab).map(|e| spec(&e.raw)).collect()
  }

  // a frame from the account: Some(the tabs to show) when it changes them, None when it is stale or our own
  // echo. `user` is who the socket is signed in as
  pub fn inbound(&mut self, user: &str, f: Frame) -> Option<Vec<Spec>> {
    if is_stale(&mut self.seen, user, f.rev) {
      return None;
    }
    let vals: Vec<Value> = f.list.iter().cloned().map(Value::Object).collect();
    // the echo of a send of ours: the newest one is already what we show; an older one would undo the
    // sends after it, which are on their way
    if let Some(i) = self.pending.iter().position(|p| *p == vals) {
      self.pending.drain(..=i);
      return None;
    }
    self.pending.clear();
    self.set_list(f.list);
    Some(self.tabs())
  }

  // the list to send for your channel tabs (in order), or None when the account has it already
  pub fn outbound(&mut self, tabs: &[Spec], now_ms: i64) -> Option<Vec<Value>> {
    let next = self.build(tabs, now_ms);
    let vals: Vec<Value> = next.iter().cloned().map(Value::Object).collect();
    if vals == self.values() {
      return None;
    }
    self.set_list(next);
    if self.pending.len() == MAX_PENDING {
      self.pending.pop_front();
    }
    self.pending.push_back(vals.clone());
    Some(vals)
  }

  fn build(&self, tabs: &[Spec], now_ms: i64) -> Vec<Raw> {
    let mut used = vec![false; self.list.len()];
    let mut out: Vec<Option<Raw>> = vec![None; tabs.len()];
    // the same feeds: the entry as it is
    for (t, slot) in tabs.iter().zip(out.iter_mut()) {
      if let Some(i) = (0..self.list.len()).find(|&i| !used[i] && self.list[i].tab && spec(&self.list[i].raw) == *t) {
        used[i] = true;
        *slot = Some(self.list[i].raw.clone());
      }
    }
    // an edited tab: the entry it shares a feed with keeps its other fields
    for (t, slot) in tabs.iter().zip(out.iter_mut()).filter(|(_, s)| s.is_none()) {
      if let Some(i) =
        (0..self.list.len()).find(|&i| !used[i] && self.list[i].tab && shares(&spec(&self.list[i].raw), t))
      {
        used[i] = true;
        let others: Vec<&str> =
          self.list.iter().enumerate().filter(|e| e.0 != i).map(|e| text(&e.1.raw, "id")).collect();
        *slot = Some(retarget(&self.list[i].raw, t, &others));
      }
    }
    // as many tabs as entries with one: nothing was added or removed, so a tab still unpaired was edited out of
    // every feed it had, and takes the unpaired entry in its place
    if tabs.len() == self.list.iter().filter(|e| e.tab).count() {
      let mut free = (0..self.list.len()).filter(|&i| !used[i] && self.list[i].tab).collect::<Vec<_>>().into_iter();
      for (t, slot) in tabs.iter().zip(out.iter_mut()).filter(|(_, s)| s.is_none()) {
        let Some(i) = free.next() else { break };
        let others: Vec<&str> =
          self.list.iter().enumerate().filter(|e| e.0 != i).map(|e| text(&e.1.raw, "id")).collect();
        *slot = Some(retarget(&self.list[i].raw, t, &others));
      }
    }
    let mut res: Vec<Raw> = Vec::with_capacity(self.list.len().max(tabs.len()));
    for (t, slot) in tabs.iter().zip(out) {
      let raw = match slot {
        Some(r) => r,
        None => {
          let ids: Vec<&str> =
            self.list.iter().map(|e| text(&e.raw, "id")).chain(res.iter().map(|r| text(r, "id"))).collect();
          fresh(t, &ids, now_ms)
        }
      };
      res.push(raw);
    }
    // the riders, back at their places
    for (i, e) in self.list.iter().enumerate().filter(|e| !e.1.tab) {
      res.insert(i.min(res.len()), e.raw.clone());
    }
    res.truncate(MAX_CHANNELS);
    res
  }

  // the list worth keeping between runs, so what other clients store survives a change made before the
  // account answers (store `tabsync`)
  pub fn saved(&self, user: &str) -> String {
    json!({"user": user, "channels": self.values()}).to_string()
  }

  // a saved list, for this account only, checked like a frame
  pub fn load(&mut self, user: &str, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return };
    if v.get("user").and_then(Value::as_str) != Some(user) {
      return;
    }
    if let Some(f) = frame(&v) {
      self.set_list(f.list);
    }
  }
}

fn shares(a: &Spec, b: &Spec) -> bool {
  let same = |x: &str, y: &str| !x.is_empty() && x == y;
  same(&a.twitch, &b.twitch) || same(&a.kick, &b.kick) || same(&a.youtube, &b.youtube)
}

// youtube as the website and the ext's import store it: the bare handle
fn yt_wire(key: &str) -> String {
  key.trim_start_matches('@').to_owned()
}

// an entry's feeds changed to `t`. a leg this app can't read is left as it is; one it can is rewritten only
// when it now means something else (so `Forsen` and a youtube url stay as written). the id follows the ext's
// edit: the twitch name, else the kick slug, else the old id (kept when another entry has it)
fn retarget(raw: &Raw, t: &Spec, others: &[&str]) -> Raw {
  let mut r = raw.clone();
  let old = spec(raw);
  for (k, now, was, wire) in [
    ("twitch", &t.twitch, &old.twitch, t.twitch.clone()),
    ("kick", &t.kick, &old.kick, t.kick.clone()),
    ("youtube", &t.youtube, &old.youtube, yt_wire(&t.youtube)),
  ] {
    let unreadable = truthy(raw.get(k)) && was.is_empty();
    if unreadable || now == was {
      continue;
    }
    if !now.is_empty() || raw.contains_key(k) {
      r.insert(k.into(), Value::String(wire));
    }
  }
  let id = if t.twitch.is_empty() { &t.kick } else { &t.twitch };
  if !id.is_empty() && !others.contains(&id.as_str()) {
    r.insert("id".into(), Value::String(id.clone()));
  }
  r
}

// a new tab's entry, the ext's add-form shape. the id is its first feed's name (`yt-<ms>` for youtube alone),
// made unique when another entry has it
fn fresh(t: &Spec, ids: &[&str], now_ms: i64) -> Raw {
  let base = match (t.twitch.as_str(), t.kick.as_str()) {
    ("", "") => format!("yt-{now_ms}"),
    ("", k) => k.to_owned(),
    (tw, _) => tw.to_owned(),
  };
  let reserved = |s: &str| ids.contains(&s) || chan::RESERVED.contains(&s);
  let id =
    (0..).map(|n| if n == 0 { base.clone() } else { format!("{base}-{n}") }).find(|s| !reserved(s)).unwrap_or(base);
  let mut r = Raw::new();
  r.insert("id".into(), id.into());
  r.insert("twitch".into(), t.twitch.clone().into());
  r.insert("kick".into(), t.kick.clone().into());
  r.insert("youtube".into(), yt_wire(&t.youtube).into());
  r
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sp(t: &str, k: &str, y: &str) -> Spec {
    Spec::parse(t, k, y).unwrap()
  }

  fn fr(list: Value, rev: Option<f64>) -> Frame {
    let mut v = json!({"type": "multichat:config", "channels": list});
    if let Some(r) = rev {
      v["rev"] = json!(r);
    }
    frame(&v).unwrap()
  }

  // the ext's own tests for isStaleConfigRev, line for line
  #[test]
  fn an_older_rev_is_dropped_equal_or_newer_applies() {
    let mut seen = Seen::default();
    assert!(!is_stale(&mut seen, "a", Some(5.0)));
    assert!(is_stale(&mut seen, "a", Some(4.0)));
    assert!(!is_stale(&mut seen, "a", Some(5.0)));
    assert!(!is_stale(&mut seen, "a", Some(6.0)));
    assert!(is_stale(&mut seen, "a", Some(5.0)));
    // no stamp always applies and leaves the mark alone
    let mut seen = Seen { user: Some("a".into()), rev: 9.0 };
    assert!(!is_stale(&mut seen, "a", None));
    assert_eq!(seen.rev, 9.0);
    // another account starts fresh
    assert!(!is_stale(&mut seen, "b", Some(1.0)));
    assert_eq!(seen, Seen { user: Some("b".into()), rev: 1.0 });
  }

  #[test]
  fn a_stale_frame_never_lays_old_tabs_back() {
    let mut s = TabSync::default();
    let new = fr(json!([{"id": "forsen", "twitch": "forsen"}, {"id": "xqc", "kick": "xqc"}]), Some(7.0));
    assert_eq!(s.inbound("me", new), Some(vec![sp("forsen", "", ""), sp("", "xqc", "")]));
    let old = fr(json!([{"id": "forsen", "twitch": "forsen"}]), Some(6.0));
    assert_eq!(s.inbound("me", old), None);
    assert_eq!(s.tabs(), [sp("forsen", "", ""), sp("", "xqc", "")]);
    // equal applies
    let same_rev = fr(json!([{"id": "a", "twitch": "a"}]), Some(7.0));
    assert_eq!(s.inbound("me", same_rev), Some(vec![sp("a", "", "")]));
  }

  // the ext's isValidSyncedChannel cases, plus our bounds
  #[test]
  fn entries_are_checked_like_the_ext_does() {
    for ok in [
      json!({"id": "a", "twitch": "Forsen_1"}),
      json!({"id": "a", "kick": "some-one"}),
      json!({"id": "a", "youtube": "lofigirl"}),
      json!({"id": "a", "youtube": "@lofi.girl"}),
      json!({"id": "a", "youtube": "UCabcdefghijklmnopqrstuv"}),
      json!({"id": "a", "youtube": "https://www.youtube.com/@x/live"}),
      json!({"id": "a", "twitch": "", "kick": null, "youtube": 0}),
    ] {
      assert!(valid(&ok), "{ok}");
    }
    for bad in [
      json!("forsen"),
      json!(null),
      json!({"twitch": "forsen"}),
      json!({"id": 5, "twitch": "forsen"}),
      json!({"id": "a".repeat(65)}),
      json!({"id": "a", "twitch": "bad name"}),
      json!({"id": "a", "twitch": "x\r\nJOIN #y"}),
      json!({"id": "a", "twitch": "a".repeat(26)}),
      json!({"id": "a", "twitch": 5}),
      json!({"id": "a", "kick": "a.b"}),
      json!({"id": "a", "youtube": "http://youtube.com/@x"}),
      json!({"id": "a", "youtube": "https://evil.example/x"}),
      json!({"id": "a", "youtube": "https://youtube.com/@x\n"}),
      json!({"id": "a", "twitch": "a", "extra": "x".repeat(MAX_ENTRY)}),
    ] {
      assert!(!valid(&bad), "{bad}");
    }
    // a frame without a list is dropped whole; the bad entries of one with a list are dropped, the rest kept
    assert_eq!(frame(&json!({"type": "multichat:config", "channels": {"a": 1}})), None);
    assert_eq!(frame(&json!({"type": "multichat:config"})), None);
    let f = fr(json!([{"id": "a", "twitch": "a"}, {"id": "b", "twitch": "b c"}, 7]), Some(1.0));
    assert_eq!(f.list.len(), 1);
    let many: Vec<Value> = (0..80).map(|i| json!({"id": format!("c{i}"), "twitch": format!("c{i}")})).collect();
    assert_eq!(fr(json!(many), None).list.len(), MAX_CHANNELS);
  }

  #[test]
  fn what_the_ext_stores_goes_back_untouched() {
    let mut s = TabSync::default();
    let list = json!([
      {"id": "forsen", "twitch": "Forsen", "kick": "", "youtube": "https://www.youtube.com/@forsen/live",
        "sendTargets": {"twitch": true, "kick": false}, "youtubeExplicit": true, "future": [1, {"x": 2}]},
      {"id": "yt-1", "youtube": "UCabcdefghijklmnopqrstuv"},
      {"id": "linked_5", "twitch": "xqc", "kick": "xqc", "preview": true},
    ]);
    let tabs = s.inbound("me", fr(list.clone(), Some(1.0))).unwrap();
    // the UC-only entry has nothing this app can open: no tab, still in the list
    assert_eq!(tabs, [sp("forsen", "", "@forsen"), sp("xqc", "xqc", "")]);
    // the same tabs send nothing
    assert_eq!(s.outbound(&tabs, 0), None);
    // reordered: the entries move whole, the rider keeps its place
    let out = s.outbound(&[tabs[1].clone(), tabs[0].clone()], 0).unwrap();
    assert_eq!(out, [list[2].clone(), list[1].clone(), list[0].clone()]);
    // a new tab is the ext's add-form shape
    let out = s.outbound(&[tabs[1].clone(), tabs[0].clone(), sp("", "", "@lofi")], 9).unwrap();
    assert_eq!(out[3], json!({"id": "yt-9", "twitch": "", "kick": "", "youtube": "lofi"}));
    // removed: gone, everything else as it was
    let out = s.outbound(&[tabs[0].clone()], 9).unwrap();
    assert_eq!(out, [list[0].clone(), list[1].clone()]);
  }

  #[test]
  fn an_edited_tab_keeps_its_other_fields() {
    let mut s = TabSync::default();
    let list = json!([
      {"id": "forsen", "twitch": "forsen", "kick": "", "youtube": "", "sendTargets": {"kick": false}},
      {"id": "ch", "twitch": "ch", "youtube": "UCabcdefghijklmnopqrstuv"},
    ]);
    s.inbound("me", fr(list, None));
    // kick added to forsen
    let out = s.outbound(&[sp("forsen", "forsen", ""), sp("ch", "", "")], 0).unwrap();
    assert_eq!(
      out[0],
      json!({"id": "forsen", "twitch": "forsen", "kick": "forsen", "youtube": "", "sendTargets": {"kick": false}})
    );
    // twitch renamed on the entry with a feed we can't read: the id follows twitch, the UC leg stays
    let out = s.outbound(&[sp("forsen", "forsen", ""), sp("ch2", "", "")], 0).unwrap();
    assert_eq!(out[1], json!({"id": "ch2", "twitch": "ch2", "youtube": "UCabcdefghijklmnopqrstuv"}));
  }

  // the ext's rule on sign-in: the account's list replaces the tabs you had, it is not merged
  #[test]
  fn on_sign_in_the_accounts_list_wins() {
    let mut s = TabSync::default();
    // before the account has said anything, a local change builds fresh entries
    let out = s.outbound(&[sp("a", "", ""), sp("b", "", "")], 0).unwrap();
    assert_eq!(out.len(), 2);
    s = TabSync::default();
    let got = s.inbound("me", fr(json!([{"id": "c", "twitch": "c"}]), Some(3.0)));
    assert_eq!(got, Some(vec![sp("c", "", "")]));
    // duplicates and reserved names in the account's list ride along without a second tab
    let got = s.inbound(
      "me",
      fr(json!([{"id": "c", "twitch": "c"}, {"id": "c2", "twitch": "c"}, {"id": "live", "twitch": "live"}]), None),
    );
    assert_eq!(got, Some(vec![sp("c", "", "")]));
    assert_eq!(s.outbound(&[sp("c", "", "")], 0), None);
  }

  #[test]
  fn our_own_echoes_change_nothing() {
    let mut s = TabSync::default();
    s.inbound("me", fr(json!([]), Some(1.0)));
    let a = s.outbound(&[sp("a", "", "")], 0).unwrap();
    let b = s.outbound(&[sp("a", "", ""), sp("b", "", "")], 0).unwrap();
    // the echo of the first send lands after the second went out: it must not undo it
    assert_eq!(
      s.inbound("me", Frame { list: a.iter().filter_map(|v| v.as_object().cloned()).collect(), rev: Some(2.0) }),
      None
    );
    assert_eq!(
      s.inbound("me", Frame { list: b.iter().filter_map(|v| v.as_object().cloned()).collect(), rev: Some(3.0) }),
      None
    );
    assert_eq!(s.tabs(), [sp("a", "", ""), sp("b", "", "")]);
    // another client's change applies
    let got = s.inbound("me", fr(json!([{"id": "b", "twitch": "b"}]), Some(4.0)));
    assert_eq!(got, Some(vec![sp("b", "", "")]));
  }

  #[test]
  fn the_saved_list_is_for_its_account_only() {
    let mut s = TabSync::default();
    s.inbound("me", fr(json!([{"id": "a", "twitch": "a", "x": 1}]), Some(1.0)));
    let saved = s.saved("me");
    let mut t = TabSync::default();
    t.load("someone", &saved);
    assert!(t.tabs().is_empty());
    t.load("me", &saved);
    assert_eq!(t.tabs(), [sp("a", "", "")]);
    assert_eq!(t.outbound(&[sp("a", "", "")], 0), None, "the extras are known: nothing to send");
    t.load("me", "{not json");
    assert_eq!(t.tabs(), [sp("a", "", "")]);
  }
}
