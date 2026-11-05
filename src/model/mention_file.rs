// the mentions tab between runs (ext main.js hs_mentions_v2): the last 500 lines that pinged you, written to
// the `mentions` file with the heatsync account they belong to and when. a run restores them only for that
// same account and only while the file is under a day old; they come back as history (no ping, no flash).
// the file is local but read like any other input: a size cap before reading, a line cap, every field
// bounded and typed, every url on the image allowlist, every emote range on a character boundary of its
// own line. one line of json per mention under a header line
use serde_json::{Value, json};

use super::message::{Extra, HsEmote, Kind, Message, Placed, Platform, REPLY_TEXT, flag};
use crate::heatsync::valid_name;
use crate::images::url::allowed;
use crate::util::clamp;

pub const FILE: &str = "mentions";
// lines kept (the ext's 500)
pub const KEEP: usize = 500;
// how long a file stays good
pub const MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
// a file bigger than this is not ours (500 full lines with emotes come to well under half of it)
pub const MAX_BYTES: u64 = 2 << 20;
// what a write may add up to: the oldest lines go first
const WRITE_BYTES: usize = 1 << 20;
const MAX_BODY: usize = 2000;
const MAX_NAME: usize = 100;
const MAX_ID: usize = 128;
const MAX_EMOTES: usize = 50;
const MAX_BADGES: usize = 20;
const MAX_BADGE: usize = 64;
// the flags worth keeping: the red edge and the highlight. the rest (first, new, history…) are about a
// session, and a restored line is history
const KEPT: u8 = flag::ALERT | flag::HIGHLIGHTED;

fn tag(p: Platform) -> &'static str {
  match p {
    Platform::Twitch => "t",
    Platform::Kick => "k",
    Platform::Youtube => "y",
  }
}

fn untag(s: &str) -> Option<Platform> {
  Some(match s {
    "t" => Platform::Twitch,
    "k" => Platform::Kick,
    "y" => Platform::Youtube,
    _ => return None,
  })
}

fn row(m: &Message) -> Value {
  let room = m.x.as_deref().map_or("", |x| &x.chan);
  json!({
    "p": tag(m.platform),
    "a": m.kind == Kind::Action,
    "id": m.id,
    "ts": m.ts,
    "login": m.login,
    "uid": m.user_id,
    "nick": m.nick,
    "col": m.color,
    "body": m.body,
    "room": room,
    "fl": m.flags & KEPT,
    "em": m.emotes.iter().map(|(r, u)| json!([r.start, r.end, u])).collect::<Vec<_>>(),
    "hs": m.hs.iter().map(|(w, u, z)| json!([w, u, z])).collect::<Vec<_>>(),
    "bd": m.badges.iter().map(|(s, v)| json!([s, v])).collect::<Vec<_>>(),
    "rt": m.reply_to,
    "rx": m.reply_text,
  })
}

// the file for `owner` (the heatsync username, lowercased) at `now`: the newest KEEP lines that fit, oldest first
pub fn encode<'a>(owner: &str, now: i64, rows: impl DoubleEndedIterator<Item = &'a Message>) -> String {
  let head = json!({"v": 1, "owner": owner, "ts": now}).to_string();
  let mut used = head.len();
  let mut lines: Vec<String> = Vec::new();
  for m in rows.rev().filter(|m| m.kind.said()).take(KEEP) {
    let l = row(m).to_string();
    if used + l.len() + 1 > WRITE_BYTES {
      break;
    }
    used += l.len() + 1;
    lines.push(l);
  }
  let mut out = head;
  out.push('\n');
  for l in lines.iter().rev() {
    out.push_str(l);
    out.push('\n');
  }
  out
}

#[derive(Debug)]
pub enum Read {
  // the lines (oldest first) and how many were unusable
  Rows { rows: Vec<Message>, junk: usize },
  // the file is another account's, or a day old: leave it be
  NotYours,
  Stale,
  // no readable header: not a file of ours
  Bad,
}

pub fn decode(text: &str, owner: &str, now: i64) -> Read {
  let mut lines = text.lines();
  let Some(head) = lines.next().and_then(|l| serde_json::from_str::<Value>(l).ok()) else { return Read::Bad };
  let (Some(who), Some(ts), Some(1)) = (
    head.get("owner").and_then(Value::as_str),
    head.get("ts").and_then(Value::as_i64),
    head.get("v").and_then(Value::as_u64),
  ) else {
    return Read::Bad;
  };
  if who != owner || owner.is_empty() {
    return Read::NotYours;
  }
  if now.saturating_sub(ts) > MAX_AGE_MS || ts > now.saturating_add(MAX_AGE_MS) {
    return Read::Stale;
  }
  let mut rows = Vec::new();
  let mut junk = 0;
  for l in lines.filter(|l| !l.trim().is_empty()) {
    // past the cap the rest are not read at all
    if rows.len() + junk >= KEEP {
      break;
    }
    match serde_json::from_str::<Value>(l).ok().and_then(|v| message(&v)) {
      Some(m) => rows.push(m),
      None => junk += 1,
    }
  }
  Read::Rows { rows, junk }
}

fn text(v: &Value, k: &str, max: usize) -> Option<String> {
  let s = v.get(k)?.as_str()?;
  (s.len() <= max).then(|| s.chars().filter(|c| !c.is_control()).collect())
}

fn pairs<'a>(v: &'a Value, k: &str, max: usize) -> Vec<&'a Vec<Value>> {
  v.get(k)
    .and_then(Value::as_array)
    .map(|a| a.iter().filter_map(Value::as_array).take(max).collect())
    .unwrap_or_default()
}

fn message(v: &Value) -> Option<Message> {
  let platform = untag(v.get("p")?.as_str()?)?;
  let body = text(v, "body", MAX_BODY)?;
  let login = text(v, "login", MAX_NAME).filter(|l| !l.is_empty())?;
  let room = text(v, "room", MAX_NAME).filter(|r| !r.is_empty())?;
  let ts = v.get("ts")?.as_i64()?;
  let color = u32::try_from(v.get("col")?.as_u64()?).ok().filter(|c| *c <= 0xff_ffff)?;
  let nick = text(v, "nick", MAX_NAME).filter(|n| !n.is_empty()).unwrap_or_else(|| login.clone());
  let emotes: Vec<Placed> = pairs(v, "em", MAX_EMOTES)
    .into_iter()
    .filter_map(|e| {
      let (a, b) = (u32::try_from(e.first()?.as_u64()?).ok()?, u32::try_from(e.get(1)?.as_u64()?).ok()?);
      let url = e.get(2)?.as_str().filter(|u| allowed(u))?;
      let ok = a < b && body.get(a as usize..b as usize).is_some();
      ok.then(|| (a..b, url.into()))
    })
    .collect();
  let hs: Vec<HsEmote> = pairs(v, "hs", MAX_EMOTES)
    .into_iter()
    .filter_map(|e| {
      let word = e.first()?.as_str().filter(|w| valid_name(w))?;
      let url = e.get(1)?.as_str().filter(|u| allowed(u))?;
      Some((word.into(), url.into(), e.get(2).and_then(Value::as_bool) == Some(true)))
    })
    .collect();
  let badges = pairs(v, "bd", MAX_BADGES)
    .into_iter()
    .filter_map(|b| {
      let (s, n) = (b.first()?.as_str()?, b.get(1)?.as_str()?);
      let clean = |t: &str| t.len() <= MAX_BADGE && !t.chars().any(char::is_control);
      (!s.is_empty() && clean(s) && clean(n)).then(|| (s.into(), n.into()))
    })
    .collect::<Vec<_>>();
  let mut m = Message::system(ts, platform, "");
  m.kind = if v.get("a").and_then(Value::as_bool) == Some(true) { Kind::Action } else { Kind::Chat };
  m.id = text(v, "id", MAX_ID)?.into();
  m.login = login.to_lowercase().into();
  m.user_id =
    text(v, "uid", 32).filter(|u| u.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).unwrap_or_default().into();
  m.nick = nick.into();
  m.color = color;
  m.body = body.into();
  m.emotes = emotes.into();
  m.hs = hs.into();
  m.badges = badges.into();
  m.reply_to = text(v, "rt", MAX_NAME).filter(|t| !t.is_empty()).map(Into::into);
  m.reply_text = text(v, "rx", 4 * REPLY_TEXT).filter(|t| !t.is_empty()).map(|t| clamp(&t, REPLY_TEXT).into());
  // restored lines are history: they never ping, notify or flash, and are not new people
  let kept = u8::try_from(v.get("fl").and_then(Value::as_u64).unwrap_or(0) & u64::from(KEPT)).unwrap_or(0);
  m.flags = kept | flag::HISTORY;
  m.x = Some(Box::new(Extra { chan: room.into(), ..Extra::default() }));
  Some(m)
}

#[cfg(test)]
mod tests {
  use super::*;

  const NOW: i64 = 1_800_000_000_000;

  fn mention(id: &str, body: &str) -> Message {
    let mut m = Message::system(NOW - 1000, Platform::Twitch, "");
    (m.kind, m.id, m.login, m.nick, m.user_id, m.color) =
      (Kind::Chat, id.into(), "bob".into(), "Bob".into(), "42".into(), 0x00ff00);
    m.body = body.into();
    m.flags = flag::ALERT | flag::NEW | flag::FIRST | flag::OWN;
    m.x = Some(Box::new(Extra { chan: "forsen".into(), ..Extra::default() }));
    m
  }

  fn rows(r: Read) -> Vec<Message> {
    match r {
      Read::Rows { rows, junk: 0 } => rows,
      other => panic!("{other:?}"),
    }
  }

  #[test]
  fn a_line_comes_back_as_history_with_what_it_needs_to_draw() {
    let mut m = mention("a", "hi @mellen Kappa ok");
    m.emotes = vec![(10u32..15, "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0".into())].into();
    m.hs = vec![("catJAM".into(), "https://cdn.heatsync.org/uploads/a.webp".into(), true)].into();
    m.badges = vec![("subscriber".into(), "12".into())].into();
    m.reply_to = Some("wollip".into());
    m.reply_text = Some("is this on".into());
    let back = rows(decode(&encode("mellen", NOW, [m.clone()].iter()), "mellen", NOW + 5000));
    let b = &back[0];
    assert_eq!(
      (&*b.id, &*b.login, &*b.nick, &*b.user_id, b.color, &*b.body),
      ("a", "bob", "Bob", "42", 0x00ff00, &*m.body)
    );
    assert_eq!(b.x.as_ref().map(|x| &*x.chan), Some("forsen"));
    assert_eq!((b.emotes.clone(), b.hs.clone(), b.badges.clone()), (m.emotes, m.hs, m.badges));
    assert_eq!((b.reply_to.as_deref(), b.reply_text.as_deref()), (Some("wollip"), Some("is this on")));
    assert_eq!(b.flags, flag::ALERT | flag::HISTORY, "the red edge stays; first / new / own do not; it is history");
  }

  #[test]
  fn only_the_newest_500_are_kept_oldest_first() {
    let all: Vec<Message> = (0..600).map(|i| mention(&i.to_string(), "x")).collect();
    let back = rows(decode(&encode("mellen", NOW, all.iter()), "mellen", NOW));
    assert_eq!(back.len(), KEEP);
    assert_eq!((&*back[0].id, &*back[KEEP - 1].id), ("100", "599"));
  }

  #[test]
  fn another_account_a_stale_file_and_junk_are_not_restored() {
    let file = encode("mellen", NOW, [mention("a", "x")].iter());
    assert!(matches!(decode(&file, "someone", NOW), Read::NotYours));
    assert!(matches!(decode(&file, "", NOW), Read::NotYours), "signed out has no owner");
    assert!(matches!(decode(&file, "mellen", NOW + MAX_AGE_MS + 1), Read::Stale));
    assert!(matches!(decode(&file, "mellen", NOW - MAX_AGE_MS - 1), Read::Stale), "a clock-skewed future stamp");
    assert!(matches!(decode("not json\n", "mellen", NOW), Read::Bad));
    assert!(matches!(decode("", "mellen", NOW), Read::Bad));
    assert!(matches!(decode("{\"v\":2,\"owner\":\"mellen\",\"ts\":1}\n", "mellen", NOW), Read::Bad));
  }

  #[test]
  fn hostile_lines_are_dropped_or_cut_down_never_trusted() {
    let head = json!({"v": 1, "owner": "mellen", "ts": NOW}).to_string();
    let good = row(&mention("ok", "héllo"));
    let with = |k: &str, v: Value| {
      let mut r = good.clone();
      r[k] = v;
      r.to_string()
    };
    let lines = [
      with("body", json!("x".repeat(MAX_BODY + 1))),
      with("login", json!("")),
      with("room", json!("")),
      with("p", json!("q")),
      with("col", json!(0x1_000_000)),
      with("ts", json!("soon")),
      "[1,2]".into(),
      "{broken".into(),
    ];
    let file = format!("{head}\n{}\n{good}\n", lines.join("\n"));
    let Read::Rows { rows, junk } = decode(&file, "mellen", NOW) else { panic!() };
    assert_eq!((rows.len(), junk), (1, lines.len()));
    // a bad url, a range inside a character, backwards, past the end, an odd word: each is dropped alone
    let mut r = good.clone();
    r["em"] = json!([
      [0, 1, "https://evil.example/x.png"],
      [1, 2, "https://cdn.7tv.app/emote/x/1x.webp"],
      [3, 1, "https://cdn.7tv.app/a"],
      [0, 99, "https://cdn.7tv.app/a"],
      [0, 1, "https://cdn.7tv.app/ok"]
    ]);
    r["hs"] = json!([
      ["a b", "https://cdn.heatsync.org/x.webp", false],
      ["fine", "https://evil.example/y", false],
      ["good", "https://cdn.heatsync.org/z.webp", false]
    ]);
    r["bd"] = json!([["", "1"], ["sub\u{7}", "1"], ["ok", "2"]]);
    r["nick"] = json!("a\u{1b}[31mb");
    let m = message(&r).unwrap();
    assert_eq!(m.emotes.iter().map(|e| (e.0.start, e.0.end)).collect::<Vec<_>>(), [(0, 1)]);
    assert_eq!(m.hs.iter().map(|h| &*h.0).collect::<Vec<_>>(), ["good"]);
    assert_eq!(m.badges.iter().map(|b| &*b.0).collect::<Vec<_>>(), ["ok"]);
    assert_eq!(&*m.nick, "a[31mb", "no control bytes");
  }

  #[test]
  fn lines_past_the_cap_are_not_read() {
    let head = json!({"v": 1, "owner": "mellen", "ts": NOW}).to_string();
    let good = row(&mention("a", "x")).to_string();
    let file = format!("{head}\n{}", format!("{good}\n").repeat(KEEP + 50));
    let Read::Rows { rows, .. } = decode(&file, "mellen", NOW) else { panic!() };
    assert_eq!(rows.len(), KEEP);
  }
}
