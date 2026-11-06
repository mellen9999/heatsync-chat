// settings that follow your heatsync account, both ways, like the ext's (ui-state). the schema table says
// which rows travel and under which ext key (`Def::wire`); this file is the codecs and the rules:
// - everything the server says is hostile: only mapped keys are read, each value is checked against what
//   the ext's own validator accepts, anything else is ignored and counted
// - the server's word wins over what we last agreed on; a change you made that it has not seen yet is sent
// - what we send is only what you changed, and the echo of it changes nothing (`synced` already holds it)
// - map and list keys (inlineNotifs, hermesEvents, hiddenTabs) are written from the last server value, so an
//   entry this window has no row for (another client's, a newer ext's) is carried through untouched
use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::model::schema::{Ctl, DEFS, Def, Wire};
use crate::model::settings::{Counter, Settings, Side};

// the ext batches a burst of toggles the same way (main.js saveUiSetting)
const DEBOUNCE_MS: i64 = 100;
// how long a sent value may go unechoed before we ask the server what it kept
const ECHO_MS: i64 = 3000;
// bounds on one value: a real one is a few bytes (a list of tab ids, a map of flags)
const MAX_ITEMS: usize = 256;
const MAX_TEXT: usize = 64;
// the ext's free-text settings that ride the same channel, and the longest value of each we take (the
// server caps the big ones at 32KB; the ext's own maxLen for the patterns is 4096)
pub const AUTOMOD: &str = "automodRegex";
pub const RULES: &str = "chatFilterRules";
pub const REASONS: &str = "modBanReasons";
const TEXTS: [(&str, usize); 3] = [(AUTOMOD, 4096), (RULES, 32 * 1024), (REASONS, crate::model::reasons::MAX_TEXT)];
// the ext's default for hiddenTabs: what a list that was never written means
const HIDDEN_DEFAULT: [&str; 3] = ["pinned", "modlog", "discover"];

// a value in the window's own terms: a switch, or the name of a choice
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cell {
  On(bool),
  Is(&'static str),
}

// what the server sent for the keys we map, nothing else: (ext key, value)
pub type Inbound = Vec<(&'static str, Value)>;

fn get_cell(d: &Def, s: &Settings) -> Option<Cell> {
  match d.ctl {
    Ctl::Pill(get, _) => Some(Cell::On(get(s))),
    Ctl::Pick(c, get, _) => Some(Cell::Is(c.names[get(s)])),
    Ctl::Side => Some(Cell::Is(s.tab_side.name())),
    Ctl::Counter => Some(Cell::Is(s.tab_counter.name())),
    Ctl::Zoom
    | Ctl::People
    | Ctl::Alerts
    | Ctl::Words(_)
    | Ctl::Patterns
    | Ctl::Rules
    | Ctl::Reasons
    | Ctl::Buttons(..) => None,
  }
}

fn set_cell(d: &Def, s: &mut Settings, c: Cell) -> bool {
  match (d.ctl, c) {
    (Ctl::Pill(..), Cell::On(v)) => d.put(s, v),
    (Ctl::Pick(..), Cell::Is(n)) => d.pick(s, n),
    (Ctl::Side, Cell::Is(n)) => Side::parse(n).map(|v| s.tab_side = v).is_some(),
    (Ctl::Counter, Cell::Is(n)) => Counter::parse(n).map(|v| s.tab_counter = v).is_some(),
    _ => false,
  }
}

// every name a choice row can hold
fn names(d: &Def) -> Vec<&'static str> {
  match d.ctl {
    Ctl::Pick(c, ..) => c.names.to_vec(),
    Ctl::Side => Side::ALL.iter().map(|v| v.name()).collect(),
    Ctl::Counter => Counter::ALL.iter().map(|v| v.name()).collect(),
    _ => Vec::new(),
  }
}

fn mapped() -> impl Iterator<Item = &'static Def> {
  DEFS.iter().filter(|d| d.wire != Wire::None)
}

fn text_cap(k: &str) -> Option<usize> {
  TEXTS.iter().find(|(t, _)| *t == k).map(|(_, cap)| *cap)
}

// each ext key once
fn keys() -> Vec<&'static str> {
  let mut out: Vec<&'static str> = TEXTS.iter().map(|(k, _)| *k).collect();
  for k in mapped().filter_map(|d| d.wire.key()) {
    if !out.contains(&k) {
      out.push(k);
    }
  }
  out
}

// map and list keys carry other entries than ours
fn shared(k: &str) -> bool {
  mapped().any(|d| matches!(d.wire, Wire::Flag(w, _) | Wire::Hidden(w, _) if w == k))
}

fn scalar(v: &Value) -> bool {
  match v {
    Value::String(s) => s.len() <= MAX_TEXT,
    Value::Array(_) | Value::Object(_) => false,
    _ => true,
  }
}

// one value is a few bytes: a long string, a long list or anything nested is not a setting
fn small(v: &Value) -> bool {
  match v {
    Value::Array(a) => a.len() <= MAX_ITEMS && a.iter().all(scalar),
    Value::Object(o) => o.len() <= MAX_ITEMS && o.iter().all(|(k, x)| k.len() <= MAX_TEXT && scalar(x)),
    v => scalar(v),
  }
}

// the keys we map out of a `state` / `patches` object, size-bounded. the second number is how many were
// dropped for their size. the rest of the object (10k keys of anything) is never touched
pub fn pluck(state: &Value) -> (Inbound, u32) {
  let Some(o) = state.as_object() else { return (Vec::new(), 0) };
  let (mut out, mut bad) = (Vec::new(), 0);
  for k in keys() {
    match o.get(k) {
      Some(v) if text_cap(k).map_or_else(|| small(v), |cap| v.as_str().is_some_and(|t| t.len() <= cap)) => {
        out.push((k, v.clone()))
      }
      Some(_) => bad += 1,
      None => {}
    }
  }
  (out, bad)
}

// the server's value for a row, in our terms. None = not usable (wrong type, unknown name, out of range)
// or the row's entry is simply not in it
fn decode(d: &Def, v: &Value) -> Option<Cell> {
  match d.wire {
    Wire::None => None,
    Wire::Bool(_) => v.as_bool().map(Cell::On),
    Wire::Name(_) => {
      let s = v.as_str()?;
      names(d).into_iter().find(|n| *n == s).map(Cell::Is)
    }
    Wire::Anim(_) => match v.as_str()? {
      "always" => Some(Cell::On(true)),
      "never" => Some(Cell::On(false)),
      _ => None,
    },
    Wire::Level(_) => {
      // the ext's range is 0..1; the nearest of our steps, so 0.45 reads as 50
      let x = v.as_f64().filter(|x| x.is_finite() && (0.0..=1.0).contains(x))?;
      let pct = x * 100.0;
      names(d)
        .into_iter()
        .filter_map(|n| n.parse::<f64>().ok().map(|p| (n, (p - pct).abs())))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(n, _)| Cell::Is(n))
    }
    Wire::Flag(_, sub) => v.as_object()?.get(sub)?.as_bool().map(Cell::On),
    Wire::Hidden(_, id) => {
      let list = v.as_array()?;
      Some(Cell::On(!list.iter().any(|x| x.as_str() == Some(id))))
    }
  }
}

// is the value of the right shape at all (so a wrong-typed key counts as ignored, a missing entry does not)
fn shaped(d: &Def, v: &Value) -> bool {
  match d.wire {
    Wire::Flag(..) => v.is_object(),
    Wire::Hidden(..) => v.is_array(),
    _ => true,
  }
}

// our value for a row as the ext stores it. maps and lists are written onto `base` (what the server last
// held), so entries we have no row for come through untouched
fn put(d: &Def, c: Cell, base: Option<Value>) -> Option<Value> {
  Some(match (d.wire, c) {
    (Wire::Bool(_), Cell::On(v)) => json!(v),
    (Wire::Name(_), Cell::Is(n)) => json!(n),
    (Wire::Anim(_), Cell::On(v)) => json!(if v { "always" } else { "never" }),
    (Wire::Level(_), Cell::Is(n)) => json!(n.parse::<f64>().ok()? / 100.0),
    (Wire::Flag(_, sub), Cell::On(v)) => {
      let mut o = match base {
        Some(Value::Object(o)) => o,
        _ => Map::new(),
      };
      o.insert(sub.to_owned(), json!(v));
      Value::Object(o)
    }
    (Wire::Hidden(_, id), Cell::On(shown)) => {
      let mut l: Vec<Value> = match base {
        Some(Value::Array(l)) => l,
        _ => HIDDEN_DEFAULT.iter().map(|s| json!(s)).collect(),
      };
      l.retain(|x| x.as_str() != Some(id));
      if !shown {
        l.push(json!(id));
      }
      Value::Array(l)
    }
    _ => return None,
  })
}

// the server's map / list, cut down to what a write may carry forward: flags that are bools, ids that are
// short strings. None = it is not the right shape
fn keep(v: &Value) -> Option<Value> {
  match v {
    Value::Object(o) => Some(Value::Object(
      o.iter().filter(|(k, x)| k.len() <= MAX_TEXT && x.is_boolean()).map(|(k, x)| (k.clone(), x.clone())).collect(),
    )),
    Value::Array(a) => {
      Some(Value::Array(a.iter().filter(|x| x.as_str().is_some_and(|s| s.len() <= MAX_TEXT)).cloned().collect()))
    }
    _ => None,
  }
}

// one free-text row: what the account holds, a write waiting for the echo, a change not sent yet
#[derive(Default)]
struct Text {
  synced: Option<String>,
  flight: Option<(String, i64)>,
  queued: Option<String>,
}

pub struct Acct {
  // what the account holds for every mapped row, as far as we know: the base a change is measured from
  synced: Settings,
  // the last good map / list per shared key
  server: HashMap<&'static str, Value>,
  texts: HashMap<&'static str, Text>,
  // sent but not yet echoed, per row id: (value, give up at)
  flight: HashMap<&'static str, (Cell, i64)>,
  // when the first unsent change was seen
  dirty: Option<i64>,
  // values dropped as unusable since start (never shown; for the profiler line)
  pub ignored: u32,
}

impl Acct {
  pub fn new(s: &Settings) -> Self {
    Acct { synced: *s, server: HashMap::new(), texts: HashMap::new(), flight: HashMap::new(), dirty: None, ignored: 0 }
  }

  // a different login (or none): nothing pending belongs to the new account
  pub fn reset(&mut self, s: &Settings) {
    let ignored = self.ignored;
    *self = Acct::new(s);
    self.ignored = ignored;
  }

  // the account's values arrived (the fetch at login, or a push). returns the rows that moved in `s`
  pub fn inbound(&mut self, st: &Inbound, s: &mut Settings, now: i64) -> Vec<&'static Def> {
    for (k, v) in st {
      if shared(k) {
        match keep(v) {
          Some(v) => _ = self.server.insert(k, v),
          None => self.ignored += 1,
        }
      }
    }
    let mut moved = Vec::new();
    for d in mapped() {
      let Some(key) = d.wire.key() else { continue };
      let Some((_, v)) = st.iter().find(|(k, _)| *k == key) else { continue };
      if !shaped(d, v) {
        self.ignored += 1;
        continue;
      }
      // a map without our flag says nothing about it
      let Some(cell) = decode(d, v) else {
        if !matches!(d.wire, Wire::Flag(..)) || v.as_object().is_some_and(|o| o.contains_key(flag(d))) {
          self.ignored += 1;
        }
        continue;
      };
      if let Some(&(sent, until)) = self.flight.get(d.id) {
        if cell == sent {
          self.flight.remove(d.id);
          continue;
        }
        // a push that crossed our write carries the older value: wait for the echo of ours
        if now < until {
          continue;
        }
        self.flight.remove(d.id);
      }
      if get_cell(d, &self.synced) == Some(cell) {
        continue;
      }
      set_cell(d, &mut self.synced, cell);
      if get_cell(d, s) != Some(cell) && set_cell(d, s, cell) {
        moved.push(d);
      }
    }
    moved
  }

  // the account's free-text values: (key, value, first time it said anything about this key). one the
  // account already holds, or that is ours still waiting to go, is not returned
  pub fn text_inbound(&mut self, st: &Inbound, now: i64) -> Vec<(&'static str, String, bool)> {
    let mut out = Vec::new();
    for (k, v) in st {
      let (Some(cap), Some(v)) = (text_cap(k), v.as_str()) else { continue };
      if v.len() > cap {
        self.ignored += 1;
        continue;
      }
      let t = self.texts.entry(k).or_default();
      if let Some((sent, until)) = &t.flight {
        if sent == v {
          t.flight = None;
          t.synced = Some(v.to_owned());
          continue;
        }
        // a push that crossed our write carries the older value: wait for the echo of ours
        if now < *until {
          continue;
        }
        t.flight = None;
      }
      if t.synced.as_deref() == Some(v) {
        continue;
      }
      let first = t.synced.is_none();
      t.synced = Some(v.to_owned());
      if t.queued.is_none() {
        out.push((*k, v.to_owned(), first));
      }
    }
    out
  }

  // you changed a free-text setting: send it with the next patch
  pub fn text_queue(&mut self, key: &'static str, value: String, now: i64) {
    if text_cap(key).is_some_and(|cap| value.len() <= cap) {
      self.texts.entry(key).or_default().queued = Some(value);
      self.dirty.get_or_insert(now);
    }
  }

  fn text_waiting(&self) -> bool {
    self.texts.values().any(|t| t.queued.is_some())
  }

  // the patch to send now, if there is one: only rows you changed since the account last agreed
  pub fn outbound(&mut self, s: &Settings, now: i64, ready: bool) -> Option<Value> {
    // the common wake: nothing differs at all
    if *s == self.synced && !self.text_waiting() {
      self.dirty = None;
      return None;
    }
    let changed: Vec<&Def> = mapped().filter(|d| get_cell(d, s) != get_cell(d, &self.synced)).collect();
    if changed.is_empty() && !self.text_waiting() {
      // the unmapped fields differ too: forget them so the next look is a plain compare
      self.synced = *s;
      self.dirty = None;
      return None;
    }
    if !ready {
      self.dirty = None;
      return None;
    }
    if now - *self.dirty.get_or_insert(now) < DEBOUNCE_MS {
      return None;
    }
    let mut patch = Map::new();
    for key in changed.iter().filter_map(|d| d.wire.key()).collect::<std::collections::BTreeSet<_>>() {
      let mut val = self.server.get(key).cloned();
      for d in mapped().filter(|d| d.wire.key() == Some(key)) {
        let pending = changed.iter().any(|c| c.id == d.id) || self.flight.contains_key(d.id);
        if !pending {
          continue;
        }
        if let Some(c) = get_cell(d, s) {
          val = put(d, c, val.clone()).or(val);
        }
      }
      if let Some(v) = val {
        if shared(key) {
          self.server.insert(key, v.clone());
        }
        patch.insert(key.to_owned(), v);
      }
    }
    for d in &changed {
      if let Some(c) = get_cell(d, s) {
        set_cell(d, &mut self.synced, c);
        self.flight.insert(d.id, (c, now + ECHO_MS));
      }
    }
    for (k, t) in &mut self.texts {
      if let Some(v) = t.queued.take() {
        patch.insert((*k).to_owned(), Value::String(v.clone()));
        t.synced = Some(v.clone());
        t.flight = Some((v, now + ECHO_MS));
      }
    }
    self.dirty = None;
    (!patch.is_empty()).then_some(Value::Object(patch))
  }

  // true when a write has gone unechoed too long: ask the server what it kept
  pub fn stale(&mut self, now: i64) -> bool {
    let n = self.flight.len();
    self.flight.retain(|_, (_, until)| *until > now);
    let mut late = self.flight.len() != n;
    for t in self.texts.values_mut() {
      if t.flight.as_ref().is_some_and(|(_, until)| *until <= now) {
        t.flight = None;
        late = true;
      }
    }
    late
  }

  // when `outbound` / `stale` next has something to do
  pub fn due(&self) -> Option<i64> {
    let sends = self.dirty.map(|t| t + DEBOUNCE_MS);
    let echoes = self
      .flight
      .values()
      .map(|(_, until)| *until)
      .chain(self.texts.values().filter_map(|t| t.flight.as_ref().map(|(_, until)| *until)))
      .min();
    [sends, echoes].into_iter().flatten().min()
  }
}

// the entry of a Flag row in its map
fn flag(d: &Def) -> &'static str {
  match d.wire {
    Wire::Flag(_, sub) => sub,
    _ => "",
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::schema::def;

  const NOW: i64 = 1_000_000;

  // every value the window can write for every mapped key (onto a server that holds nothing). the
  // conformance script runs each through the ext's own validator
  fn emitted() -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    for d in mapped() {
      let Some(key) = d.wire.key() else { continue };
      let cells: Vec<Cell> = match d.ctl {
        Ctl::Pill(..) => vec![Cell::On(false), Cell::On(true)],
        _ => names(d).into_iter().map(Cell::Is).collect(),
      };
      out.extend(cells.into_iter().filter_map(|c| put(d, c, None)).map(|v| (key, v)));
    }
    // the free-text rows: a few patterns, and rules as the window writes them
    let mut rules = crate::model::rules::Rules::default();
    rules.add("a1".into(), "hide", "keyword", "spoiler").unwrap();
    rules.add("a2".into(), "highlight", "expr", "user:bob && !first").unwrap();
    out.push((AUTOMOD, json!("bit\\.ly\nfree\\s+v-bucks")));
    out.push((AUTOMOD, json!("")));
    out.push((RULES, json!(rules.json())));
    out.push((RULES, json!("[]")));
    out.push((REASONS, json!("spam\nhate speech")));
    out.push((REASONS, json!("")));
    out
  }

  fn fixture() -> Value {
    serde_json::from_str(include_str!("sync_fixture.json")).unwrap()
  }

  fn one(k: &'static str, v: Value) -> Inbound {
    vec![(k, v)]
  }

  // ---- the ext's catalog, as the conformance script dumped it (scratchpad/conform.mjs) ----

  // `HSC_SYNC_DUMP=/path cargo test dump_what_we_write` hands the values to the conformance script
  #[test]
  fn dump_what_we_write() {
    let Ok(path) = std::env::var("HSC_SYNC_DUMP") else { return };
    let rows: Vec<Value> = emitted().into_iter().map(|(k, v)| json!([k, v])).collect();
    std::fs::write(path, Value::Array(rows).to_string()).unwrap();
  }

  #[test]
  fn every_mapped_key_is_in_the_ext_catalog_with_the_shape_we_write() {
    let fx = fixture();
    for d in mapped() {
      let key = d.wire.key().unwrap();
      let e = &fx[key];
      assert!(e.is_object(), "{key} is not in the ext's catalog");
      let ty = e["type"].as_str().unwrap();
      let want = match d.wire {
        Wire::Bool(_) => "bool",
        Wire::Name(_) | Wire::Anim(_) => "enum",
        Wire::Level(_) => "range",
        Wire::Flag(..) => "boolmap",
        Wire::Hidden(..) => "multiselect",
        Wire::None => unreachable!(),
      };
      assert_eq!(ty, want, "{key}");
    }
  }

  #[test]
  fn what_we_write_is_what_the_ext_accepts() {
    let fx = fixture();
    let all = emitted();
    assert!(all.len() > 60);
    for (key, v) in all {
      let e = &fx[key];
      let ok = match e["type"].as_str().unwrap() {
        "bool" => v.is_boolean(),
        "enum" => e["options"].as_array().unwrap().contains(&v),
        "range" => v.as_f64().is_some_and(|x| x >= e["min"].as_f64().unwrap() && x <= e["max"].as_f64().unwrap()),
        "text" => v.as_str().is_some_and(|t| t.len() as u64 <= e["maxLen"].as_u64().unwrap_or(4096)),
        "boolmap" => {
          // our own entry must be a known one; someone else's pass through
          let o = v.as_object().unwrap();
          o.values().all(Value::is_boolean)
            && DEFS
              .iter()
              .filter_map(|d| match d.wire {
                Wire::Flag(k, sub) if k == key => Some(sub),
                _ => None,
              })
              .all(|sub| e["options"].as_array().unwrap().contains(&json!(sub)))
        }
        "multiselect" => v.as_array().unwrap().iter().all(Value::is_string),
        t => panic!("{t}"),
      };
      assert!(ok, "{key} = {v}");
    }
  }

  #[test]
  fn the_text_rows_fit_the_ext() {
    let fx = fixture();
    let (empty, none) = (crate::model::automod::Automod::default(), crate::model::rules::Rules::default());
    for (key, cap) in TEXTS {
      assert_eq!(fx[key]["type"], "text", "{key}");
      // a text setting with no maxLen of its own takes the ext's default, 4096
      assert!(cap as u64 <= fx[key]["maxLen"].as_u64().unwrap_or(4096), "{key}");
    }
    assert_eq!(json!(empty.text()), fx[AUTOMOD]["default"]);
    assert_eq!(json!(none.json()), fx[RULES]["default"]);
    assert_eq!(json!(crate::model::reasons::Reasons::default().text()), fx[REASONS]["default"]);
  }

  #[test]
  fn text_rows_follow_the_account_and_go_up_once() {
    let s = Settings::default();
    let mut a = Acct::new(&s);
    // the account's word, the first time: applied
    let got = a.text_inbound(&one(AUTOMOD, json!("a\nb")), NOW);
    assert_eq!(got, vec![(AUTOMOD, "a\nb".to_owned(), true)]);
    assert!(a.text_inbound(&one(AUTOMOD, json!("a\nb")), NOW + 1).is_empty(), "nothing new");
    assert_eq!(a.text_inbound(&one(AUTOMOD, json!("c")), NOW + 2), vec![(AUTOMOD, "c".to_owned(), false)]);
    // wrong type, too long, not ours: ignored
    assert!(a.text_inbound(&one(RULES, json!(5)), NOW).is_empty());
    assert!(a.text_inbound(&one(AUTOMOD, json!("x".repeat(5000))), NOW).is_empty());
    assert_eq!(a.ignored, 1);
    // our edit goes once, after the debounce, and its echo changes nothing
    a.text_queue(AUTOMOD, "mine".into(), NOW + 10);
    assert!(a.outbound(&s, NOW + 10, true).is_none());
    let patch = a.outbound(&s, NOW + 10 + DEBOUNCE_MS, true).unwrap();
    assert_eq!(patch[AUTOMOD], json!("mine"));
    assert!(a.outbound(&s, NOW + 500, true).is_none());
    // a push that crossed our write is waited out, the echo clears it
    assert!(a.text_inbound(&one(AUTOMOD, json!("c")), NOW + 300).is_empty());
    assert!(a.text_inbound(&one(AUTOMOD, json!("mine")), NOW + 400).is_empty());
    assert!(!a.stale(NOW + 9999));
    // not signed in yet: it waits, and goes when ready
    a.text_queue(RULES, "[]".into(), NOW + 1000);
    assert!(a.outbound(&s, NOW + 2000, false).is_none());
    assert!(a.outbound(&s, NOW + 2200, true).is_none());
    assert!(a.outbound(&s, NOW + 2200 + DEBOUNCE_MS, true).is_some());
    // never echoed: ask again
    assert!(a.stale(NOW + 2200 + DEBOUNCE_MS + ECHO_MS + 1));
  }

  #[test]
  fn text_rows_are_plucked_with_their_own_bounds() {
    let st = json!({ "automodRegex": "x", "chatFilterRules": "[]", "other": "y".repeat(100) });
    let (got, bad) = pluck(&st);
    assert_eq!((got.len(), bad), (2, 0));
    let st = json!({ "automodRegex": "x".repeat(5000), "chatFilterRules": ["a"] });
    assert_eq!(pluck(&st), (Vec::new(), 2));
    let st = json!({ "chatFilterRules": "r".repeat(30_000) });
    assert_eq!(pluck(&st).0.len(), 1);
  }

  #[test]
  fn every_choice_the_ext_offers_is_ours_or_a_named_gap() {
    let fx = fixture();
    // ext values this window has no equivalent for, kept on the server as they are
    let gaps = [("animateEmotes", "hover")];
    for d in mapped().filter(|d| matches!(d.wire, Wire::Name(_))) {
      let key = d.wire.key().unwrap();
      for o in fx[key]["options"].as_array().unwrap() {
        let known = names(d).iter().any(|n| json!(n) == *o);
        assert!(known || gaps.iter().any(|(k, v)| *k == key && json!(v) == *o), "{key} {o}");
      }
    }
  }

  #[test]
  fn defaults_match_the_ext() {
    let fx = fixture();
    let s = Settings::default();
    for d in mapped() {
      let key = d.wire.key().unwrap();
      let want = &fx[key]["default"];
      let ours = get_cell(d, &s).unwrap();
      let theirs = match d.wire {
        Wire::Bool(_) => want.as_bool().map(Cell::On),
        Wire::Name(_) => decode(d, want),
        Wire::Anim(_) => decode(d, want),
        Wire::Level(_) => decode(d, want),
        Wire::Flag(_, sub) => want.get(sub).and_then(Value::as_bool).map(Cell::On),
        Wire::Hidden(_, id) => want.as_array().map(|l| Cell::On(!l.iter().any(|x| x.as_str() == Some(id)))),
        Wire::None => None,
      };
      // a tab the ext hides by default (pinned, modlog, discover) is not one of ours
      assert_eq!(Some(ours), theirs, "{} ({key})", d.id);
    }
  }

  // ---- codecs ----

  #[test]
  fn codecs_round_trip_every_value_of_every_row() {
    for d in mapped() {
      let key = d.wire.key().unwrap();
      let cells: Vec<Cell> = match d.ctl {
        Ctl::Pill(..) => vec![Cell::On(false), Cell::On(true)],
        _ => names(d).into_iter().map(Cell::Is).collect(),
      };
      for c in cells {
        let v = put(d, c, None).unwrap();
        assert_eq!(decode(d, &v), Some(c), "{key}");
        let mut s = Settings::default();
        assert!(set_cell(d, &mut s, c));
        assert_eq!(get_cell(d, &s), Some(c));
      }
    }
  }

  #[test]
  fn volume_is_the_exts_zero_to_one() {
    let d = def("mention_volume").unwrap();
    assert_eq!(put(d, Cell::Is("30"), None), Some(json!(0.3)));
    assert_eq!(put(d, Cell::Is("100"), None), Some(json!(1.0)));
    assert_eq!(decode(d, &json!(0.3)), Some(Cell::Is("30")));
    assert_eq!(decode(d, &json!(0.45)), Some(Cell::Is("50")), "the nearest step");
    assert_eq!(decode(d, &json!(0)), Some(Cell::Is("0")));
    for bad in [json!(1.5), json!(-0.1), json!("0.3"), json!(null), json!(true), json!([0.3])] {
      assert_eq!(decode(d, &bad), None, "{bad}");
    }
  }

  #[test]
  fn paint_motion_is_always_or_never() {
    let d = def("paint_motion").unwrap();
    assert_eq!(decode(d, &json!("never")), Some(Cell::On(false)));
    assert_eq!(decode(d, &json!("always")), Some(Cell::On(true)));
    assert_eq!(decode(d, &json!("hover")), None);
    assert_eq!(decode(d, &json!(false)), None);
  }

  #[test]
  fn hostile_values_are_ignored_not_applied() {
    let huge = "x".repeat(100_000);
    let nested = json!({"a": {"b": {"c": [1, 2, 3]}}});
    for d in mapped() {
      for bad in [json!(null), json!(7), json!(huge), json!("nope"), nested.clone(), json!([[1]]), json!({"zzz": 1})] {
        let mut s = Settings::default();
        let before = s;
        let mut a = Acct::new(&s);
        let (inb, _) = pluck(&json!({ d.wire.key().unwrap(): bad.clone() }));
        let moved = a.inbound(&inb, &mut s, NOW);
        assert!(moved.is_empty() && s == before, "{} took {bad:.30}", d.id);
      }
    }
  }

  #[test]
  fn ten_thousand_keys_cost_nothing_and_unknown_ones_are_never_read() {
    let mut o = Map::new();
    for i in 0..10_000 {
      o.insert(format!("junk{i}"), json!({"deep": [i]}));
    }
    o.insert("zebra".into(), json!(false));
    o.insert("platformFilters".into(), json!({"x": 1}));
    o.insert("hs_emote_size".into(), json!("4x"));
    o.insert("viewer_show_nsfw".into(), json!(true));
    let (inb, bad) = pluck(&Value::Object(o));
    assert_eq!((inb.len(), bad), (1, 0), "only zebra is ours");
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    assert_eq!(a.inbound(&inb, &mut s, NOW).len(), 1);
    assert!(!s.zebra && s.emote_size == Settings::default().emote_size && !s.show_nsfw);
  }

  #[test]
  fn an_oversized_value_for_a_mapped_key_is_dropped_and_counted() {
    let (inb, bad) = pluck(&json!({"hiddenTabs": vec!["a"; 300], "zebra": "x".repeat(500), "viMode": true}));
    assert_eq!((inb.len(), bad), (1, 2));
    let (none, _) = pluck(&json!("not an object"));
    assert!(none.is_empty());
  }

  // ---- shared maps and lists ----

  #[test]
  fn a_write_carries_entries_we_have_no_row_for() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    // the account's map holds the ext's dm flag, a flag from a newer ext, and junk
    let held = json!({"op": true, "dm": true, "futureFlag": false, "junk": 5, "nested": {"a": 1}});
    a.inbound(&one("inlineNotifs", held), &mut s, NOW);
    s.inline_re = false;
    let patch = a.outbound(&s, NOW, true);
    assert!(patch.is_none(), "debounced");
    let p = a.outbound(&s, NOW + DEBOUNCE_MS, true).unwrap();
    assert_eq!(p["inlineNotifs"], json!({"op": true, "dm": true, "futureFlag": false, "re": false}));
    assert_eq!(p.as_object().unwrap().len(), 1, "only the key that changed");
  }

  #[test]
  fn hidden_tabs_keep_the_ids_we_do_not_have() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    let held = json!(["pinned", "modlog", "discover", "whispers", 5, {"x": 1}]);
    let moved = a.inbound(&one("hiddenTabs", held), &mut s, NOW);
    assert_eq!(moved.len(), 1, "whispers is hidden");
    assert!(!s.tab_whispers && s.tab_feed && s.tab_mentions);
    s.tab_feed = false;
    s.tab_whispers = true;
    a.outbound(&s, NOW, true);
    let p = a.outbound(&s, NOW + DEBOUNCE_MS, true).unwrap();
    let mut got: Vec<&str> = p["hiddenTabs"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    got.sort_unstable();
    assert_eq!(got, ["discover", "feed", "modlog", "pinned"]);
  }

  #[test]
  fn a_list_the_server_never_held_starts_from_the_exts_default() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    s.tab_mentions = false;
    a.outbound(&s, NOW, true);
    let p = a.outbound(&s, NOW + DEBOUNCE_MS, true).unwrap();
    assert_eq!(p["hiddenTabs"], json!(["pinned", "modlog", "discover", "mentions"]));
  }

  #[test]
  fn a_map_the_server_never_held_is_written_with_just_our_entry() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    s.ev_offline = true;
    a.outbound(&s, NOW, true);
    let p = a.outbound(&s, NOW + DEBOUNCE_MS, true).unwrap();
    assert_eq!(p["hermesEvents"], json!({"offline": true}));
  }

  #[test]
  fn two_quick_flips_in_one_map_do_not_undo_each_other_when_a_stale_push_lands_between() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    a.inbound(&one("inlineNotifs", json!({"op": true, "mop": true})), &mut s, NOW);
    s.inline_op = false;
    a.outbound(&s, NOW, true);
    a.outbound(&s, NOW + DEBOUNCE_MS, true).unwrap();
    // the echo of an earlier write still says op is on
    a.inbound(&one("inlineNotifs", json!({"op": true, "mop": true})), &mut s, NOW + 500);
    assert!(!s.inline_op, "the old push does not undo your flip");
    s.inline_mop = false;
    a.outbound(&s, NOW + 600, true);
    let p = a.outbound(&s, NOW + 600 + DEBOUNCE_MS, true).unwrap();
    assert_eq!(p["inlineNotifs"], json!({"op": false, "mop": false}));
  }

  // ---- when things travel ----

  #[test]
  fn the_server_wins_for_keys_it_has_and_keys_it_lacks_stay_and_are_not_pushed() {
    let mut s = Settings { zebra: true, vi: true, ..Settings::default() };
    let mut a = Acct::new(&s);
    let moved = a.inbound(&one("zebra", json!(false)), &mut s, NOW);
    assert_eq!(moved.iter().map(|d| d.id).collect::<Vec<_>>(), ["zebra"]);
    assert!(!s.zebra && s.vi, "vi is not on the server: stays");
    assert!(a.outbound(&s, NOW, true).is_none() && a.outbound(&s, NOW + 5000, true).is_none(), "nothing to push");
  }

  #[test]
  fn applying_what_the_server_said_sends_nothing_back() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    let st = vec![
      ("zebra", json!(false)),
      ("viMode", json!(true)),
      ("tabPosition", json!("left")),
      ("mentionSoundVolume", json!(0.7)),
    ];
    assert_eq!(a.inbound(&st, &mut s, NOW).len(), 4);
    assert!(a.outbound(&s, NOW, true).is_none());
    assert!(a.outbound(&s, NOW + 10_000, true).is_none());
    // the same state again moves nothing
    assert!(a.inbound(&st, &mut s, NOW + 1).is_empty());
  }

  #[test]
  fn your_change_goes_once_debounced_and_its_echo_changes_nothing() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    s.zebra = false;
    s.timestamps = true;
    assert!(a.outbound(&s, NOW, true).is_none(), "wait out the burst");
    assert!(a.outbound(&s, NOW + 50, true).is_none());
    let p = a.outbound(&s, NOW + DEBOUNCE_MS, true).unwrap();
    assert_eq!(p, json!({"zebra": false, "timestamps": true}));
    assert!(a.outbound(&s, NOW + 1000, true).is_none(), "sent once");
    // the server's merged state comes back to us
    let echo = vec![("zebra", json!(false)), ("timestamps", json!(true))];
    assert!(a.inbound(&echo, &mut s, NOW + 200).is_empty());
    assert!(a.outbound(&s, NOW + 5000, true).is_none());
    assert!(!a.stale(NOW + 5000), "echoed: nothing to ask again");
  }

  #[test]
  fn nothing_is_sent_while_signed_out_or_the_socket_is_not_in() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    s.vi = true;
    assert!(a.outbound(&s, NOW, false).is_none() && a.outbound(&s, NOW + 9999, false).is_none());
    // in, the change goes (after its own wait)
    assert!(a.outbound(&s, NOW + 10_000, true).is_none());
    assert!(a.outbound(&s, NOW + 10_000 + DEBOUNCE_MS, true).is_some());
  }

  #[test]
  fn the_account_repeating_what_we_agreed_on_is_not_news_but_a_different_value_beats_a_pending_change() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    a.inbound(&one("hideBots", json!(true)), &mut s, NOW);
    s.hide_bots = false;
    a.outbound(&s, NOW + 1, true);
    a.inbound(&one("hideBots", json!(true)), &mut s, NOW + 2);
    assert!(!s.hide_bots, "equal to the agreed value is not news: yours stands");
    assert_eq!(a.outbound(&s, NOW + 1 + DEBOUNCE_MS, true), Some(json!({"hideBots": false})));
    // a change not yet sent, then the account says something else it did not say before
    let mut s = Settings::default();
    let mut b = Acct::new(&s);
    s.zebra = false;
    b.outbound(&s, NOW, true);
    assert_eq!(b.inbound(&one("zebra", json!("garbage")), &mut s, NOW + 5).len(), 0);
    s.timestamps = true;
    b.outbound(&s, NOW + 6, true);
    assert_eq!(b.inbound(&one("timestamps", json!(false)), &mut s, NOW + 7).len(), 0, "false is what we agreed on");
    let mut s = Settings::default();
    let mut c = Acct::new(&s);
    s.zebra = false;
    c.outbound(&s, NOW, true);
    // zebra: agreed true, pending false; the account now says false -> same as ours, nothing to do
    assert!(c.inbound(&one("zebra", json!(false)), &mut s, NOW + 1).is_empty());
    let mut s = Settings::default();
    let mut d = Acct::new(&s);
    s.hide_dupes = true;
    d.outbound(&s, NOW, true);
    // agreed false, pending true; the ext set it to true and back is impossible to tell apart: only a value
    // different from the agreed one counts, and it equals ours
    assert!(d.inbound(&one("hideDuplicates", json!(true)), &mut s, NOW + 1).is_empty());
    assert!(s.hide_dupes);
  }

  #[test]
  fn a_push_that_crossed_our_write_is_waited_out_then_resolved() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    s.timestamps = true;
    a.outbound(&s, NOW, true);
    a.outbound(&s, NOW + DEBOUNCE_MS, true).unwrap();
    // an older state arrives first: does not flip it back
    assert!(a.inbound(&one("timestamps", json!(false)), &mut s, NOW + 150).is_empty());
    assert!(s.timestamps);
    // never echoed: after the wait we ask, and the answer decides
    assert!(!a.stale(NOW + DEBOUNCE_MS + 100));
    assert!(a.stale(NOW + DEBOUNCE_MS + ECHO_MS + 1));
    assert_eq!(a.inbound(&one("timestamps", json!(false)), &mut s, NOW + 9000).len(), 1);
    assert!(!s.timestamps, "the server did not keep it: follow the server");
  }

  #[test]
  fn a_new_login_drops_what_was_pending_for_the_old_one() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    s.zebra = false;
    a.reset(&s);
    assert!(a.outbound(&s, NOW, true).is_none() && a.outbound(&s, NOW + 9999, true).is_none());
  }

  #[test]
  fn tab_side_and_counter_follow_by_name() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    let moved = a.inbound(&vec![("tabPosition", json!("bottom")), ("tabCounter", json!("heat"))], &mut s, NOW);
    assert_eq!(moved.len(), 2);
    assert_eq!((s.tab_side, s.tab_counter), (Side::Bottom, Counter::Heat));
    let none = a.inbound(&vec![("tabPosition", json!("diagonal")), ("tabCounter", json!(1))], &mut s, NOW);
    assert!(none.is_empty() && s.tab_side == Side::Bottom);
  }

  #[test]
  fn animate_emotes_hover_is_left_alone() {
    let mut s = Settings::default();
    let mut a = Acct::new(&s);
    assert!(a.inbound(&one("animateEmotes", json!("hover")), &mut s, NOW).is_empty());
    assert!(a.inbound(&one("animateEmotes", json!("never")), &mut s, NOW).len() == 1);
  }

  #[test]
  fn device_local_keys_have_no_row() {
    let ids: Vec<&str> = keys();
    for local in [
      "hs_readable_names",
      "hs_emote_size",
      "hs_dim_timeouts",
      "hs_notifications",
      "hs_mute_keywords",
      "platformFilters",
      "keywordHighlights",
    ] {
      assert!(!ids.contains(&local), "{local}");
    }
    assert!(!ids.iter().any(|k| k.starts_with("viewer_show_")));
  }
}
