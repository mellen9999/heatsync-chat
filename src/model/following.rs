// who you follow that is live right now (the ext's live-followed snapshot, background.js): rows from
// `/api/live/following` and the ws `follow:live:snapshot`, kept current by `follow:stream:online|offline|update`.
// pure: no ui, no net. every row came from a server, so counts, names and numbers are bounded here
use serde_json::Value;

use super::ident;
use super::message::Platform;
use super::stream_events::{Kind, clean};
use crate::util::time::parse_iso;

// the most rows kept (a heavily followed account is a few hundred live at the very most)
pub const MAX_ROWS: usize = 500;
const MAX_VIEWERS: u64 = 100_000_000;
const ALL: [Platform; 3] = [Platform::Twitch, Platform::Kick, Platform::Youtube];

// one live person. `legs` are the channels they are live on, the one the list is named for first: a
// simulcaster is one row, and opening it joins every leg in one merged tab
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
  pub name: Box<str>,
  pub game: Box<str>,
  pub title: Box<str>,
  pub viewers: u64,
  // when the stream began (ms), when the server says (twitch does; kick and youtube rows carry none)
  pub since: Option<i64>,
  // (platform, channel key): twitch login and kick slug in lowercase, youtube `@handle`
  pub legs: Vec<(Platform, Box<str>)>,
}

impl Live {
  pub fn primary(&self) -> &(Platform, Box<str>) {
    &self.legs[0]
  }

  fn has(&self, p: Platform, key: &str) -> bool {
    self.legs.iter().any(|l| l.0 == p && *l.1 == *key)
  }

  // the words a filter matches against
  pub fn hay(&self) -> String {
    format!("{} {} {} {}", self.name, self.game, self.title, self.legs[0].1).to_lowercase()
  }
}

fn platform(s: &str) -> Option<Platform> {
  ALL.into_iter().find(|p| p.name() == s.trim().to_ascii_lowercase())
}

// a channel key a tab and a stream link can take, from the server's word for it. youtube is a handle
// (a bare UC id can't be a tab, so a row with only that is dropped)
fn leg_key(p: Platform, raw: &str) -> Option<Box<str>> {
  let k = raw.trim().to_ascii_lowercase();
  match p {
    Platform::Twitch => ident::twitch_room(&k).then(|| k.into()),
    Platform::Kick => ident::kick_slug(&k).then(|| k.into()),
    Platform::Youtube => {
      let h = k.trim_start_matches('@');
      (ident::yt_handle(h) && !(h.len() >= 22 && h.starts_with("uc"))).then(|| format!("@{h}").into())
    }
  }
}

fn text(v: &Value, keys: &[&str]) -> Box<str> {
  keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str).map(clean).filter(|s| !s.is_empty())).unwrap_or_default()
}

fn count(v: &Value) -> u64 {
  ["viewerCount", "viewer_count"]
    .iter()
    .find_map(|k| v.get(*k))
    .map(|n| n.as_u64().or_else(|| n.as_f64().filter(|f| f.is_finite() && *f > 0.0).map(|f| f as u64)).unwrap_or(0))
    .unwrap_or(0)
    .min(MAX_VIEWERS)
}

// one row of `streams`: None for anything that isn't a channel we could open
fn row(v: &Value) -> Option<Live> {
  let p = platform(v.get("platform")?.as_str()?)?;
  let key = leg_key(p, v.get("username")?.as_str()?)?;
  let mut legs = vec![(p, key)];
  // a merged card names its other platforms' handles (they can differ from the first's)
  if let Some(m) = v.get("platformUsernames").and_then(Value::as_object) {
    for q in ALL.into_iter().filter(|q| *q != p) {
      if let Some(k) = m.get(q.name()).and_then(Value::as_str).and_then(|s| leg_key(q, s)) {
        legs.push((q, k));
      }
    }
  }
  let mut name = text(v, &["heatsyncDisplayName", "displayName", "display_name"]);
  if name.is_empty() {
    name = legs[0].1.trim_start_matches('@').into();
  }
  Some(Live {
    name,
    game: text(v, &["category", "game", "gameName", "game_name"]),
    title: text(v, &["title"]),
    viewers: count(v),
    since: v.get("startedAt").and_then(Value::as_str).and_then(parse_iso).filter(|t| *t > 0),
    legs,
  })
}

// the rows of a `streams` array, at most MAX_ROWS, one per channel, biggest first
pub fn parse(streams: Option<&Value>) -> Vec<Live> {
  let mut f = Follows::default();
  for r in streams.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default().iter().take(MAX_ROWS * 2) {
    if let Some(l) = row(r) {
      f.put(l);
    }
  }
  f.sort();
  f.rows
}

// a `follow:stream:*` frame, once checked
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
  pub kind: Kind,
  pub platform: Platform,
  pub key: Box<str>,
  pub name: Box<str>,
  pub game: Box<str>,
  pub title: Box<str>,
}

impl Change {
  pub fn from_frame(kind: Kind, v: &Value) -> Option<Change> {
    let platform = platform(v.get("platform")?.as_str()?)?;
    let key = leg_key(platform, v.get("channel")?.as_str()?)?;
    let name = text(v, &["displayName"]);
    // the name as they write it, only when it is their channel key in other letters
    let name = if name.eq_ignore_ascii_case(key.trim_start_matches('@')) { name } else { "".into() };
    Some(Change { kind, platform, key, name, game: text(v, &["game"]), title: text(v, &["title"]) })
  }
}

#[derive(Default)]
pub struct Follows {
  rows: Vec<Live>,
}

impl Follows {
  pub fn rows(&self) -> &[Live] {
    &self.rows
  }

  // the row with this channel among its legs: whose channel it is
  pub fn person(&self, p: Platform, key: &str) -> Option<&Live> {
    self.rows.iter().find(|r| r.has(p, key))
  }

  // biggest audience first, then by name: the ext's order (its snapshot sorts on viewers)
  fn sort(&mut self) {
    self.rows.sort_by(|a, b| b.viewers.cmp(&a.viewers).then_with(|| a.legs[0].1.cmp(&b.legs[0].1)));
  }

  fn at(&self, l: &Live) -> Option<usize> {
    self.rows.iter().position(|r| l.legs.iter().any(|g| r.has(g.0, &g.1)))
  }

  // add a row, or refresh the one that already names a channel of it. no sorting: callers do it once
  fn put(&mut self, l: Live) {
    match self.at(&l) {
      Some(i) => self.rows[i] = l,
      None if self.rows.len() < MAX_ROWS => self.rows.push(l),
      None => {}
    }
  }

  // the answer to a full ask: exactly who is live now
  pub fn replace(&mut self, rows: Vec<Live>) {
    self.rows = Vec::new();
    for l in rows {
      self.put(l);
    }
    self.sort();
  }

  // the socket's catch-up on connect: adds and refreshes, never takes anyone off (a cold cache sends
  // nothing, a partial one must not wipe what we knew)
  pub fn union(&mut self, rows: Vec<Live>) {
    for l in rows {
      self.put(l);
    }
    self.sort();
  }

  // a go-live, a game switch or a sign-off from the socket. true when the list changed
  pub fn change(&mut self, c: Change) -> bool {
    let at = self.rows.iter().position(|r| r.has(c.platform, &c.key));
    let before = self.rows.clone();
    match (c.kind, at) {
      (Kind::Offline, Some(i)) => {
        self.rows[i].legs.retain(|l| !(l.0 == c.platform && l.1 == c.key));
        if self.rows[i].legs.is_empty() {
          self.rows.remove(i);
        }
      }
      (Kind::Offline, None) | (Kind::Update, None) => {}
      (_, Some(i)) => {
        let r = &mut self.rows[i];
        if !c.game.is_empty() {
          r.game = c.game;
        }
        if !c.title.is_empty() {
          r.title = c.title;
        }
      }
      (Kind::Online, None) => {
        let name = if c.name.is_empty() { c.key.trim_start_matches('@').into() } else { c.name };
        self.put(Live { name, game: c.game, title: c.title, viewers: 0, since: None, legs: vec![(c.platform, c.key)] });
      }
    }
    self.sort();
    self.rows != before
  }
}

// 1.2k, 12k, 1.3m
pub fn compact(n: u64) -> String {
  match n {
    n if n >= 10_000_000 => format!("{}m", n / 1_000_000),
    n if n >= 1_000_000 => format!("{:.1}m", n as f64 / 1e6).replace(".0m", "m"),
    n if n >= 10_000 => format!("{}k", n / 1000),
    n if n >= 1000 => format!("{:.1}k", n as f64 / 1e3).replace(".0k", "k"),
    n => n.to_string(),
  }
}

// how long they have been live: 5m, 3h12m, 1d4h. None when the server gave no start
pub fn uptime(now: i64, since: Option<i64>) -> Option<String> {
  let mins = (now - since?).max(0) / 60_000;
  Some(match mins {
    0 => "<1m".into(),
    m if m < 60 => format!("{m}m"),
    m if m < 1440 => format!("{}h{:02}m", m / 60, m % 60),
    m => format!("{}d{}h", m / 1440, m % 1440 / 60),
  })
}

pub fn tag(l: &Live) -> String {
  let t: String = l.legs.iter().map(|(p, _)| super::mark::Mark::from(*p).letter()).collect();
  format!("[{t}]")
}

// the right half of a row: game · viewers · uptime
pub fn detail(l: &Live, now: i64) -> String {
  let mut parts = Vec::with_capacity(3);
  if !l.game.is_empty() {
    parts.push(l.game.to_string());
  }
  if l.viewers > 0 {
    parts.push(compact(l.viewers));
  }
  parts.extend(uptime(now, l.since));
  parts.join(" · ")
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn live(p: &str, name: &str, viewers: u64) -> Value {
    json!({"platform": p, "username": name, "displayName": name, "category": "Chess", "title": "t", "viewerCount": viewers})
  }

  #[test]
  fn rows_read_the_servers_shape_and_sort_by_audience() {
    let v = json!([
      live("twitch", "Small", 5),
      {
        "platform": "twitch", "username": "big", "displayName": "Big", "heatsyncDisplayName": "Biggie",
        "category": "Just Chatting", "title": "hi", "viewerCount": 1200, "startedAt": "2026-11-04T10:00:00.000Z",
        "platformUsernames": {"twitch": "big", "kick": "Big-K", "youtube": "@bigyt"}
      },
      live("kick", "mid", 90),
    ]);
    let rows = parse(Some(&v));
    assert_eq!(rows.iter().map(|r| &*r.name).collect::<Vec<_>>(), ["Biggie", "mid", "Small"]);
    let big = &rows[0];
    assert_eq!(&*big.game, "Just Chatting");
    assert_eq!(big.viewers, 1200);
    assert!(big.since.is_some_and(|t| t > 0));
    let legs: Vec<_> = big.legs.iter().map(|(p, k)| (p.name(), &**k)).collect();
    assert_eq!(legs, [("twitch", "big"), ("kick", "big-k"), ("youtube", "@bigyt")]);
    assert_eq!(tag(big), "[TKY]");
    assert_eq!(rows[2].primary().1.as_ref(), "small", "keys are lowercase, the name stays as written");
  }

  #[test]
  fn hostile_rows_are_dropped_or_cut_down() {
    let evil = json!([
      {"platform": "twitch", "username": "a/../b", "displayName": "x"},
      {"platform": "twitch", "username": "ok_name\n", "displayName": "x"},
      {"platform": "mixer", "username": "ok"},
      {"platform": "youtube", "username": "UCabcdefghijklmnopqrstuv"},
      {"platform": "twitch"},
      {"username": "nope"},
      42, null, "x", [],
      {"platform": "twitch", "username": "fine", "displayName": "a\u{7}b\nc", "viewerCount": -5, "title": 7,
       "category": "x".repeat(10_000), "startedAt": "garbage", "platformUsernames": {"kick": "BAD NAME", "youtube": 9}},
      {"platform": "twitch", "username": "huge", "viewerCount": 1e30},
      {"platform": "twitch", "username": "nan", "viewerCount": "lots"},
    ]);
    let rows = parse(Some(&evil));
    let names: Vec<&str> = rows.iter().map(|r| &*r.legs[0].1).collect();
    assert_eq!(names, ["huge", "fine", "nan", "ok_name"], "a stray newline is trimmed, the rest is refused");
    let fine = rows.iter().find(|r| &*r.legs[0].1 == "fine").unwrap();
    assert_eq!(&*fine.name, "a b c");
    assert_eq!(fine.viewers, 0);
    assert_eq!(fine.game.chars().count(), 100);
    assert_eq!((&*fine.title, fine.since, fine.legs.len()), ("", None, 1));
    assert_eq!(rows[0].viewers, MAX_VIEWERS, "a silly count is clamped");
    // not an array, nothing at all
    assert!(parse(Some(&json!({"streams": 1}))).is_empty() && parse(None).is_empty());
  }

  #[test]
  fn a_flood_is_bounded_and_one_row_per_channel() {
    let many: Vec<Value> = (0..5000).map(|i| live("twitch", &format!("u{i}"), i)).collect();
    assert_eq!(parse(Some(&Value::Array(many))).len(), MAX_ROWS);
    let twice = json!([live("twitch", "same", 1), live("twitch", "SAME", 9)]);
    let rows = parse(Some(&twice));
    assert_eq!((rows.len(), rows[0].viewers), (1, 9));
    // a merged card and a plain row of one of its legs are one person
    let both = json!([live("kick", "k", 3), {"platform": "twitch", "username": "t", "platformUsernames": {"kick": "k"}, "viewerCount": 8}]);
    assert_eq!(parse(Some(&both)).len(), 1);
  }

  fn frame(ty: &str, ch: &str) -> Value {
    json!({"type": ty, "platform": "twitch", "channel": ch, "displayName": "Ennortix", "game": "Chess", "title": "x"})
  }

  #[test]
  fn the_socket_keeps_the_list_current() {
    let mut f = Follows::default();
    f.replace(parse(Some(&json!([live("twitch", "a", 10), live("kick", "b", 20)]))));
    let ch = |k, v: Value| Change::from_frame(k, &v).unwrap();
    // a go-live adds a row (no audience yet, so it sorts last); a repeat changes nothing
    assert!(f.change(ch(Kind::Online, frame("follow:stream:online", "ennortix"))));
    assert!(!f.change(ch(Kind::Online, frame("follow:stream:online", "ennortix"))));
    assert_eq!(f.rows().len(), 3);
    assert_eq!(&*f.rows()[2].name, "Ennortix");
    // a game switch updates the row, but never adds one
    let mut sw = frame("follow:stream:update", "a");
    sw["game"] = json!("Go");
    assert!(f.change(ch(Kind::Update, sw)));
    assert_eq!(&*f.rows().iter().find(|r| &*r.legs[0].1 == "a").unwrap().game, "Go");
    assert!(!f.change(ch(Kind::Update, frame("follow:stream:update", "stranger"))));
    assert_eq!(f.rows().len(), 3);
    // sign-off removes it; for a leg of a merged row only that leg goes
    assert!(f.change(ch(Kind::Offline, frame("follow:stream:offline", "ennortix"))));
    assert!(!f.change(ch(Kind::Offline, frame("follow:stream:offline", "ennortix"))));
    let mut g = Follows::default();
    g.replace(parse(Some(&json!([{"platform": "twitch", "username": "t", "platformUsernames": {"kick": "k"}}]))));
    assert!(g.change(ch(Kind::Offline, frame("follow:stream:offline", "t"))));
    assert_eq!(g.rows().len(), 1);
    assert_eq!(g.rows()[0].primary().0, Platform::Kick, "the kick leg is the row now");
    // the socket's catch-up adds and refreshes but never removes; a full answer replaces
    f.union(parse(Some(&json!([live("twitch", "c", 1)]))));
    assert_eq!(f.rows().len(), 3);
    f.replace(parse(Some(&json!([live("twitch", "c", 1)]))));
    assert_eq!(f.rows().len(), 1);
  }

  #[test]
  fn hostile_frames_make_no_change() {
    for v in [
      json!({"platform": "twitch", "channel": "a b"}),
      json!({"platform": "twitch", "channel": ""}),
      json!({"platform": "nope", "channel": "x"}),
      json!({"platform": "twitch", "channel": 5}),
      json!({"channel": "x"}),
    ] {
      assert!(Change::from_frame(Kind::Online, &v).is_none(), "{v}");
    }
    // a display name that isn't their channel in other letters is not trusted
    let c =
      Change::from_frame(Kind::Online, &json!({"platform": "twitch", "channel": "x", "displayName": "Someone Else"}));
    assert_eq!(&*c.unwrap().name, "");
  }

  #[test]
  fn numbers_and_times_read_short() {
    assert_eq!(
      [0, 999, 1000, 1234, 12_345, 1_300_000, 25_000_000].map(compact),
      ["0", "999", "1k", "1.2k", "12k", "1.3m", "25m"]
    );
    let now = 10 * 86_400_000;
    let up = |ago_min: i64| uptime(now, Some(now - ago_min * 60_000));
    assert_eq!(up(0).as_deref(), Some("<1m"));
    assert_eq!(up(45).as_deref(), Some("45m"));
    assert_eq!(up(192).as_deref(), Some("3h12m"));
    assert_eq!(up(1440 + 240).as_deref(), Some("1d4h"));
    assert_eq!(up(-5).as_deref(), Some("<1m"), "a start in the future reads as just started");
    assert_eq!(uptime(now, None), None);
    let l = row(&json!({"platform": "twitch", "username": "a", "category": "Chess", "viewerCount": 2500, "startedAt": "1970-01-10T22:00:00Z"})).unwrap();
    assert_eq!(detail(&l, now), "Chess · 2.5k · 2h00m");
    let bare = row(&live("kick", "b", 0)).map(|mut l| {
      l.game = "".into();
      l
    });
    assert_eq!(detail(&bare.unwrap(), now), "");
  }
}
