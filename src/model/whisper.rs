// twitch whispers and heatsync dms (ext whispers.js): one chronological timeline of everything said
// to and by you, `hh:mm [T] you → them: text` (`[H]` for a dm). the server keeps whispers (it holds the
// EventSub subscription) and pushes `whisper:new`; sends go through /api/twitch/whisper. dms are
// `dm:new` pushes and POST /api/dm. pure: parsing, dedupe, caps, row text
use super::ident;
use std::collections::{HashSet, VecDeque};

use serde_json::Value;

use super::color;
use super::message::{Extra, Kind, Message, Platform, Seg, flag};
use crate::ui::theme;
use crate::util::time::{now_ms, parse_iso};

// the ext keeps 500 read + 500 unread; one ring of 500 here
pub const MAX: usize = 500;
const MAX_TEXT: usize = 2000;
const MAX_SEEN: usize = 2000;
// a local echo and the server's copy of the same send land within this of each other
const ECHO_MS: i64 = 120_000;

// which wire a conversation rides (the ext keys them `twitch:` / `hs:`). the same digits can be a
// twitch id and a heatsync id, so a person is always (via, id)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
  Twitch,
  Heatsync,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Who {
  pub via: Via,
  // twitch user id (digits) or heatsync user id. empty = not resolved yet
  pub id: Box<str>,
  pub login: Box<str>,
  pub name: Box<str>,
  pub color: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fail {
  // twitch whispers permission missing: re-link with the whispers pack
  Relink,
  // not logged in on heatsync
  Auth,
  // dms need a follow both ways: the login to follow first
  Follow(Box<str>),
  Server,
  Other(Box<str>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
  Sent,
  Pending,
  Failed(Fail),
}

#[derive(Clone, Debug)]
pub struct Whisper {
  pub key: u64,
  // the server's id, for dedupe. empty on a local echo
  pub ext: Box<str>,
  pub ts: i64,
  pub out: bool,
  pub who: Who,
  pub text: Box<str>,
  pub status: Status,
}

#[derive(Default)]
pub struct Timeline {
  list: VecDeque<Whisper>,
  seen: HashSet<Box<str>>,
  seen_order: VecDeque<Box<str>>,
  next: u64,
  // /r target: the last person involved in a whisper, either direction (ext lastWhisperKey)
  pub last: Option<Who>,
}

fn clean(s: &str, max: usize) -> String {
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().to_string()
}

fn digits(v: Option<&Value>) -> Option<Box<str>> {
  let s = match v? {
    Value::String(s) => s.clone(),
    Value::Number(n) => n.to_string(),
    _ => return None,
  };
  (1..=20).contains(&s.len()).then_some(s).filter(|s| s.bytes().all(|b| b.is_ascii_digit())).map(Into::into)
}

fn text_of(v: &Value, k: &str) -> Option<String> {
  v.get(k).and_then(Value::as_str).map(|s| clean(s, 100)).filter(|s| !s.is_empty())
}

// the colour a whisper row stores for a name: readable, or white when the server sent none
fn color_of(v: &Value, k: &str) -> u32 {
  v.get(k).and_then(Value::as_str).and_then(color::parse_hex).map_or(theme::WHITE, color::readable)
}

// a `whisper:new` frame or one row of GET /api/whispers/:id. `me` is our twitch id (0 = unknown);
// `other` fills in the person for rows that carry no name of theirs (sent by us)
pub fn parse(v: &Value, me: u64, other: Option<&Who>) -> Option<Whisper> {
  let from = digits(v.get("from_twitch_id"))?;
  let to = digits(v.get("to_twitch_id"));
  let out = me != 0 && *from == *me.to_string();
  let text = clean(v.get("content").and_then(Value::as_str)?, MAX_TEXT);
  if text.is_empty() {
    return None;
  }
  let who = if out {
    let id = to.or_else(|| other.map(|o| o.id.clone()))?;
    let login = text_of(v, "to_twitch_login").map(|l| l.to_lowercase());
    let login: Box<str> = login.map_or_else(|| other.map_or("".into(), |o| o.login.clone()), Into::into);
    let name = other.map_or_else(|| login.clone(), |o| o.name.clone());
    Who { via: Via::Twitch, id, login, name, color: other.map_or(theme::WHITE, |o| o.color) }
  } else {
    let login: Box<str> = text_of(v, "from_twitch_login").unwrap_or_default().to_lowercase().into();
    let name = text_of(v, "from_twitch_display")
      .or_else(|| text_of(v, "from_display_name"))
      .map_or_else(|| login.clone(), Into::into);
    Who { via: Via::Twitch, id: from, login, name, color: color_of(v, "from_color") }
  };
  let ext: Box<str> = v
    .get("external_message_id")
    .and_then(Value::as_str)
    .filter(|s| !s.is_empty() && s.len() <= 64)
    .map(String::from)
    .or_else(|| digits(v.get("id")).map(|d| format!("db{d}")))
    .unwrap_or_default()
    .into();
  let ts = v.get("created_at").and_then(Value::as_str).and_then(parse_iso).unwrap_or_else(now_ms);
  Some(Whisper { key: 0, ext, ts, out, who, text: text.into(), status: Status::Sent })
}

// a heatsync user id: digits, or the kick_ / yt_ namespaced forms (server PLATFORM_USER_ID_REGEX)
pub fn user_id(v: Option<&Value>) -> Option<Box<str>> {
  let s = match v? {
    Value::String(s) => s.clone(),
    Value::Number(n) => n.to_string(),
    _ => return None,
  };
  ident::slug(&s, 64).then(|| s.into())
}

// a `dm:new` frame (`other` None: from them, to us) or one row of GET /api/dm/:userId, where `other`
// is the person the conversation is with: a row they sent is theirs, anything else we sent
pub fn parse_dm(v: &Value, other: Option<&Who>) -> Option<Whisper> {
  let from = user_id(v.get("from_user_id"))?;
  let text = clean(v.get("content").and_then(Value::as_str)?, MAX_TEXT);
  if text.is_empty() {
    return None;
  }
  let out = other.is_some_and(|o| *o.id != *from);
  let who = match other {
    Some(o) => o.clone(),
    None => {
      let name = text_of(v, "from_display_name")?;
      Who {
        via: Via::Heatsync,
        id: from,
        login: name.to_lowercase().into(),
        name: name.into(),
        color: color_of(v, "from_color"),
      }
    }
  };
  let ext: Box<str> = user_id(v.get("id")).map_or_else(Default::default, |d| format!("dm{d}").into());
  let ts = v.get("created_at").and_then(Value::as_str).and_then(parse_iso).unwrap_or_else(now_ms);
  Some(Whisper { key: 0, ext, ts, out, who, text: text.into(), status: Status::Sent })
}

impl Timeline {
  fn mark(&mut self, k: Box<str>) -> bool {
    if !self.seen.insert(k.clone()) {
      return false;
    }
    self.seen_order.push_back(k);
    if self.seen_order.len() > MAX_SEEN
      && let Some(old) = self.seen_order.pop_front()
    {
      self.seen.remove(&old);
    }
    true
  }

  #[cfg(test)]
  pub fn len(&self) -> usize {
    self.list.len()
  }

  pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, Whisper> {
    self.list.iter()
  }

  // a person we already know the id of, from any whisper or dm with them
  pub fn known(&self, via: Via, login: &str) -> Option<Who> {
    self
      .list
      .iter()
      .rev()
      .find(|w| w.who.via == via && *w.who.login == *login && !w.who.id.is_empty())
      .map(|w| w.who.clone())
  }

  pub fn get(&self, key: u64) -> Option<&Whisper> {
    self.list.iter().find(|w| w.key == key)
  }

  // the server's copy of a whisper (live push or history). returns the stored one and whether it
  // is new; a duplicate (already seen, or the server's echo of a send we showed) is None
  pub fn add(&mut self, mut w: Whisper) -> Option<&Whisper> {
    if !w.ext.is_empty() {
      // our own send comes back in history: it is the local echo, now with its server id
      if w.out
        && let Some(e) = self.list.iter_mut().find(|e| {
          e.out
            && e.ext.is_empty()
            && e.who.via == w.who.via
            && e.who.id == w.who.id
            && e.text == w.text
            && (e.ts - w.ts).abs() < ECHO_MS
        })
      {
        e.ext = w.ext.clone();
        self.mark(w.ext);
        return None;
      }
      if !self.mark(w.ext.clone()) {
        return None;
      }
    }
    self.next += 1;
    w.key = self.next;
    // history arrives oldest-first after live ones may exist: keep time order
    let at = self.list.iter().rposition(|x| x.ts <= w.ts).map_or(0, |i| i + 1);
    if !w.out {
      self.last = Some(w.who.clone());
    }
    self.list.insert(at, w);
    if self.list.len() > MAX {
      self.list.pop_front();
      // older than everything we keep: it was the one dropped
      return at.checked_sub(1).and_then(|i| self.list.get(i));
    }
    self.list.get(at)
  }

  // a send of ours, shown at once as pending
  pub fn push_out(&mut self, who: Who, text: &str) -> &Whisper {
    self.next += 1;
    let w = Whisper {
      key: self.next,
      ext: "".into(),
      ts: now_ms(),
      out: true,
      who: who.clone(),
      text: text.into(),
      status: Status::Pending,
    };
    self.last = Some(who);
    self.list.push_back(w);
    if self.list.len() > MAX {
      self.list.pop_front();
    }
    self.list.back().expect("just pushed")
  }

  // the send settled; a resolved person fills in the id/name the echo lacked
  pub fn settle(&mut self, key: u64, status: Status, who: Option<Who>) -> Option<&Whisper> {
    let w = self.list.iter_mut().find(|w| w.key == key)?;
    w.status = status;
    if let Some(p) = who {
      w.who = p.clone();
      if self.last.as_ref().is_some_and(|l| l.via == p.via && l.login == p.login) {
        self.last = Some(p);
      }
    }
    Some(&*w)
  }

  // make a failed send pending again; the text goes out as before
  pub fn retry(&mut self, key: u64) -> Option<&Whisper> {
    let w = self.list.iter_mut().find(|w| w.key == key && matches!(w.status, Status::Failed(_)))?;
    w.status = Status::Pending;
    Some(&*w)
  }
}

pub fn row_id(key: u64) -> Box<str> {
  format!("w{key}").into()
}

pub fn key_of(row: &str) -> Option<u64> {
  row.strip_prefix('w')?.parse().ok()
}

// ext failure text (locale mc_whisper_*)
// a history read is only for the login that asked for it; a live push (no epoch) is always current
pub fn current(tag: Option<u32>, now: u32) -> bool {
  tag.is_none_or(|t| t == now)
}

pub fn fail_text(f: &Fail) -> String {
  match f {
    Fail::Relink => "twitch whispers not enabled. re-link on heatsync.org".into(),
    Fail::Auth => "log in to heatsync.org to send whispers and dms".into(),
    Fail::Follow(login) => format!("dms need a follow both ways. /follow {login} first"),
    Fail::Server => "heatsync server unreachable".into(),
    Fail::Other(e) => e.to_string(),
  }
}

pub fn message(w: &Whisper) -> Message {
  let (arrow, to_name, to_ink) =
    if w.out { (theme::HEATSYNC, w.who.name.clone(), w.who.color) } else { (theme::CYAN, "you".into(), theme::WHITE) };
  let mut body = w.text.to_string();
  let mut flags = if w.out { flag::BRAND } else { flag::HIGHLIGHTED };
  match &w.status {
    Status::Sent => {}
    Status::Pending => body.push_str(" …"),
    Status::Failed(f) => {
      flags = flag::ALERT;
      let how = match f {
        Fail::Relink => "enable twitch whispers: enter to re-link".to_string(),
        other => format!("{}: enter to retry", fail_text(other)),
      };
      body.push_str(&format!(" ⚠ {how}"));
    }
  }
  let (brand, ink) = match w.who.via {
    Via::Twitch => ("[T] ", theme::TWITCH),
    Via::Heatsync => ("[H] ", theme::HEATSYNC),
  };
  let mut m = Message::system(w.ts, Platform::Twitch, "");
  m.kind = Kind::Chat;
  m.id = row_id(w.key);
  m.flags = flags;
  m.login = w.who.login.clone();
  // their paint rides on the name; your own side reads "you"
  if !w.out {
    m.user_id = w.who.id.clone();
  }
  m.nick = if w.out { "you".into() } else { w.who.name.clone() };
  m.color = if w.out { theme::WHITE } else { w.who.color };
  m.body = body.into();
  m.x = Some(Box::new(Extra {
    chan: "".into(),
    head: Box::new([Seg { text: brand.into(), ink, bold: true }]),
    tail: Box::new([
      Seg { text: " → ".into(), ink: arrow, bold: true },
      Seg { text: to_name.to_string().into(), ink: to_ink, bold: true },
    ]),
    edge: 0,
    paint: None,
    pic: None,
    late: false,
    held: None,
  }));
  m
}

#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
  // /w user [text] or /dm user [text]: with no text it only opens the conversation
  To(Via, Box<str>, Option<String>),
  // /r text
  Reply(String),
  // /w, /dm or /r with nothing usable
  Usage,
}

pub fn command(line: &str) -> Option<Cmd> {
  let line = line.trim();
  let (head, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
  let rest = rest.trim();
  match head.to_ascii_lowercase().as_str() {
    "/w" | "/whisper" | "/dm" => {
      let via = if head.eq_ignore_ascii_case("/dm") { Via::Heatsync } else { Via::Twitch };
      let (user, text) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
      let user = user.trim_start_matches('@');
      if !ident::twitch_login(user) {
        return Some(Cmd::Usage);
      }
      let text = text.trim();
      Some(Cmd::To(via, user.to_ascii_lowercase().into(), (!text.is_empty()).then(|| text.to_string())))
    }
    "/r" | "/reply" => Some(if rest.is_empty() { Cmd::Usage } else { Cmd::Reply(rest.to_string()) }),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn live(id: &str, from: &str, text: &str, ts: &str) -> Value {
    json!({"external_message_id": id, "from_twitch_id": from, "from_twitch_login": "Bob", "from_twitch_display": "Bob",
      "to_twitch_id": "1", "to_twitch_login": "me", "from_color": "#ff0000", "content": text, "created_at": ts})
  }

  #[test]
  fn parses_a_push_and_a_history_row() {
    let w = parse(&live("a1", "42", "hey\nthere", "2026-10-30T10:00:00.000Z"), 0, None).unwrap();
    assert!(!w.out);
    assert_eq!((&*w.who.id, &*w.who.login, &*w.who.name), ("42", "bob", "Bob"));
    assert_eq!(&*w.text, "hey there");
    assert_eq!(&*w.ext, "a1");
    // a row we sent: other person comes from the `to` side
    let mut row = live("a2", "1", "yo", "2026-10-30T10:01:00.000Z");
    row["to_twitch_id"] = json!(42);
    row["to_twitch_login"] = json!("Bob");
    let w = parse(&row, 1, None).unwrap();
    assert!(w.out);
    assert_eq!((&*w.who.id, &*w.who.login), ("42", "bob"));
    // hostile shapes
    for bad in [
      json!({}),
      json!({"from_twitch_id": "x", "content": "a"}),
      json!({"from_twitch_id": "5", "content": "  "}),
      json!({"from_twitch_id": "5"}),
      json!({"from_twitch_id": {"a": 1}, "content": "a"}),
    ] {
      assert!(parse(&bad, 0, None).is_none(), "{bad}");
    }
    let long = json!({"from_twitch_id": "5", "content": "x".repeat(9000)});
    assert_eq!(parse(&long, 0, None).unwrap().text.len(), MAX_TEXT);
    let evil = json!({"from_twitch_id": "5", "content": "a", "from_twitch_display": "x\u{7}y", "external_message_id": "z".repeat(100)});
    let w = parse(&evil, 0, None).unwrap();
    assert_eq!(&*w.who.name, "x y");
    assert!(w.ext.starts_with("db") || w.ext.is_empty());
  }

  #[test]
  fn timeline_dedupes_orders_and_caps() {
    let mut t = Timeline::default();
    let a = parse(&live("a1", "42", "one", "2026-10-30T10:00:00.000Z"), 1, None).unwrap();
    let b = parse(&live("a2", "42", "two", "2026-10-30T10:05:00.000Z"), 1, None).unwrap();
    assert!(t.add(b.clone()).is_some());
    // older history slots in front; a repeat is dropped
    assert!(t.add(a.clone()).is_some());
    assert!(t.add(a).is_none());
    assert_eq!(t.list.iter().map(|w| &*w.text).collect::<Vec<_>>(), ["one", "two"]);
    assert_eq!(t.last.as_ref().map(|w| &*w.login), Some("bob"));
    for i in 0..MAX + 20 {
      let mut w = b.clone();
      w.ext = format!("x{i}").into();
      w.ts += i as i64;
      t.add(w);
    }
    assert_eq!(t.len(), MAX);
  }

  #[test]
  fn a_send_and_its_history_copy_are_one_row() {
    let mut t = Timeline::default();
    let who = Who { via: Via::Twitch, id: "42".into(), login: "bob".into(), name: "Bob".into(), color: 0xff0000 };
    let key = t.push_out(who.clone(), "hello").key;
    assert!(matches!(t.get(key).unwrap().status, Status::Pending));
    t.settle(key, Status::Sent, None);
    let mut row = live("s9", "1", "hello", "2026-10-30T10:00:00.000Z");
    row["to_twitch_id"] = json!("42");
    let mut w = parse(&row, 1, Some(&who)).unwrap();
    w.ts = t.get(key).unwrap().ts + 500;
    assert!(t.add(w).is_none());
    assert_eq!(t.len(), 1);
    assert_eq!(&*t.get(key).unwrap().ext, "s9");
    // failing then retrying
    let k2 = t.push_out(who, "again").key;
    assert!(t.retry(k2).is_none());
    t.settle(k2, Status::Failed(Fail::Auth), None);
    assert!(t.retry(k2).is_some());
  }

  #[test]
  fn rows_read_like_the_ext() {
    let who = Who { via: Via::Twitch, id: "42".into(), login: "bob".into(), name: "Bob".into(), color: 0xff0000 };
    let mut w = Whisper { key: 7, ext: "".into(), ts: 1, out: true, who, text: "hi".into(), status: Status::Pending };
    let m = message(&w);
    assert_eq!((&*m.id, &*m.nick, &*m.body), ("w7", "you", "hi …"));
    assert!(m.user_id.is_empty(), "your own side is just \"you\": no paint");
    assert!(m.has(flag::BRAND) && !m.has(flag::ALERT));
    let x = m.x.unwrap();
    assert_eq!(&*x.tail[1].text, "Bob");
    w.status = Status::Failed(Fail::Relink);
    let m = message(&w);
    assert!(m.has(flag::ALERT) && m.body.contains("re-link"));
    w.out = false;
    w.status = Status::Sent;
    assert_eq!(&*message(&w).user_id, "42", "their name wears their paint");
    let m = message(&w);
    assert!(m.has(flag::HIGHLIGHTED));
    assert_eq!(&*m.nick, "Bob");
    assert_eq!(&*m.x.unwrap().tail[1].text, "you");
    assert_eq!(key_of("w7"), Some(7));
    assert_eq!(key_of("x7"), None);
  }

  #[test]
  fn slash_commands() {
    let w = Via::Twitch;
    assert_eq!(command("/w Bob hello there"), Some(Cmd::To(w, "bob".into(), Some("hello there".into()))));
    assert_eq!(command("/w @bob"), Some(Cmd::To(w, "bob".into(), None)));
    assert_eq!(command("/whisper bob  hi"), Some(Cmd::To(w, "bob".into(), Some("hi".into()))));
    assert_eq!(command("/dm @Bob yo"), Some(Cmd::To(Via::Heatsync, "bob".into(), Some("yo".into()))));
    assert_eq!(command("/DM bob"), Some(Cmd::To(Via::Heatsync, "bob".into(), None)));
    assert_eq!(command("/dm"), Some(Cmd::Usage));
    assert_eq!(command("/w"), Some(Cmd::Usage));
    assert_eq!(command("/w bad-name hi"), Some(Cmd::Usage));
    assert_eq!(command("/r thanks"), Some(Cmd::Reply("thanks".into())));
    assert_eq!(command("/r"), Some(Cmd::Usage));
    assert_eq!(command("/wow"), None);
    assert_eq!(command("hello"), None);
  }

  fn hs_who() -> Who {
    Who { via: Via::Heatsync, id: "77".into(), login: "amy".into(), name: "Amy".into(), color: 0x00ff00 }
  }

  #[test]
  fn parses_a_dm_push_and_history_rows() {
    let push = json!({"id": "9", "from_user_id": "77", "to_user_id": "1", "from_display_name": "Amy",
      "from_color": "#00ff00", "content": "hi\nthere", "created_at": "2026-10-30T10:00:00.000Z"});
    let w = parse_dm(&push, None).unwrap();
    assert!(!w.out && w.who.via == Via::Heatsync);
    assert_eq!((&*w.who.id, &*w.who.login, &*w.who.name, &*w.text, &*w.ext), ("77", "amy", "Amy", "hi there", "dm9"));
    // history: the row they sent is theirs, ours is out, both under the conversation's person
    let mine = json!({"id": 10, "from_user_id": "1", "to_user_id": "77", "content": "yo"});
    let w = parse_dm(&mine, Some(&hs_who())).unwrap();
    assert!(w.out && &*w.who.id == "77" && &*w.who.name == "Amy");
    assert!(!parse_dm(&push, Some(&hs_who())).unwrap().out);
    // namespaced ids pass, junk does not
    assert!(parse_dm(&json!({"from_user_id": "kick_123", "from_display_name": "K", "content": "a"}), None).is_some());
    let big = "9".repeat(80);
    for bad in [
      json!({}),
      json!({"from_user_id": "77", "content": "a"}),
      json!({"from_user_id": "77", "from_display_name": "A", "content": "  "}),
      json!({"from_user_id": "77", "from_display_name": "A"}),
      json!({"from_user_id": "a/b", "from_display_name": "A", "content": "a"}),
      json!({"from_user_id": big, "from_display_name": "A", "content": "a"}),
      json!({"from_user_id": {"a": 1}, "from_display_name": "A", "content": "a"}),
      json!({"from_user_id": "", "from_display_name": "A", "content": "a"}),
    ] {
      assert!(parse_dm(&bad, None).is_none(), "{bad}");
    }
    let long = json!({"from_user_id": "77", "from_display_name": "A", "content": "x".repeat(9000)});
    assert_eq!(parse_dm(&long, None).unwrap().text.len(), MAX_TEXT);
    let evil = json!({"from_user_id": "77", "from_display_name": "x\u{7}y", "content": "a", "id": {"z": 1}});
    let w = parse_dm(&evil, None).unwrap();
    assert_eq!((&*w.who.name, &*w.ext), ("x y", ""));
  }

  #[test]
  fn a_dm_and_a_whisper_with_the_same_id_are_different_people() {
    let mut t = Timeline::default();
    let mut tw = hs_who();
    tw.via = Via::Twitch;
    t.push_out(tw, "whisper");
    t.push_out(hs_who(), "dm");
    let w =
      parse_dm(&json!({"id": "1", "from_user_id": "77", "from_display_name": "Amy", "content": "dm"}), None).unwrap();
    assert!(t.add(w).is_some());
    assert_eq!(t.known(Via::Heatsync, "amy").map(|w| w.via), Some(Via::Heatsync));
    assert_eq!(t.known(Via::Twitch, "amy").map(|w| w.via), Some(Via::Twitch));
    assert!(t.known(Via::Heatsync, "bob").is_none());
    // /r points at the dm
    assert_eq!(t.last.as_ref().map(|w| w.via), Some(Via::Heatsync));
  }

  #[test]
  fn dm_rows_wear_the_heatsync_mark() {
    let w = Whisper {
      key: 3,
      ext: "".into(),
      ts: 1,
      out: true,
      who: hs_who(),
      text: "hi".into(),
      status: Status::Failed(Fail::Follow("amy".into())),
    };
    let m = message(&w);
    let x = m.x.unwrap();
    assert_eq!((&*x.head[0].text, x.head[0].ink), ("[H] ", theme::HEATSYNC));
    assert!(m.body.contains("/follow amy"));
  }

  #[test]
  fn a_history_from_an_older_login_is_dropped() {
    assert!(current(None, 3));
    assert!(current(Some(3), 3));
    assert!(!current(Some(2), 3));
  }
}
