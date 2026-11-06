// tab-complete's remote leg (ext input.js fetchRemoteEmoteMatches): when the local pools run out, the 7tv
// and ffz catalogs are searched a page at a time and appended to the cycle. a catalog hit is only ever a
// candidate in the box; it reaches chat the one way anything does, by being in your heatsync inventory
// (auto-add on send, `Registry`). every response is hostile: names, ids and urls are checked here
use std::collections::HashMap;

use serde_json::Value;

use super::complete::Pick;
use crate::images::url::allowed;

// a short page is the end of a provider (the ext's MC_PAGE_SIZE; 7tv's tab-complete page is 60)
pub const PAGE_STV: usize = 60;
pub const PAGE_FFZ: usize = 200;
// the heatsync route answers with at most this many native uploads
pub const PAGE_HS: usize = 100;
// start asking this many picks before the end of the cycle
pub const LOOKAHEAD: usize = 5;
// the whole cycle, locals included
pub const MAX_MATCHES: usize = 1000;
// all-duplicate pages chased per trigger before giving up on it
pub const CHASE: u8 = 4;
// tab-completed names remembered for auto-add
pub const REGISTRY_CAP: usize = 300;
// adds one sent line can start
const MAX_ADDS: usize = 25;
const MIN_NAME: usize = 2;
const MAX_NAME: usize = 64;
// longest search sent to a third party
pub const MAX_QUERY: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
  SevenTv,
  Ffz,
  // heatsync's own emote directory (the picker's `hs` chip)
  Heatsync,
}

impl Source {
  fn what(self) -> &'static str {
    match self {
      Source::SevenTv => "catalog · 7tv",
      Source::Ffz => "catalog · ffz",
      Source::Heatsync => "catalog · hs",
    }
  }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
  pub name: Box<str>,
  pub url: Box<str>,
  pub source: Source,
  pub zero_width: bool,
}

// one provider's page: what passed the checks, and whether a next page is worth asking for
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
  pub hits: Vec<Hit>,
  pub more: bool,
}

// the ext's NAME_RE, for what a third party hands us as an emote name
fn clean_name(n: &str) -> bool {
  (MIN_NAME..=MAX_NAME).contains(&n.len())
    && n.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'-' | b'(' | b')'))
}

fn clean_id(id: &str) -> bool {
  (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

const FFZ_MODS: [&str; 14] = [
  "ffzx",
  "ffzy",
  "ffzw",
  "ffzwide",
  "ffztall",
  "ffzcursed",
  "ffzhyper",
  "ffzrainbow",
  "ffzbounce",
  "ffzjam",
  "ffzslide",
  "ffzarrive",
  "ffzleave",
  "ffzspin",
];

// a word worth searching the catalogs for: 2+ chars, not `@name`, not `:emoji`, not a modifier token
// (`w!`, `h!`, `c!#fff`, `ffzWide`: the grammar is the ext's src/lib/modifiers.js)
pub fn eligible(search: &str) -> bool {
  if search.chars().count() < MIN_NAME || search.len() > MAX_QUERY || search.starts_with(['@', ':']) {
    return false;
  }
  if search.chars().any(|c| c.is_control() || c.is_whitespace()) {
    return false;
  }
  let low = search.to_ascii_lowercase();
  let bang = |s: &str| s.len() == 2 && s.ends_with('!') && b"whxvyzclrps".contains(&s.as_bytes()[0]);
  let tint = low.starts_with("c!#");
  let affixed = low.get(..2).is_some_and(bang) || low.len() >= 2 && low.get(low.len() - 2..).is_some_and(bang);
  !(FFZ_MODS.contains(&low.as_str()) || tint || affixed)
}

// what the picker may search for: 2-50 of `[a-z0-9_\-:.]` (the heatsync route's own rule, so one word fits
// all three providers), lowercased. None = nothing to ask
pub fn picker_query(typed: &str) -> Option<String> {
  let q = typed.trim().to_ascii_lowercase();
  let ok = (MIN_NAME..=MAX_QUERY).contains(&q.len())
    && q.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b':' | b'.'));
  ok.then_some(q)
}

fn items<'a>(v: &'a Value, path: &[&str], key: &str) -> &'a [Value] {
  let mut at = v;
  for p in path {
    at = at.get(p).unwrap_or(&Value::Null);
  }
  at.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

// 7tv v4 graphql `emotes.search.items[{id, defaultName, flags{defaultZeroWidth}}]`. the url is built
// here on 7tv's cdn from the id, never read from the answer
pub fn parse_7tv(v: &Value) -> Page {
  let rows = items(v, &["data", "emotes", "search"], "items");
  let hits = rows
    .iter()
    .take(PAGE_STV)
    .filter_map(|e| {
      let id = e.get("id")?.as_str().filter(|i| clean_id(i))?;
      let name = e.get("defaultName")?.as_str().filter(|n| clean_name(n))?;
      let zero_width = e.pointer("/flags/defaultZeroWidth").and_then(Value::as_bool).unwrap_or(false);
      Some(Hit {
        name: name.into(),
        url: format!("https://cdn.7tv.app/emote/{id}/1x.webp").into(),
        source: Source::SevenTv,
        zero_width,
      })
    })
    .collect();
  Page { hits, more: rows.len() >= PAGE_STV }
}

// ffz `emoticons[{name, usage_count, animated{1,2,4}, urls{1,2,4}}]`, most used first. animated art wins
// over the static first frame; urls must be on the image allowlist
pub fn parse_ffz(v: &Value) -> Page {
  let rows = v.get("emoticons").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
  let mut hits: Vec<(u64, Hit)> = rows
    .iter()
    .take(PAGE_FFZ)
    .filter_map(|e| {
      let name = e.get("name")?.as_str().filter(|n| clean_name(n))?;
      let pick = |k: &str| {
        let m = e.get(k)?.as_object()?;
        ["1", "2", "4"].iter().find_map(|r| {
          let u = m.get(*r)?.as_str()?;
          let u = u.strip_prefix("//").map_or_else(|| u.to_string(), |rest| format!("https://{rest}"));
          allowed(&u).then_some(u)
        })
      };
      let url = pick("animated").or_else(|| pick("urls"))?;
      let uses = e.get("usage_count").and_then(Value::as_u64).unwrap_or(0);
      Some((uses, Hit { name: name.into(), url: url.into(), source: Source::Ffz, zero_width: false }))
    })
    .collect();
  hits.sort_by_key(|a| std::cmp::Reverse(a.0));
  Page { hits: hits.into_iter().map(|h| h.1).collect(), more: rows.len() >= PAGE_FFZ }
}

// heatsync `GET /api/emote-search?p=hs` -> `results.hs[{name, url, id, animated, pop}]`, already ranked by the
// server and one page only (the route has no paging). names must be legal emote names, urls on the image
// allowlist; the server's own `merged` is ignored
pub fn parse_hs(v: &Value) -> Page {
  let rows = v.pointer("/results/hs").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
  let hits = rows
    .iter()
    .take(PAGE_HS)
    .filter_map(|e| {
      let name = e.get("name")?.as_str().filter(|n| n.chars().count() >= MIN_NAME && crate::heatsync::valid_name(n))?;
      let url = e.get("url")?.as_str().filter(|u| allowed(u))?;
      Some(Hit { name: name.into(), url: url.into(), source: Source::Heatsync, zero_width: false })
    })
    .collect();
  Page { hits, more: false }
}

// what the next page asks for
#[derive(Debug, PartialEq)]
pub struct Ask {
  pub page: u32,
  pub stv: bool,
  pub ffz: bool,
}

// what one answer did to the cycle
#[derive(Debug, Default, PartialEq)]
pub struct Step {
  pub added: usize,
  // the cycle had no pick and now has: show the first at once
  pub first: bool,
  // nothing new but more to page through: ask again
  pub chase: bool,
}

// the catalog leg of one tab cycle
pub struct Cycle {
  pub search: Box<str>,
  // the ask whose answer is wanted; any other answer is stale
  pub epoch: u32,
  pub pending: bool,
  page: u32,
  chased: u8,
  // [7tv, ffz] have no next page
  ex: [bool; 2],
  maxed: bool,
  // what this cycle added, by its place in the matches: the url and flags a pick needs when it is
  // taken. by place, not name: several emotes can share a name (a channel's IDK and another IDK)
  hits: HashMap<usize, Hit>,
}

impl Cycle {
  pub fn new(search: &str) -> Cycle {
    Cycle {
      search: search.into(),
      epoch: 0,
      pending: false,
      page: 0,
      chased: 0,
      ex: [false; 2],
      maxed: false,
      hits: HashMap::new(),
    }
  }

  // nothing left to page through (the cycle then wraps like any list)
  pub fn done(&self) -> bool {
    self.maxed || self.ex == [true; 2]
  }

  pub fn hit(&self, at: usize) -> Option<&Hit> {
    self.hits.get(&at)
  }

  // the next page, from each provider that has one. None while one is out or when there is no more.
  // `fresh` = a trigger (resets the chase budget), not a chase
  pub fn ask(&mut self, epoch: u32, have: usize, fresh: bool) -> Option<Ask> {
    if have >= MAX_MATCHES {
      self.maxed = true;
    }
    if self.pending || self.done() {
      return None;
    }
    if fresh {
      self.chased = 0;
    }
    self.epoch = epoch;
    self.pending = true;
    self.page += 1;
    Some(Ask { page: self.page, stv: !self.ex[0], ffz: !self.ex[1] })
  }

  // an answer (None = that provider failed or was not asked: it is finished for this search). new picks
  // only ever go on the END of `matches`: what the user already cycled never moves. the first exact
  // name leads the new block, and is registered for auto-add (the one name typed in full and sent
  // without ever tabbing onto it) unless a local pick answers to it. what `skip` vetoes never surfaces
  // (blocked, or this very emote already yours here)
  pub fn absorb(
    &mut self,
    stv: Option<Page>,
    ffz: Option<Page>,
    matches: &mut Vec<Pick>,
    reg: &mut Registry,
    skip: impl Fn(&str, &str) -> bool,
  ) -> Step {
    self.pending = false;
    for (i, p) in [&stv, &ffz].into_iter().enumerate() {
      if p.as_ref().is_none_or(|p| !p.more) {
        self.ex[i] = true;
      }
    }
    // the emote provider priority (ext emoteProviderPriority default): ffz lists before 7tv
    let incoming = ffz.into_iter().chain(stv).flat_map(|p| p.hits);
    let low = self.search.to_lowercase();
    // another emote under a name already offered is still offered (the point: a different IDK); the
    // very same picture twice is not. `skip` also vetoes what is already yours here under that name
    let mut have: std::collections::HashSet<Box<str>> = self.hits.values().map(|h| h.url.clone()).collect();
    let mut add: Vec<Hit> = Vec::new();
    for h in incoming {
      if !h.name.to_lowercase().contains(&low) || have.contains(&h.url) || skip(&h.name, &h.url) {
        continue;
      }
      have.insert(h.url.clone());
      add.push(h);
    }
    // exact name first inside the new block, the rest in the order they came
    add.sort_by_key(|h| !h.name.eq_ignore_ascii_case(&low));
    let was_empty = matches.is_empty();
    add.truncate(MAX_MATCHES.saturating_sub(matches.len()));
    // the name typed in full counts as picked, unless something here already answers to it
    let local = |n: &str| matches.iter().any(|m| m.emote && *m.name == *n);
    if let Some(h) = add.first().filter(|h| h.name.eq_ignore_ascii_case(&low) && !local(&h.name)) {
      reg.track(h);
    }
    let added = add.len();
    for h in add {
      matches.push(Pick { name: h.name.clone(), what: h.source.what(), emote: true, code: None });
      self.hits.insert(matches.len() - 1, h);
    }
    if matches.len() >= MAX_MATCHES {
      self.maxed = true;
    }
    let chase = added == 0 && !self.done() && self.chased < CHASE;
    if chase {
      self.chased += 1;
    }
    Step { added, first: was_empty && added > 0, chase }
  }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rec {
  pub url: Box<str>,
  pub zero_width: bool,
}

// names tab-completed from the catalogs, newest last, bounded. this is the ONLY thing that carries a
// catalog hit past its cycle: the box draws them, and a sent line adds them to your inventory
#[derive(Default)]
pub struct Registry {
  order: Vec<Box<str>>,
  map: HashMap<Box<str>, Rec>,
}

impl Registry {
  pub fn track(&mut self, h: &Hit) {
    self.order.retain(|n| *n != h.name);
    self.order.push(h.name.clone());
    self.map.insert(h.name.clone(), Rec { url: h.url.clone(), zero_width: h.zero_width });
    while self.order.len() > REGISTRY_CAP {
      let old = self.order.remove(0);
      self.map.remove(&old);
    }
  }

  pub fn get(&self, name: &str) -> Option<&Rec> {
    self.map.get(name)
  }

  // the cycle moved on to a pick of this name that isn't from the catalogs: that one is meant now
  pub fn forget(&mut self, name: &str) {
    if self.map.remove(name).is_some() {
      self.order.retain(|n| &**n != name);
    }
  }

  #[cfg(test)]
  fn len(&self) -> usize {
    self.map.len()
  }

  // the words of a sent line that were catalog picks and `skip` doesn't veto (owned, blocked, a
  // global). each is handed over once and forgotten: it is being added now
  pub fn take_sent(&mut self, text: &str, skip: impl Fn(&str, &Rec) -> bool) -> Vec<(Box<str>, Rec)> {
    let mut out: Vec<(Box<str>, Rec)> = Vec::new();
    for w in text.split_whitespace() {
      if out.len() >= MAX_ADDS {
        break;
      }
      let Some(rec) = self.map.get(w) else { continue };
      if out.iter().any(|o| &*o.0 == w) || skip(w, rec) {
        continue;
      }
      out.push((w.into(), rec.clone()));
    }
    for (n, _) in &out {
      self.map.remove(n);
      self.order.retain(|o| o != n);
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn hit(name: &str, source: Source) -> Hit {
    Hit {
      name: name.into(),
      url: format!("https://cdn.7tv.app/emote/{name}/1x.webp").into(),
      source,
      zero_width: false,
    }
  }

  fn page(names: &[&str], more: bool) -> Page {
    Page { hits: names.iter().map(|n| hit(n, Source::Ffz)).collect(), more }
  }

  fn names(m: &[Pick]) -> Vec<&str> {
    m.iter().map(|p| &*p.name).collect()
  }

  #[test]
  fn which_words_are_searched() {
    assert!(eligible("pog") && eligible("Pog:") && eligible("kek-w"));
    assert!(!eligible("p") && !eligible("@bob") && !eligible(":joy") && !eligible(""));
    assert!(!eligible("w!") && !eligible("kappaw!") && !eligible("h!kappa") && !eligible("c!#ff0000"));
    assert!(!eligible("ffzWide") && !eligible("FFZrainbow"));
    assert!(eligible("ffzfoo") && eligible("show") && eligible("kek!"));
    assert!(!eligible(&"a".repeat(MAX_QUERY + 1)) && !eligible("a\u{7}b") && !eligible("a b"));
  }

  #[test]
  fn seventv_page_is_parsed_and_the_url_is_ours() {
    let v = json!({"data": {"emotes": {"search": {"totalCount": 9, "items": [
      {"id": "01ABCXYZ", "defaultName": "Sadge", "flags": {"animated": true, "defaultZeroWidth": true}},
      {"id": "01ABCXYY", "defaultName": "PepeLaugh"},
      {"id": "../evil", "defaultName": "Bad"},
      {"id": "01ABCXYW", "defaultName": "has space"},
      {"id": "01ABCXYV", "defaultName": "x"},
      {"id": "01ABCXYU", "defaultName": "a/b"},
      {"id": 7, "defaultName": "Num"},
      {"defaultName": "NoId"},
      "junk", null
    ]}}}});
    let p = parse_7tv(&v);
    let got: Vec<_> = p.hits.iter().map(|h| (&*h.name, &*h.url, h.zero_width)).collect();
    assert_eq!(
      got,
      [
        ("Sadge", "https://cdn.7tv.app/emote/01ABCXYZ/1x.webp", true),
        ("PepeLaugh", "https://cdn.7tv.app/emote/01ABCXYY/1x.webp", false)
      ]
    );
    assert!(!p.more, "9 rows is a short page");
    for junk in [json!(null), json!({}), json!({"data": null}), json!({"data": {"emotes": {"search": 5}}}), json!([1])]
    {
      assert_eq!(parse_7tv(&junk), Page::default());
    }
  }

  #[test]
  fn seventv_full_page_means_more_and_is_bounded() {
    let rows: Vec<_> =
      (0..PAGE_STV + 40).map(|i| json!({"id": format!("I{i}"), "defaultName": format!("Em{i}")})).collect();
    let p = parse_7tv(&json!({"data": {"emotes": {"search": {"items": rows}}}}));
    assert_eq!(p.hits.len(), PAGE_STV);
    assert!(p.more);
  }

  #[test]
  fn ffz_page_prefers_animated_sorts_by_use_and_checks_hosts() {
    let v = json!({"emoticons": [
      {"name": "quiet", "usage_count": 5, "urls": {"1": "//cdn.frankerfacez.com/emote/1/1"}},
      {"name": "loud", "usage_count": 900, "urls": {"1": "https://cdn.frankerfacez.com/emote/2/1"},
        "animated": {"1": "https://cdn.frankerfacez.com/emote/2/animated/1"}},
      {"name": "evil", "usage_count": 99999, "urls": {"1": "https://evil.example/x.png"}},
      {"name": "sneaky", "usage_count": 99999, "urls": {"1": "https://cdn.frankerfacez.com@evil.example/x"}},
      {"name": "http", "usage_count": 99999, "urls": {"1": "http://cdn.frankerfacez.com/emote/3/1"}},
      {"name": "second", "usage_count": 1, "urls": {"1": "javascript:alert(1)", "2": "https://cdn.frankerfacez.com/emote/4/2"}},
      {"name": "bad name!", "usage_count": 1, "urls": {"1": "https://cdn.frankerfacez.com/emote/5/1"}},
      {"name": "nourl", "urls": {}},
      {"name": "negative", "usage_count": -3, "urls": {"4": "https://cdn.frankerfacez.com/emote/6/4"}},
      7, null
    ]});
    let p = parse_ffz(&v);
    let got: Vec<_> = p.hits.iter().map(|h| (&*h.name, &*h.url)).collect();
    assert_eq!(
      got,
      [
        ("loud", "https://cdn.frankerfacez.com/emote/2/animated/1"),
        ("quiet", "https://cdn.frankerfacez.com/emote/1/1"),
        ("second", "https://cdn.frankerfacez.com/emote/4/2"),
        ("negative", "https://cdn.frankerfacez.com/emote/6/4"),
      ]
    );
    assert!(!p.more);
    for junk in [json!(null), json!({}), json!({"emoticons": "x"}), json!([])] {
      assert_eq!(parse_ffz(&junk), Page::default());
    }
  }

  #[test]
  fn picker_queries_follow_the_routes_rule() {
    assert_eq!(picker_query(" KEKw ").as_deref(), Some("kekw"));
    assert_eq!(picker_query("a_b-c:d.e").as_deref(), Some("a_b-c:d.e"));
    for bad in ["", "k", "a b", "kek!", "@bob", "a\u{7}b", "é", &"a".repeat(51)] {
      assert!(picker_query(bad).is_none(), "{bad:?}");
    }
  }

  #[test]
  fn heatsync_page_is_checked_and_ignores_merged() {
    let v = json!({"results": {"hs": [
      {"name": "hsKek", "url": "https://cdn.heatsync.org/e/abc.webp", "id": "abc", "animated": true, "pop": 3},
      {"name": "evil", "url": "https://evil.example/x.png"},
      {"name": "bad name", "url": "https://cdn.heatsync.org/e/d.webp"},
      {"name": "/me", "url": "https://cdn.heatsync.org/e/d.webp"},
      {"name": "x", "url": "https://cdn.heatsync.org/e/d.webp"},
      {"name": "ctl\u{7}", "url": "https://cdn.heatsync.org/e/d.webp"},
      {"name": "http", "url": "http://cdn.heatsync.org/e/d.webp"},
      {"url": "https://cdn.heatsync.org/e/d.webp"}, 5, null
    ]}, "merged": [{"name": "merged", "url": "https://cdn.heatsync.org/e/m.webp"}]});
    let p = parse_hs(&v);
    assert_eq!(p.hits.len(), 1);
    assert_eq!((&*p.hits[0].name, p.hits[0].source, p.more), ("hsKek", Source::Heatsync, false));
    for junk in [json!(null), json!({}), json!({"results": {"hs": 4}}), json!({"results": []})] {
      assert_eq!(parse_hs(&junk), Page::default());
    }
    let many: Vec<_> =
      (0..300).map(|i| json!({"name": format!("hs{i}"), "url": "https://cdn.heatsync.org/e/a.webp"})).collect();
    assert_eq!(parse_hs(&json!({"results": {"hs": many}})).hits.len(), PAGE_HS);
  }

  fn pick(n: &str) -> Pick {
    Pick { name: n.into(), what: "x", emote: true, code: None }
  }

  #[test]
  fn merge_only_appends_exact_first_dedupes_and_filters() {
    let mut cy = Cycle::new("sad");
    let mut reg = Registry::default();
    let mut m = vec![pick("sadge"), pick("Other")];
    let a = cy.ask(1, m.len(), true).unwrap();
    assert_eq!(a, Ask { page: 1, stv: true, ffz: true });
    assert!(cy.ask(2, m.len(), true).is_none(), "one in flight");
    let blocked = |n: &str, _: &str| n == "SadBlocked";
    let s = cy.absorb(
      Some(Page {
        hits: vec![hit("NotSad", Source::SevenTv), hit("SADGE", Source::SevenTv), hit("SadSub", Source::SevenTv)],
        more: true,
      }),
      Some(page(&["SadSub", "SAD", "nothing", "SadBlocked", "sadsub"], true)),
      &mut m,
      &mut reg,
      blocked,
    );
    // what was there stays put; the exact name leads the new block, then ffz before 7tv. other emotes
    // under a similar or the same name all come (SADGE beside the local sadge, sadsub beside SadSub);
    // the same picture twice (7tv's SadSub, the very one ffz gave) does not
    assert_eq!(names(&m), ["sadge", "Other", "SAD", "SadSub", "sadsub", "NotSad", "SADGE"]);
    assert_eq!(s, Step { added: 5, first: false, chase: false });
    assert_eq!(cy.hit(2).map(|h| h.source), Some(Source::Ffz));
    assert!(cy.hit(0).is_none() && cy.hit(1).is_none(), "local picks aren't the catalog's");
    assert!(reg.get("SAD").is_some() && reg.len() == 1, "only the exact name registers up front");
    assert!(!cy.pending);
  }

  #[test]
  fn ffz_leads_the_new_block() {
    let run = || {
      let mut cy = Cycle::new("sad");
      let mut m: Vec<Pick> = Vec::new();
      cy.ask(1, 0, true).unwrap();
      let stv = Page { hits: vec![hit("SadA", Source::SevenTv), hit("SadDup", Source::SevenTv)], more: false };
      let ffz = Page { hits: vec![hit("SadB", Source::Ffz), hit("SadDup", Source::Ffz)], more: false };
      cy.absorb(Some(stv), Some(ffz), &mut m, &mut Registry::default(), |_, _| false);
      (names(&m).into_iter().map(String::from).collect::<Vec<_>>(), cy.hit(1).map(|h| h.source))
    };
    assert_eq!(run(), (["SadB", "SadDup", "SadA"].map(String::from).to_vec(), Some(Source::Ffz)));
  }

  // the channel's IDK and another IDK: both on offer, and the name typed in full stays the channel's
  // until the other one is tabbed onto
  #[test]
  fn another_emote_under_a_local_name_is_offered_but_not_assumed() {
    let mut cy = Cycle::new("IDK");
    let mut reg = Registry::default();
    let mut m = vec![pick("IDK")];
    cy.ask(1, m.len(), true).unwrap();
    let mine = |_: &str, u: &str| u.contains("/ChannelIDK/");
    let channel = Hit { url: "https://cdn.7tv.app/emote/ChannelIDK/1x.webp".into(), ..hit("IDK", Source::SevenTv) };
    let s = cy.absorb(
      Some(Page { hits: vec![channel, hit("IDK", Source::SevenTv)], more: false }),
      None,
      &mut m,
      &mut reg,
      mine,
    );
    assert_eq!(names(&m), ["IDK", "IDK"], "the channel's own picture isn't offered twice");
    assert_eq!(s.added, 1);
    assert!(cy.hit(1).is_some_and(|h| h.url.contains("/IDK/")));
    assert!(reg.get("IDK").is_none(), "typing IDK still means the channel's");
    reg.track(cy.hit(1).unwrap());
    assert!(reg.get("IDK").is_some());
    reg.forget("IDK");
    assert!(reg.get("IDK").is_none() && reg.len() == 0);
  }

  #[test]
  fn first_hit_when_there_were_no_locals_and_chasing_dupes() {
    let mut cy = Cycle::new("kek");
    let mut reg = Registry::default();
    let mut m: Vec<Pick> = Vec::new();
    cy.ask(1, 0, true).unwrap();
    let s = cy.absorb(None, Some(page(&["Kekw"], true)), &mut m, &mut reg, |_, _| false);
    assert_eq!(s, Step { added: 1, first: true, chase: false });
    assert!(cy.ex == [true, false] && !cy.done(), "a failed provider is finished, the other pages on");
    assert_eq!(cy.ask(2, 1, true), Some(Ask { page: 2, stv: false, ffz: true }));
    // an all-duplicate page chases, up to the budget
    let mut chased = 0;
    loop {
      let s = cy.absorb(None, Some(page(&["Kekw"], true)), &mut m, &mut reg, |_, _| false);
      if !s.chase {
        break;
      }
      chased += 1;
      cy.ask(10, m.len(), false).unwrap();
    }
    assert_eq!(chased, CHASE as usize);
    assert!(!cy.done());
    // a short last page ends it
    cy.ask(11, m.len(), true).unwrap();
    cy.absorb(None, Some(page(&[], false)), &mut m, &mut reg, |_, _| false);
    assert!(cy.done() && cy.ask(12, m.len(), true).is_none());
  }

  #[test]
  fn the_cycle_is_capped() {
    let mut cy = Cycle::new("em");
    let mut reg = Registry::default();
    let mut m: Vec<Pick> = (0..MAX_MATCHES - 2).map(|i| pick(&format!("loc{i}"))).collect();
    cy.ask(1, m.len(), true).unwrap();
    let s = cy.absorb(None, Some(page(&["Em1", "Em2", "Em3", "Em4"], true)), &mut m, &mut reg, |_, _| false);
    assert_eq!((m.len(), s.added), (MAX_MATCHES, 2));
    assert!(cy.done());
  }

  #[test]
  fn registry_is_bounded_and_hands_each_name_over_once() {
    let mut reg = Registry::default();
    for i in 0..REGISTRY_CAP + 20 {
      reg.track(&hit(&format!("emote{i}"), Source::SevenTv));
    }
    assert_eq!(reg.len(), REGISTRY_CAP);
    assert!(reg.get("emote0").is_none() && reg.get("emote319").is_some());
    reg.track(&hit("emote20", Source::SevenTv));
    let got = reg.take_sent("hi emote20 emote20 emote25 emote30 nope", |w, _| w == "emote25");
    assert_eq!(got.iter().map(|g| &*g.0).collect::<Vec<_>>(), ["emote20", "emote30"]);
    assert!(reg.get("emote20").is_none() && reg.get("emote25").is_some());
    assert!(reg.take_sent("emote20", |_, _| false).is_empty());
    assert!(reg.take_sent("EMOTE25 emote 25", |_, _| false).is_empty(), "exact words only");
  }
}
