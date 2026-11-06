// the emote picker's remote leg (ext emotes.js mcRerenderSearch / mcTriggerProviderSearches): the word typed
// is also searched in the 7tv, ffz and heatsync catalogs, a page at a time, and the hits are ranked below
// your local matches. a hit is a picker tile and nothing else: it reaches chat only by being picked
// (`Registry`, then auto-add on send puts it in YOUR inventory). nothing here is visible to the render path
use std::collections::HashSet;

use super::catalog::{Hit, Page};
use crate::store;

// 7tv, ffz, hs: the chip order, and the index everywhere below
pub const NAMES: [&str; 3] = ["7tv", "ffz", "hs"];
const FILE: &str = "picker-search";
// stored hits per provider, tiles shown, pages asked, and all-duplicate pages chased per trigger
const ROW_CAP: usize = 600;
const SHOWN_CAP: usize = 800;
const MAX_PAGE: u32 = 12;
pub const CHASE: u8 = 6;

// the chips: which providers are searched, and exact-name-only. this machine only (the ext's localStorage)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs {
  pub on: [bool; 3],
  pub exact: bool,
}

impl Default for Prefs {
  fn default() -> Self {
    Prefs { on: [true; 3], exact: false }
  }
}

impl Prefs {
  pub fn load() -> Prefs {
    store::read(FILE).map_or_else(Prefs::default, |s| Prefs::parse(&s))
  }

  fn parse(s: &str) -> Prefs {
    let mut p = Prefs::default();
    for line in s.lines() {
      match line.split_once('=') {
        Some(("sources", v)) => p.on = NAMES.map(|n| v.split(',').any(|w| w.trim() == n)),
        Some(("exact", v)) => p.exact = v.trim() == "on",
        _ => {}
      }
    }
    p
  }

  pub fn save(&self) {
    let on: Vec<&str> = NAMES.iter().zip(self.on).filter(|(_, on)| *on).map(|(n, _)| *n).collect();
    let text = format!("sources={}\nexact={}\n", on.join(","), if self.exact { "on" } else { "off" });
    if let Err(e) = store::write(FILE, &text) {
      eprintln!("save picker search: {e}");
    }
  }

  pub fn any(&self) -> bool {
    self.on.contains(&true)
  }

  // chip `i` (the providers, then exact) flipped
  pub fn flip(&mut self, i: usize) {
    match i {
      0..=2 => self.on[i] = !self.on[i],
      _ => self.exact = !self.exact,
    }
  }

  pub fn is_on(&self, i: usize) -> bool {
    if i < 3 { self.on[i] } else { self.exact }
  }
}

#[derive(Debug, PartialEq)]
pub struct Ask {
  pub page: u32,
  pub which: [bool; 3],
}

#[derive(Debug, Default, PartialEq)]
pub struct Step {
  pub added: usize,
  pub chase: bool,
  // every provider asked gave nothing at all
  pub failed: bool,
}

#[derive(Default)]
pub struct Search {
  // the word the rows below answer (empty = none asked)
  pub q: Box<str>,
  // the ask whose answer is wanted
  epoch: u32,
  page: u32,
  pending: bool,
  ex: [bool; 3],
  rows: [Vec<Hit>; 3],
  chased: u8,
}

impl Search {
  // a new word (or None): everything from the last one is gone
  pub fn reset(&mut self, q: Option<&str>) {
    *self = Search { q: q.unwrap_or("").into(), ..Default::default() };
  }

  pub fn pending(&self) -> bool {
    self.pending
  }

  pub fn asked(&self) -> bool {
    self.page > 0
  }

  // nothing left to page through for the providers that are on
  pub fn done(&self, prefs: &Prefs) -> bool {
    self.page >= MAX_PAGE || (0..3).all(|i| !prefs.on[i] || self.ex[i] || self.rows[i].len() >= ROW_CAP)
  }

  // the next page from each provider that is on and has one. None while one is out or nothing is left.
  // `fresh` = a trigger (resets the chase budget), not a chase. hs has one page only
  pub fn ask(&mut self, epoch: u32, prefs: &Prefs, fresh: bool) -> Option<Ask> {
    if self.q.is_empty() || self.pending || self.done(prefs) {
      return None;
    }
    if fresh {
      self.chased = 0;
    }
    let which = [0, 1, 2].map(|i| prefs.on[i] && !self.ex[i] && (i < 2 || self.page == 0));
    if !which.contains(&true) {
      return None;
    }
    self.epoch = epoch;
    self.pending = true;
    self.page += 1;
    Some(Ask { page: self.page, which })
  }

  // an answer (None = that provider failed or was not asked: it is finished for this word). a stale one
  // is dropped
  pub fn absorb(&mut self, epoch: u32, pages: [Option<Page>; 3]) -> Option<Step> {
    if !self.pending || epoch != self.epoch {
      return None;
    }
    self.pending = false;
    let failed = pages.iter().all(Option::is_none);
    let mut added = 0;
    for (i, page) in pages.into_iter().enumerate() {
      let Some(page) = page else {
        self.ex[i] = true;
        continue;
      };
      self.ex[i] = !page.more;
      let have: HashSet<Box<str>> = self.rows[i].iter().map(|h| h.url.clone()).collect();
      let fresh: Vec<Hit> = page.hits.into_iter().filter(|h| !have.contains(&h.url)).collect();
      let room = ROW_CAP.saturating_sub(self.rows[i].len());
      added += fresh.len().min(room);
      self.rows[i].extend(fresh.into_iter().take(room));
    }
    let chase = added == 0 && !failed && self.chased < CHASE;
    if chase {
      self.chased += 1;
    }
    Some(Step { added, chase, failed: failed && self.rows.iter().all(Vec::is_empty) })
  }

  // the tiles, best first. match quality (exact, prefix, substring, the provider's fuzzy), then the
  // providers in priority order and each one's own popularity order. one tile per name, except the name
  // typed in full: every different picture of it shows. what you have under that name (`names`) or that
  // very picture (`urls`) is not offered again. `exact` keeps only the exact-name tiles
  pub fn ranked(&self, prefs: &Prefs, names: &HashSet<&str>, urls: &HashSet<&str>) -> Vec<Hit> {
    let q = &*self.q;
    let quality = |n: &str| {
      let n = n.to_ascii_lowercase();
      if n == q {
        0
      } else if n.starts_with(q) {
        1
      } else if n.contains(q) {
        2
      } else {
        3
      }
    };
    // ffz before 7tv (the ext's emoteProviderPriority default)
    let (a, b) = (1, 0);
    let mut seen: HashSet<&str> = names.iter().copied().collect();
    let mut seen_pics: HashSet<&str> = HashSet::new();
    let mut out: Vec<(u8, usize, &Hit)> = Vec::new();
    for i in [a, b, 2] {
      if !prefs.on[i] {
        continue;
      }
      for h in &self.rows[i] {
        if urls.contains(&*h.url) {
          continue;
        }
        let mq = quality(&h.name);
        let exact_name = mq == 0;
        let variant = seen.contains(&*h.name) && exact_name && !seen_pics.contains(&*h.url);
        if (seen.contains(&*h.name) && !variant) || (prefs.exact && !exact_name) {
          continue;
        }
        seen.insert(&h.name);
        if exact_name {
          seen_pics.insert(&h.url);
        }
        out.push((mq, out.len(), h));
      }
    }
    out.sort_by_key(|(mq, n, _)| (*mq, *n));
    out.into_iter().take(SHOWN_CAP).map(|(_, _, h)| h.clone()).collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::catalog::Source;

  fn hit(name: &str, id: &str, source: Source) -> Hit {
    Hit { name: name.into(), url: format!("https://cdn.7tv.app/emote/{id}/1x.webp").into(), source, zero_width: false }
  }

  fn page(hits: Vec<Hit>, more: bool) -> Page {
    Page { hits, more }
  }

  fn search(q: &str) -> Search {
    let mut s = Search::default();
    s.reset(Some(q));
    s
  }

  fn names(h: &[Hit]) -> Vec<&str> {
    h.iter().map(|h| &*h.name).collect()
  }

  fn none() -> HashSet<&'static str> {
    HashSet::new()
  }

  #[test]
  fn prefs_round_trip_and_survive_junk() {
    assert_eq!(Prefs::default(), Prefs { on: [true; 3], exact: false });
    let p = Prefs { on: [true, false, true], exact: true };
    assert_eq!(Prefs::parse("sources=7tv,hs\nexact=on\n"), p);
    assert_eq!(Prefs::parse("sources=\nexact=off").on, [false; 3]);
    assert_eq!(Prefs::parse("junk\n\u{0}\nsources=bttv,7tv"), Prefs { on: [true, false, false], exact: false });
    assert_eq!(Prefs::parse(""), Prefs::default());
    let mut f = Prefs::default();
    f.flip(1);
    f.flip(3);
    assert_eq!((f.on, f.exact, f.is_on(1), f.is_on(3)), ([true, false, true], true, false, true));
  }

  #[test]
  fn ranking_is_quality_then_provider_order_and_one_tile_per_name() {
    let mut s = search("sad");
    s.ask(1, &Prefs::default(), true).unwrap();
    let stv = page(
      vec![hit("NotSad", "a", Source::SevenTv), hit("SadA", "b", Source::SevenTv), hit("sad", "c", Source::SevenTv)],
      true,
    );
    let ffz =
      page(vec![hit("SadB", "d", Source::Ffz), hit("SadA", "e", Source::Ffz), hit("Sad", "f", Source::Ffz)], true);
    let hs = page(vec![hit("hsSad", "g", Source::Heatsync), hit("zzz", "h", Source::Heatsync)], false);
    s.absorb(1, [Some(stv), Some(ffz), Some(hs)]).unwrap();
    let r = s.ranked(&Prefs::default(), &none(), &none());
    // exact first (every different upload of it), then prefix, then substring, then fuzzy; ffz ahead of 7tv
    assert_eq!(names(&r), ["Sad", "sad", "SadB", "SadA", "NotSad", "hsSad", "zzz"]);
    assert_eq!(&*r[2].url, "https://cdn.7tv.app/emote/d/1x.webp");
    // exact: only the name typed in full
    let ex = Prefs { exact: true, ..Prefs::default() };
    assert_eq!(names(&s.ranked(&ex, &none(), &none())), ["Sad", "sad"]);
    // a provider off takes its rows away without a new search
    let no_stv = Prefs { on: [false, true, true], exact: false };
    assert!(!s.ranked(&no_stv, &none(), &none()).iter().any(|h| h.source == Source::SevenTv));
  }

  #[test]
  fn what_you_have_is_not_offered_again_but_another_picture_of_it_is() {
    let mut s = search("idk");
    s.ask(1, &Prefs::default(), true).unwrap();
    let stv = page(
      vec![
        hit("IDK", "mine", Source::SevenTv),
        hit("IDK", "other", Source::SevenTv),
        hit("IDKfa", "x", Source::SevenTv),
      ],
      false,
    );
    s.absorb(1, [Some(stv), None, None]).unwrap();
    let names_have: HashSet<&str> = ["IDK"].into();
    let urls_have: HashSet<&str> = ["https://cdn.7tv.app/emote/mine/1x.webp"].into();
    let r = s.ranked(&Prefs::default(), &names_have, &urls_have);
    assert_eq!(names(&r), ["IDK", "IDKfa"]);
    assert!(r[0].url.contains("/other/"), "the same name, a different picture");
    // not typed in full: one tile per name, and a local name hides the remote one
    let mut s = search("id");
    s.ask(1, &Prefs::default(), true).unwrap();
    let both = page(vec![hit("IDK", "o", Source::SevenTv), hit("IDK", "p", Source::SevenTv)], false);
    s.absorb(1, [Some(both), None, None]);
    assert!(s.ranked(&Prefs::default(), &names_have, &none()).is_empty());
    assert_eq!(s.ranked(&Prefs::default(), &none(), &none()).len(), 1);
  }

  #[test]
  fn paging_asks_each_provider_once_hs_only_the_first_page() {
    let mut s = search("kek");
    let p = Prefs::default();
    assert_eq!(s.ask(1, &p, true), Some(Ask { page: 1, which: [true; 3] }));
    assert!(s.ask(2, &p, true).is_none(), "one in flight");
    assert!(s.absorb(9, [None, None, None]).is_none(), "an answer to some other ask is dropped");
    let first =
      [Some(page(vec![hit("kek1", "1", Source::SevenTv)], true)), Some(page(vec![], false)), Some(page(vec![], false))];
    s.absorb(1, first).unwrap();
    assert_eq!(s.ask(2, &p, true), Some(Ask { page: 2, which: [true, false, false] }));
    // a failed provider is finished for this word; a short page ends the last one
    s.absorb(2, [Some(page(vec![hit("kek2", "2", Source::SevenTv)], false)), None, None]).unwrap();
    assert!(s.done(&p) && s.ask(3, &p, true).is_none());
    // providers that are off are never asked
    let mut s = search("kek");
    let only_ffz = Prefs { on: [false, true, false], exact: false };
    assert_eq!(s.ask(1, &only_ffz, true), Some(Ask { page: 1, which: [false, true, false] }));
    assert!(search("kek").ask(1, &Prefs { on: [false; 3], exact: false }, true).is_none());
    assert!(Search::default().ask(1, &p, true).is_none(), "no word, no ask");
  }

  #[test]
  fn duplicate_pages_chase_to_a_budget_and_rows_are_bounded() {
    let mut s = search("kek");
    let p = Prefs { on: [true, false, false], exact: false };
    let one = || page(vec![hit("kek", "1", Source::SevenTv)], true);
    s.ask(1, &p, true).unwrap();
    let st = s.absorb(1, [Some(one()), None, None]).unwrap();
    assert_eq!(st, Step { added: 1, chase: false, failed: false });
    let mut chased = 0;
    for e in 2..30 {
      if s.ask(e, &p, chased == 0).is_none() {
        break;
      }
      let st = s.absorb(e, [Some(one()), None, None]).unwrap();
      if !st.chase {
        break;
      }
      chased += 1;
    }
    assert_eq!(chased, CHASE);
    // a flood is cut at the row cap
    let mut s = search("a1");
    s.ask(1, &p, true).unwrap();
    let many: Vec<Hit> =
      (0..ROW_CAP + 50).map(|i| hit(&format!("a1x{i}"), &format!("i{i}"), Source::SevenTv)).collect();
    s.absorb(1, [Some(page(many, true)), None, None]).unwrap();
    assert_eq!(s.rows[0].len(), ROW_CAP);
    assert!(s.done(&p));
  }

  #[test]
  fn all_providers_failing_is_said() {
    let mut s = search("kek");
    s.ask(1, &Prefs::default(), true).unwrap();
    assert!(s.absorb(1, [None, None, None]).unwrap().failed);
  }
}
