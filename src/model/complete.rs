// tab-complete: the word around the cursor, the candidates for it and the one shown. which emotes are
// offered, and in what order, is the emote law's (src/law `offer`, asked in app/complete.rs); people who
// spoke here join it at the two places the law names, `@` completes people only, `/` commands
use super::catalog::Rec;
use crate::i18n::{f, t};

pub const MAX: usize = 200;
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
  // the picture an emote candidate stands for: two of one name are two candidates, and the one stepped
  // onto is the draft's pick for that name
  pub pic: Option<Rec>,
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
  // the shown pick's name and what the draft held for it before this step: put back when the cycle moves
  // on, so a step only passed leaves the name as it found it
  pub passed: Option<(Box<str>, Option<Rec>)>,
  // the catalog leg of this cycle (None for `@`, `/`, `:` and short words)
  pub remote: Option<super::catalog::Cycle>,
}

impl Complete {
  // the emote this completion settled on, if it was one (not a name or a /command): its name and its
  // picture ("" when the candidate carries none)
  pub fn emote(&self) -> Option<(&str, &str)> {
    let m = self.matches.get(self.at).filter(|m| m.emote)?;
    Some((&m.name, m.pic.as_ref().map_or("", |p| &p.url)))
  }

  // the tooltip over the box: "2/14 PogU · channel · 7tv users", plus "searching…" while a catalog page is out
  pub fn tip(&self) -> Option<String> {
    let busy = self.remote.as_ref().is_some_and(|r| r.pending);
    let Some(m) = self.matches.get(self.at) else {
      return busy.then(|| t("chat1.complete.searching_both").to_string());
    };
    let wait = if busy { t("chat1.complete.searching") } else { "" };
    Some(f(
      "chat1.complete.tip",
      &[
        ("at", &(self.at + 1).to_string()),
        ("total", &self.matches.len().to_string()),
        ("name", &m.shown()),
        ("what", m.what),
        ("wait", wait),
      ],
    ))
  }
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
    .map(|(code, glyph)| Pick {
      name: glyph.into(),
      what: t("chat1.complete.what.emoji"),
      emote: false,
      code: Some(code),
      pic: None,
    })
    .collect()
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
      out.push(Pick { name: c.into(), what: t("chat1.complete.what.command"), emote: false, code: None, pic: None });
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

  #[test]
  fn people_recent_first() {
    let r = people("al", &["alex", "albert"], &["al", "alice", "xalx"], false);
    assert_eq!(r, [("al".into(), false), ("alex".into(), true), ("albert".into(), true), ("alice".into(), false)]);
    let r = people("al", &[], &["xalx", "alpha"], true);
    assert_eq!(r.iter().map(|r| &*r.0).collect::<Vec<_>>(), ["alpha", "xalx"]);
  }

  #[test]
  fn tip_and_emote_pick() {
    let p = |name: &str, emote| Pick { name: name.into(), what: "channel · 7tv users", emote, code: None, pic: None };
    let c = Complete {
      start: 0,
      end: 0,
      matches: vec![p("Kappa", true), p("alice", false)],
      at: 0,
      chipped: None,
      passed: None,
      remote: None,
    };
    assert_eq!(c.emote().map(|e| e.0), Some("Kappa"));
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
    let c = Complete { start: 0, end: 0, matches: r, at: 0, chipped: None, passed: None, remote: None };
    assert_eq!(c.tip().as_deref().map(|t| t.ends_with("😄 :smile: · emoji")), Some(true));
    assert_eq!(c.emote(), None, "an emoji is text, never a chip");
  }

  #[test]
  fn commands_by_prefix() {
    assert_eq!(names(&commands("/vi", [vec!["/vip", "/ban"], vec!["/vip", "/view"]])), ["/vip", "/view"]);
  }
}
