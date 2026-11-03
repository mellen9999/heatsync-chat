// tab-complete, in the ext's order (input.js findEmoteMatches + compareAcMatches): the word around the
// cursor matches names that start with it, then ones that hold it, any case. emotes come from the
// channel, your heatsync inventory, then globals; an exact name leads, then what you use (frecency),
// then the nearer tier. bare words put people who spoke here lately first; `@` completes people
use std::cmp::Ordering;

const MAX: usize = 200;
// the first tab needs this much of a word
pub const MIN_WORD: usize = 2;

// one candidate: what goes in the box, and the cycle tooltip's "category · who sees it"
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
  pub name: Box<str>,
  pub what: &'static str,
  pub emote: bool,
  // an emoji's shortcode (`name` is the glyph that goes in the box)
  pub code: Option<&'static str>,
}

impl Pick {
  // how a list row reads: the name, or an emoji with its shortcode
  pub fn shown(&self) -> String {
    match self.code {
      Some(c) => format!("{} :{c}:", self.name),
      None => self.name.to_string(),
    }
  }
}

pub struct Complete {
  // byte range in the line the candidate replaces
  pub start: usize,
  pub end: usize,
  pub matches: Vec<Pick>,
  pub at: usize,
  // the shown pick's chip, when the cycle added it (dropped again on the next step)
  pub chipped: Option<Box<str>>,
}

impl Complete {
  // the emote this completion settled on, if it was one (not a name or a /command)
  pub fn emote(&self) -> Option<&str> {
    self.matches.get(self.at).filter(|m| m.emote).map(|m| &*m.name)
  }

  // the tooltip over the box: "2/14 PogU · channel · 7tv users"
  pub fn tip(&self) -> Option<String> {
    let m = self.matches.get(self.at)?;
    Some(format!("{}/{} {} · {}", self.at + 1, self.matches.len(), m.shown(), m.what))
  }
}

// an emote in one of the three pools: 0 channel, 1 your inventory, 2 global
pub struct Cand<'a> {
  pub name: &'a str,
  pub tier: u8,
  pub what: &'static str,
}

struct Hit<'a> {
  c: Cand<'a>,
  exact: bool,
  prefix: bool,
  score: f64,
}

fn order(a: &Hit, b: &Hit) -> Ordering {
  let used = |h: &Hit| h.score > 0.0;
  b.exact
    .cmp(&a.exact)
    .then_with(|| if a.exact && b.exact { a.c.tier.cmp(&b.c.tier) } else { Ordering::Equal })
    .then_with(|| used(b).cmp(&used(a)))
    .then_with(|| match used(a) && used(b) {
      true => b.prefix.cmp(&a.prefix).then(b.score.total_cmp(&a.score)).then(a.c.tier.cmp(&b.c.tier)),
      false => a.c.tier.cmp(&b.c.tier).then(b.prefix.cmp(&a.prefix)),
    })
    .then_with(|| if a.prefix && b.prefix { a.c.name.len().cmp(&b.c.name.len()) } else { Ordering::Equal })
    .then_with(|| a.c.name.to_lowercase().cmp(&b.c.name.to_lowercase()))
    .then_with(|| a.c.name.cmp(b.c.name))
}

// starts-with or holds the query, any case: (exact, prefix)
fn hit(name: &str, low: &str) -> Option<(bool, bool)> {
  let n = name.to_lowercase();
  if n.starts_with(low) { Some((n.len() == low.len(), true)) } else { n.contains(low).then_some((false, false)) }
}

// emoji for `:smi`, after the emotes (the ext ranks them last: their tier is 9)
pub fn emoji(query: &str) -> Vec<Pick> {
  super::emoji::search(query, super::emoji::MAX)
    .into_iter()
    .map(|(code, glyph)| Pick { name: glyph.into(), what: "emoji", emote: false, code: Some(code) })
    .collect()
}

// emote candidates, pools in tier order; a name a nearer pool has is skipped further out
pub fn emotes<'a>(query: &str, pools: impl IntoIterator<Item = Cand<'a>>, score: impl Fn(&str) -> f64) -> Vec<Pick> {
  let low = query.to_lowercase();
  let mut seen = std::collections::HashSet::new();
  let mut hits: Vec<Hit> = pools
    .into_iter()
    .filter(|c| seen.insert(c.name))
    .filter_map(|c| {
      let (exact, prefix) = hit(c.name, &low)?;
      let score = score(c.name);
      Some(Hit { c, exact, prefix, score })
    })
    .collect();
  hits.sort_by(order);
  hits.truncate(MAX);
  hits.into_iter().map(|h| Pick { name: h.c.name.into(), what: h.c.what, emote: true, code: None }).collect()
}

// people, `recent` in recency order (newest first) then everyone else: an exact name, then who spoke
// last, then shorter, then a-z. `substr` lets a name that only holds the query in, after the rest
pub fn people(query: &str, recent: &[&str], others: &[&str], substr: bool) -> Vec<(Box<str>, bool)> {
  let low = query.to_lowercase();
  let mut seen = std::collections::HashSet::new();
  let mut hits: Vec<(&str, bool, bool, usize, bool)> = recent
    .iter()
    .map(|n| (*n, true))
    .chain(others.iter().map(|n| (*n, false)))
    .enumerate()
    .filter(|(_, (n, _))| seen.insert(n.to_lowercase()))
    .filter_map(|(i, (n, near))| {
      let (exact, prefix) = hit(n, &low)?;
      (prefix || substr).then_some((n, exact, prefix, if near { i } else { usize::MAX }, near))
    })
    .collect();
  hits.sort_by(|a, b| {
    (b.1.cmp(&a.1))
      .then(b.2.cmp(&a.2))
      .then(a.3.cmp(&b.3))
      .then(a.0.len().cmp(&b.0.len()))
      .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
  });
  hits.into_iter().take(MAX).map(|h| (h.0.into(), h.4)).collect()
}

// commands that start with what's typed, in the order given
pub fn commands(query: &str, groups: impl IntoIterator<Item = Vec<&'static str>>) -> Vec<Pick> {
  let low = query.to_lowercase();
  let mut out: Vec<Pick> = Vec::new();
  for c in groups.into_iter().flatten() {
    if c.starts_with(&low) && !out.iter().any(|o| *o.name == *c) {
      out.push(Pick { name: c.into(), what: "command", emote: false, code: None });
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn names(p: &[Pick]) -> Vec<&str> {
    p.iter().map(|p| &*p.name).collect()
  }

  fn cands<'a>(tier: u8, n: &[&'a str]) -> Vec<Cand<'a>> {
    n.iter().map(|name| Cand { name, tier, what: "x" }).collect()
  }

  #[test]
  fn exact_then_used_then_tier_then_prefix() {
    let pools =
      [cands(0, &["PogU", "pog", "Pogger", "xpogx"]), cands(1, &["PogBones", "PogU"]), cands(2, &["PogChamp"])]
        .into_iter()
        .flatten();
    let r = emotes("pog", pools, |n| if n == "PogChamp" { 3.0 } else { 0.0 });
    // the exact name first, then the one you use, then the channel's (prefix, shorter first, a hold last)
    assert_eq!(names(&r), ["pog", "PogChamp", "PogU", "Pogger", "xpogx", "PogBones"]);
    assert!(r.iter().all(|p| p.emote));
  }

  #[test]
  fn people_recent_first() {
    let r = people("al", &["alex", "albert"], &["al", "alice", "xalx"], false);
    assert_eq!(r, [("al".into(), false), ("alex".into(), true), ("albert".into(), true), ("alice".into(), false)]);
    let r = people("al", &[], &["xalx", "alpha"], true);
    assert_eq!(r.iter().map(|r| &*r.0).collect::<Vec<_>>(), ["alpha", "xalx"]);
  }

  #[test]
  fn tip_and_emote_pick() {
    let p = |name: &str, emote| Pick { name: name.into(), what: "channel · 7tv users", emote, code: None };
    let c = Complete { start: 0, end: 0, matches: vec![p("Kappa", true), p("alice", false)], at: 0, chipped: None };
    assert_eq!(c.emote(), Some("Kappa"));
    assert_eq!(c.tip().as_deref(), Some("1/2 Kappa · channel · 7tv users"));
    let c = Complete { at: 1, ..c };
    assert_eq!(c.emote(), None);
    assert_eq!(Complete { at: usize::MAX, ..c }.emote(), None);
  }

  #[test]
  fn emoji_picks_carry_the_glyph_and_their_code() {
    let r = emoji(":smil");
    assert_eq!((&*r[0].name, r[0].code, r[0].emote), ("😄", Some("smile"), false));
    assert_eq!(r[0].shown(), "😄 :smile:");
    assert!(r.len() <= crate::model::emoji::MAX && r.iter().all(|p| p.what == "emoji"));
    assert!(emoji("").is_empty() && emoji("zzzzzzzz").is_empty());
    let c = Complete { start: 0, end: 0, matches: r, at: 0, chipped: None };
    assert_eq!(c.tip().as_deref().map(|t| t.ends_with("😄 :smile: · emoji")), Some(true));
    assert_eq!(c.emote(), None, "an emoji is text, never a chip");
  }

  #[test]
  fn commands_by_prefix() {
    assert_eq!(names(&commands("/vi", [vec!["/vip", "/ban"], vec!["/vip", "/view"]])), ["/vip", "/view"]);
  }
}
