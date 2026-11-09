// 7tv cosmetics of chatters: the paint on their name and the badge before it. heatsync's
// `/api/cosmetics/batch` hands them over per twitch id (the ext's route); everything is hostile
pub mod fill;
pub mod fx;
mod fx_tints;
pub mod hidden;
pub mod hsbadge;
pub mod id;
pub mod paint;
pub mod plus;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::badges::Badge;
use crate::images::url::allowed;
use id::PaintId;
use paint::Paint;

// ext: 30min for a hit, 5min for "has none" so a fresh grant shows soon; 500 chatters
const TTL_HIT: Duration = Duration::from_secs(30 * 60);
const TTL_MISS: Duration = Duration::from_secs(5 * 60);
const INFLIGHT: Duration = Duration::from_secs(30);
const MAX_USERS: usize = 500;

#[derive(Clone, Debug, Default)]
pub struct Cosmetic {
  // the paint they win with: heatsync's own when they have one, else their 7tv one
  pub paint: Option<Arc<Paint>>,
  // their 7tv paint under a heatsync one: what shows when heatsync paints are switched off
  pub stv: Option<Arc<Paint>>,
  pub badge: Option<Badge>,
  // an active heatsync plus member since (unix ms): the `+` after their name
  pub plus: Option<i64>,
  // the one heatsync badge they picked and still have, drawn after the name (and the `+`)
  pub hs_badge: Option<&'static hsbadge::Def>,
}

// badge image: 2x when 7tv has it (downscaled crisp), else 1x, else any webp
fn badge(v: &Value) -> Option<Badge> {
  let host = v.get("host")?;
  let base = host.get("url")?.as_str()?;
  let base = base.strip_prefix("//").map_or_else(|| base.to_owned(), |b| format!("https://{b}"));
  let files: Vec<&str> = host
    .get("files")?
    .as_array()?
    .iter()
    .filter_map(|f| f.get("name")?.as_str())
    .filter(|n| n.ends_with(".webp") && n.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
    .collect();
  let file = ["2x.webp", "1x.webp"].into_iter().find(|w| files.contains(w)).or_else(|| files.first().copied())?;
  let url = format!("{}/{file}", base.trim_end_matches('/'));
  if !allowed(&url) {
    return None;
  }
  Some(Badge { url: url.into(), bg: None })
}

// one entry of the batch response. `hs` is the chatter's heatsync-native paint (`/api/paints`), which
// outranks their 7tv one like in the ext; `plus` their plus_since from the same answer. None = nothing to show
pub fn parse(
  v: &Value,
  hs: Option<&Value>,
  plus: Option<&Value>,
  hs_badge: Option<&Value>,
  paints: &mut HashMap<Box<str>, Arc<Paint>>,
) -> Option<Cosmetic> {
  let mut share = |p: Paint| {
    // one allocation per distinct paint: thousands of chatters share a few hundred
    if paints.len() > 400 {
      paints.clear();
    }
    paints.entry(p.id.clone()).or_insert_with(|| Arc::new(p)).clone()
  };
  let own = hs.and_then(Paint::from_hs).map(&mut share);
  let seven = v.get("paint").and_then(Paint::parse).map(&mut share);
  let (paint, stv) = if own.is_some() { (own, seven) } else { (seven, None) };
  let badge = v.get("badge").and_then(badge);
  let plus = plus.and_then(Value::as_str).filter(|t| t.len() <= 40).and_then(crate::util::time::parse_iso);
  let hs_badge = hsbadge::parse(hs_badge);
  (paint.is_some() || badge.is_some() || plus.is_some() || hs_badge.is_some()).then_some(Cosmetic {
    paint,
    stv,
    badge,
    plus,
    hs_badge,
  })
}

struct Entry {
  c: Option<Cosmetic>,
  fetched: Option<Instant>,
  asked: Option<Instant>,
}

pub struct Cosmetics {
  map: HashMap<PaintId, Entry>,
  wanted: Vec<PaintId>,
  // the ext setting sevenTvPaints (default on) gates 7tv paints and badges alike
  pub on: bool,
  // showNamePaints (default on) gates heatsync's own paints
  pub hs_on: bool,
  // paints the user chose not to see: the name wears its plain color
  pub hidden: hidden::Hidden,
}

impl Default for Cosmetics {
  fn default() -> Self {
    Cosmetics { map: HashMap::new(), wanted: Vec::new(), on: true, hs_on: true, hidden: hidden::Hidden::default() }
  }
}

impl Cosmetics {
  // what `uid` wears. queues a fetch when unknown or stale; stale data keeps serving meanwhile
  pub fn get(&mut self, uid: &PaintId) -> Option<&Cosmetic> {
    if !self.on && !self.hs_on {
      return None;
    }
    self.entry(uid)
  }

  // the plus tenure: who someone is, not a cosmetic, so neither paint switch hides it (ext)
  pub fn plus(&mut self, uid: &PaintId) -> Option<i64> {
    self.entry(uid)?.plus
  }

  // the picked heatsync badge: who someone is, like the plus tenure, so no paint switch hides it
  pub fn hs_badge(&mut self, uid: &PaintId) -> Option<&'static hsbadge::Def> {
    self.entry(uid)?.hs_badge
  }

  fn entry(&mut self, uid: &PaintId) -> Option<&Cosmetic> {
    let now = Instant::now();
    if !self.map.contains_key(uid) {
      if self.map.len() >= MAX_USERS {
        self.evict();
      }
      self.map.insert(uid.clone(), Entry { c: None, fetched: None, asked: None });
    }
    let e = self.map.get_mut(uid)?;
    let ttl = if e.c.is_some() { TTL_HIT } else { TTL_MISS };
    if e.fetched.is_none_or(|f| now - f > ttl) && e.asked.is_none_or(|a| now - a > INFLIGHT) {
      e.asked = Some(now);
      self.wanted.push(uid.clone());
    }
    e.c.as_ref()
  }

  // the paint a name is drawn with: none while its wearer is hidden. the one read the renderer makes
  pub fn paint(&mut self, uid: &PaintId) -> Option<Arc<Paint>> {
    if self.hidden.has(uid) {
      return None;
    }
    self.shown(uid)
  }

  // what the two paint switches leave of their paint: heatsync's when on, else their 7tv one when on
  fn shown(&mut self, uid: &PaintId) -> Option<Arc<Paint>> {
    let (on, hs_on) = (self.on, self.hs_on);
    let c = self.get(uid)?;
    let p = c.paint.as_ref()?;
    match (p.native, hs_on, on) {
      (true, true, _) | (false, _, true) => Some(p.clone()),
      (true, false, true) => c.stv.clone(),
      _ => None,
    }
  }

  // the 7tv badge before the name: behind sevenTvPaints, not the heatsync paint switch
  pub fn badge(&mut self, uid: &PaintId) -> Option<&Badge> {
    if !self.on {
      return None;
    }
    self.get(uid)?.badge.as_ref()
  }

  // whether they have a paint at all, hidden or not (what the menu offers hide / show for)
  pub fn has_paint(&mut self, uid: &PaintId) -> bool {
    self.shown(uid).is_some()
  }

  // a chatter whose cosmetics are already known (the synthetic flood): never fetched
  pub fn seed(&mut self, uid: PaintId, c: Cosmetic) {
    self.map.insert(uid, Entry { c: Some(c), fetched: Some(Instant::now()), asked: None });
  }

  pub fn take_wanted(&mut self) -> Vec<PaintId> {
    std::mem::take(&mut self.wanted)
  }

  // an answer for `uid`. true when what their lines show may have changed
  pub fn put(&mut self, uid: &PaintId, c: Option<Cosmetic>) -> bool {
    let Some(e) = self.map.get_mut(uid) else { return false };
    let same = |a: &Option<Cosmetic>, b: &Option<Cosmetic>| match (a, b) {
      (None, None) => true,
      (Some(a), Some(b)) => {
        a.paint.as_ref().map(|p| &p.id) == b.paint.as_ref().map(|p| &p.id)
          && a.stv.as_ref().map(|p| &p.id) == b.stv.as_ref().map(|p| &p.id)
          && a.badge.as_ref().map(|x| &x.url) == b.badge.as_ref().map(|x| &x.url)
          && a.plus == b.plus
          && a.hs_badge.map(|d| d.id) == b.hs_badge.map(|d| d.id)
      }
      _ => false,
    };
    let changed = !same(&e.c, &c);
    e.c = c;
    e.fetched = Some(Instant::now());
    e.asked = None;
    changed
  }

  // 7tv says their cosmetics changed: ask again now (the old look serves until the answer)
  pub fn stale(&mut self, uid: &PaintId) {
    if let Some(e) = self.map.get_mut(uid) {
      e.fetched = None;
      e.asked = Some(Instant::now());
      if !self.wanted.contains(uid) {
        self.wanted.push(uid.clone());
      }
    }
  }

  fn evict(&mut self) {
    let oldest = self.map.iter().min_by_key(|(_, e)| e.fetched.or(e.asked)).map(|(k, _)| k.clone());
    if let Some(k) = oldest {
      self.map.remove(&k);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[allow(non_snake_case)]
  fn T(n: u64) -> PaintId {
    PaintId::Twitch(n)
  }

  fn paints() -> HashMap<Box<str>, Arc<Paint>> {
    HashMap::new()
  }

  #[test]
  fn plus_since_alone_is_worth_keeping_and_junk_is_not() {
    let mut cache = HashMap::new();
    let none = json!({"paint": null, "badge": null});
    let c = parse(&none, None, Some(&json!("2026-08-13T01:58:02.000Z")), None, &mut cache).unwrap();
    assert_eq!(c.plus, crate::util::time::parse_iso("2026-08-13T01:58:02.000Z"));
    for bad in [json!(null), json!(5), json!("soon"), json!("x".repeat(200))] {
      assert!(parse(&none, None, Some(&bad), None, &mut cache).is_none(), "{bad}");
    }
  }

  #[test]
  fn a_picked_heatsync_badge_is_kept_alone_and_junk_is_not() {
    let mut cache = HashMap::new();
    let none = json!({"paint": null, "badge": null});
    let c = parse(&none, None, None, Some(&json!("ember")), &mut cache).unwrap();
    assert_eq!(c.hs_badge.map(|d| d.id), Some("ember"));
    for bad in [json!("unknown_future_id"), json!(5), json!(null), json!({"id": "ember"})] {
      assert!(parse(&none, None, None, Some(&bad), &mut cache).is_none(), "{bad}");
    }
    let mut s = Cosmetics { on: false, hs_on: false, ..Cosmetics::default() };
    s.hs_badge(&T(1));
    s.put(&T(1), Some(c));
    assert_eq!(s.hs_badge(&T(1)).map(|d| d.id), Some("ember"), "no paint switch hides who someone is");
  }

  #[test]
  fn parses_paint_and_badge_and_shares_paints() {
    let v = json!({
      "paint": {"id": "P1", "function": "linear-gradient", "angle": 0, "stops": [{"at": 0, "color": -1}]},
      "badge": {"id": "B1", "tooltip": "Subscriber", "host": {"url": "//cdn.7tv.app/badge/B1", "files": [{"name": "1x.webp"}, {"name": "2x.webp"}, {"name": "3x.avif"}]}}
    });
    let mut cache = paints();
    let c = parse(&v, None, None, None, &mut cache).unwrap();
    assert_eq!(&*c.badge.as_ref().unwrap().url, "https://cdn.7tv.app/badge/B1/2x.webp");
    let again = parse(&v, None, None, None, &mut cache).unwrap();
    assert!(Arc::ptr_eq(c.paint.as_ref().unwrap(), again.paint.as_ref().unwrap()));
  }

  #[test]
  fn own_paint_outranks_the_7tv_one_and_stands_alone() {
    let hs = json!({"base": {"type": "solid", "stops": [{"pos": 0, "color": "#ffb000"}]}});
    let seven =
      json!({"paint": {"id": "P1", "function": "linear-gradient", "angle": 0, "stops": [{"at": 0, "color": -1}]}});
    let mut cache = paints();
    let c = parse(&seven, Some(&hs), None, None, &mut cache).unwrap();
    assert!(c.paint.unwrap().id.starts_with("hs"));
    // ennortix: heatsync says `paint: null`, /api/paints has the row
    let c = parse(&json!({"paint": null, "badge": null}), Some(&hs), None, None, &mut cache).unwrap();
    assert!(c.paint.is_some() && c.badge.is_none());
    assert!(parse(&seven, None, None, None, &mut cache).unwrap().paint.unwrap().id.as_ref() == "P1");
  }

  // showNamePaints and sevenTvPaints are two switches: heatsync's paint, 7tv's paint and badge
  #[test]
  fn the_two_paint_switches_gate_their_own_cosmetics() {
    let hs = json!({"base": {"type": "solid", "stops": [{"pos": 0, "color": "#ffb000"}]}});
    let seven = json!({
      "paint": {"id": "P1", "function": "linear-gradient", "angle": 0, "stops": [{"at": 0, "color": -1}]},
      "badge": {"id": "B1", "host": {"url": "//cdn.7tv.app/badge/B1", "files": [{"name": "1x.webp"}]}}
    });
    let mut cache = paints();
    let both = parse(&seven, Some(&hs), None, None, &mut cache).unwrap();
    assert!(both.paint.as_ref().unwrap().native && both.stv.is_some());
    let only_7tv = parse(&seven, None, None, None, &mut cache).unwrap();
    assert!(!only_7tv.paint.as_ref().unwrap().native && only_7tv.stv.is_none());
    let mut s = Cosmetics::default();
    let ids = [T(1), T(2)];
    for id in &ids {
      s.get(id);
    }
    s.put(&ids[0], Some(both));
    s.put(&ids[1], Some(only_7tv));
    let id_of = |s: &mut Cosmetics, i: usize| s.paint(&ids[i]).map(|p| p.id.to_string());
    assert!(id_of(&mut s, 0).unwrap().starts_with("hs"), "heatsync's wins when both are on");
    assert_eq!(id_of(&mut s, 1).as_deref(), Some("P1"));
    assert!(s.badge(&ids[1]).is_some());
    // heatsync paints off: a chatter with both wears the 7tv one; a 7tv-only chatter is unchanged
    s.hs_on = false;
    assert_eq!(id_of(&mut s, 0).as_deref(), Some("P1"));
    assert_eq!(id_of(&mut s, 1).as_deref(), Some("P1"));
    // 7tv off: heatsync's stays, 7tv's paint and badge go
    (s.hs_on, s.on) = (true, false);
    assert!(id_of(&mut s, 0).unwrap().starts_with("hs"));
    assert!(id_of(&mut s, 1).is_none() && !s.has_paint(&ids[1]));
    assert!(s.badge(&ids[1]).is_none());
    // both off: nothing, and nobody is asked about
    s.hs_on = false;
    assert!(id_of(&mut s, 0).is_none() && s.get(&T(9)).is_none());
    assert!(s.take_wanted().iter().all(|id| *id != T(9)));
  }

  #[test]
  fn hostile_cosmetics_are_dropped() {
    let mut cache = paints();
    assert!(parse(&Value::Null, None, None, None, &mut cache).is_none());
    assert!(parse(&json!({"paint": 5, "badge": []}), None, None, None, &mut cache).is_none());
    let evil = json!({"badge": {"host": {"url": "//evil.example/b", "files": [{"name": "1x.webp"}]}}});
    assert!(parse(&evil, None, None, None, &mut cache).is_none());
    let traversal =
      json!({"badge": {"host": {"url": "//cdn.7tv.app/badge/x", "files": [{"name": "../../1x.webp"}, {"name": 5}]}}});
    assert!(parse(&traversal, None, None, None, &mut cache).is_none());
  }

  #[test]
  fn store_asks_once_then_serves_and_refreshes_on_stale() {
    let mut s = Cosmetics::default();
    assert!(s.get(&T(7)).is_none());
    assert!(s.get(&T(7)).is_none());
    assert_eq!(s.take_wanted(), vec![T(7)]);
    let c = Cosmetic {
      paint: None,
      badge: Some(Badge { url: "https://cdn.7tv.app/b/1x.webp".into(), bg: None }),
      ..Default::default()
    };
    assert!(s.put(&T(7), Some(c.clone())));
    assert!(!s.put(&T(7), Some(c)));
    assert!(s.get(&T(7)).is_some());
    assert!(s.take_wanted().is_empty());
    s.stale(&T(7));
    assert_eq!(s.take_wanted(), vec![T(7)]);
    // the old badge serves while the refresh is out
    assert!(s.get(&T(7)).is_some());
    // an answer for someone never asked about is ignored
    assert!(!s.put(&T(99), None));
    (s.on, s.hs_on) = (false, false);
    assert!(s.get(&T(7)).is_none());
  }

  // the same number on another platform is another person: it neither reads nor refreshes their entry
  #[test]
  fn a_number_on_two_platforms_is_two_entries() {
    let mut s = Cosmetics::default();
    let (tw, nat) = (T(5), PaintId::Native("5".into()));
    let c = Cosmetic {
      paint: None,
      badge: Some(Badge { url: "https://cdn.7tv.app/b/1x.webp".into(), bg: None }),
      ..Default::default()
    };
    s.get(&tw);
    s.get(&nat);
    s.get(&PaintId::Kick(5));
    assert_eq!(s.take_wanted().len(), 3, "three people, three lookups");
    assert!(s.put(&tw, Some(c)));
    assert!(s.get(&tw).is_some());
    assert!(s.get(&nat).is_none() && s.get(&PaintId::Kick(5)).is_none(), "twitch 5's look never lands on them");
    assert!(!s.put(&PaintId::Youtube("UC0123456789abcdefghij_-".into()), None));
  }

  #[test]
  fn store_is_bounded() {
    let mut s = Cosmetics::default();
    for id in 0..(MAX_USERS as u64 * 2) {
      s.get(&T(id));
      s.put(&T(id), None);
    }
    assert!(s.map.len() <= MAX_USERS);
  }
}
