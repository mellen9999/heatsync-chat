// the notifs tab: heatsync's own notifications (ext background.js `notification:new` + GET
// /api/notifications). the server only ever sends the kind, the post id and a snippet over the socket and
// a bare count over rest, so the list is what arrived this session; the count is the server's
// (`base`) plus what arrived since. kinds: mention, reply, feedback_fixed, system; an unknown one still
// shows, as a plain notice. every field is hostile: bounded, control bytes out, ids and paths checked
use std::collections::VecDeque;

use serde_json::Value;

use super::message::{Extra, Kind, Message, Platform, Seg, flag};
use crate::ui::theme;

// rows kept (the server's own list stops at its 200 newest unread)
pub const MAX_ITEMS: usize = 100;
const MAX_TEXT: usize = 300;
const MAX_COUNT: u32 = 200;
const MAX_PATH: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum What {
  Mention,
  // `op`: whose post it answers (true = your post, false = your reply, None = the server didn't say)
  Reply(Option<bool>),
  Fixed,
  System,
  Other,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
  pub seq: u64,
  pub at: i64,
  pub what: What,
  // the post it is about (a base36 id), opened in the feed
  pub post: Option<Box<str>>,
  // a page on heatsync.org, opened in the browser
  pub path: Option<Box<str>>,
  pub text: Box<str>,
}

fn clean(s: &str, max: usize) -> String {
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().to_string()
}

fn post_id(v: Option<&Value>) -> Option<Box<str>> {
  let s = v?.as_str()?;
  ((1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| s.into())
}

// `/cli`, `/feedback`: one rooted path of plain segments, nothing that could turn into another host
fn site_path(s: &str) -> Option<Box<str>> {
  let ok = s.len() <= MAX_PATH
    && s.starts_with('/')
    && !s.starts_with("//")
    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_-".contains(&b));
  ok.then(|| s.into())
}

// a `notification:new` frame
pub fn parse(v: &Value, at: i64) -> Option<Item> {
  // the envelope's own `type` shadows the notification's, so the kind is the only word that says what it is
  let kind = v.get("kind").and_then(Value::as_str).unwrap_or("");
  let content = clean(v.get("content").and_then(Value::as_str).unwrap_or(""), MAX_TEXT);
  let post = post_id(v.get("message_id"));
  let anon = v.get("from_username").and_then(Value::as_str) == Some("Anonymous");
  let who = if anon { "anonymous: " } else { "" };
  let (what, path, text) = match kind {
    "mention" => {
      let t = if content.is_empty() { "you were mentioned in a post".to_string() } else { content };
      (What::Mention, None, format!("{who}{t}"))
    }
    "reply" => {
      let op = v.get("parent_is_op").and_then(Value::as_bool);
      let to = match op {
        Some(true) => "replied to your post",
        Some(false) => "replied to your reply",
        None => "replied to you",
      };
      let t = if content.is_empty() { to.to_string() } else { format!("{to}: {content}") };
      (What::Reply(op), None, format!("{who}{t}"))
    }
    "feedback_fixed" => {
      let t =
        if content.is_empty() { "your report was fixed".into() } else { format!("your report was fixed: {content}") };
      (What::Fixed, Some("/feedback".into()), t)
    }
    "system" => {
      let path = v.get("url").and_then(Value::as_str).and_then(site_path);
      (What::System, path, content)
    }
    _ => (What::Other, None, content),
  };
  let post = post.filter(|_| matches!(what, What::Mention | What::Reply(_)));
  // nothing to say and nowhere to go is no notification
  if text.is_empty() && post.is_none() && path.is_none() {
    return None;
  }
  let text = if text.is_empty() { "new notification".into() } else { text };
  Some(Item { seq: 0, at, what, post, path, text: text.into() })
}

// GET /api/notifications: how many are unread (the server's own `count`, its 200 newest)
pub fn parse_count(v: &Value) -> Option<u32> {
  let n = v.get("count")?.as_u64()?;
  Some(n.min(MAX_COUNT as u64) as u32)
}

// the word at the head of a row, and its ink
fn label(w: What) -> (&'static str, u32) {
  match w {
    What::Mention => ("mention", theme::RED),
    What::Reply(_) => ("reply", theme::CYAN),
    What::Fixed => ("fixed", theme::GREEN),
    What::System => ("heatsync", theme::YELLOW),
    What::Other => ("notice", theme::WHITE),
  }
}

pub fn row_id(seq: u64) -> Box<str> {
  format!("n{seq}").into()
}

// the tab's row for an item
pub fn message(it: &Item) -> Message {
  let (word, ink) = label(it.what);
  let mut m = Message::system(it.at, Platform::Twitch, "");
  m.kind = Kind::Chat;
  m.id = row_id(it.seq);
  m.flags = flag::BRAND;
  m.nick = word.into();
  m.login = word.into();
  m.color = ink;
  m.body = it.text.clone();
  m.x = Some(Box::new(Extra {
    chan: "".into(),
    head: Box::new([Seg { text: "[H] ".into(), ink: theme::HEATSYNC, bold: true }]),
    ..Extra::default()
  }));
  m
}

#[derive(Default)]
pub struct Inbox {
  items: VecDeque<Item>,
  seq: u64,
  // unread the server counted that are not in `items` (older, or from before this window opened)
  base: u32,
  // when `base` was read; a look before that did not see them
  base_at: i64,
  // the newest moment you looked, here or on another client
  read_at: i64,
}

impl Inbox {
  pub fn push(&mut self, mut it: Item) -> &Item {
    self.seq += 1;
    it.seq = self.seq;
    if self.items.len() == MAX_ITEMS {
      self.items.pop_front();
    }
    self.items.push_back(it);
    self.items.back().expect("just pushed")
  }

  pub fn find(&self, row: &str) -> Option<&Item> {
    self.items.iter().find(|i| *row_id(i.seq) == *row)
  }

  fn fresh(&self) -> u32 {
    self.items.iter().filter(|i| i.at > self.read_at).count() as u32
  }

  // how many are unread: the server's older ones plus what arrived since you last looked
  pub fn unread(&self) -> u32 {
    (self.base + self.fresh()).min(MAX_COUNT)
  }

  // you looked at `at`, here or elsewhere. never moves back. the server's count only goes if it was
  // read before you looked (a look from earlier cannot have seen what it counted since)
  pub fn read(&mut self, at: i64) {
    self.read_at = self.read_at.max(at);
    if at >= self.base_at {
      self.base = 0;
    }
  }

  // the server's count, asked at `now`. what already sits in the list is part of it
  pub fn hydrate(&mut self, n: u32, now: i64) {
    self.base_at = now;
    if n == 0 {
      return self.read(now);
    }
    self.base = n.saturating_sub(self.fresh());
  }

  pub fn clear(&mut self) {
    *self = Self::default();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn each_kind_reads_in_its_own_words() {
    let m = parse(&json!({"kind": "mention", "message_id": "abc12", "content": "hi @you"}), 5).unwrap();
    assert_eq!((m.what, m.post.as_deref(), &*m.text), (What::Mention, Some("abc12"), "hi @you"));
    let r = parse(&json!({"kind": "reply", "message_id": "x1", "content": "lol", "parent_is_op": true}), 5).unwrap();
    assert_eq!((r.what, &*r.text), (What::Reply(Some(true)), "replied to your post: lol"));
    let r = parse(&json!({"kind": "reply", "message_id": "x1", "parent_is_op": false}), 5).unwrap();
    assert_eq!(&*r.text, "replied to your reply");
    let f = parse(&json!({"kind": "feedback_fixed", "content": "crash on start", "feedback_id": 7}), 5).unwrap();
    assert_eq!((f.what, f.path.as_deref()), (What::Fixed, Some("/feedback")));
    assert_eq!(&*f.text, "your report was fixed: crash on start");
    let s = parse(&json!({"kind": "system", "content": "a terminal logged in", "url": "/cli"}), 5).unwrap();
    assert_eq!((s.what, s.path.as_deref()), (What::System, Some("/cli")));
    let a = parse(&json!({"kind": "mention", "message_id": "q", "from_username": "Anonymous", "content": "yo"}), 5);
    assert_eq!(&*a.unwrap().text, "anonymous: yo");
  }

  #[test]
  fn an_unknown_kind_still_shows_and_an_empty_one_does_not() {
    let o = parse(&json!({"kind": "follow", "content": "bob followed you"}), 1).unwrap();
    assert_eq!((o.what, &*o.text), (What::Other, "bob followed you"));
    assert!(parse(&json!({"kind": "follow"}), 1).is_none());
    assert!(parse(&json!({}), 1).is_none());
    assert!(parse(&json!("x"), 1).is_none());
  }

  #[test]
  fn hostile_fields_are_bounded() {
    let big = "x".repeat(5000);
    let v = json!({"kind": "mention", "message_id": "../../etc", "content": format!("a\u{7}b\n{big}")});
    let m = parse(&v, 1).unwrap();
    assert!(m.post.is_none(), "not a base36 id");
    assert_eq!(m.text.chars().count(), MAX_TEXT);
    assert!(m.text.starts_with("a b "));
    // a post id on a kind that has no post is dropped
    assert!(parse(&json!({"kind": "system", "message_id": "abc", "content": "x"}), 1).unwrap().post.is_none());
    // paths: rooted plain segments only
    for bad in ["//evil.com", "https://evil.com", "/a?b=c", "cli", "/a b", &format!("/{big}")] {
      let s = parse(&json!({"kind": "system", "content": "x", "url": bad}), 1).unwrap();
      assert!(s.path.is_none(), "{bad}");
    }
    // wrong types
    assert!(parse(&json!({"kind": 5, "content": ["a"], "message_id": 7}), 1).is_none());
  }

  #[test]
  fn the_count_is_bounded() {
    assert_eq!(parse_count(&json!({"count": 3, "total": 2})), Some(3));
    assert_eq!(parse_count(&json!({"count": 9999999})), Some(MAX_COUNT));
    assert_eq!(parse_count(&json!({"count": -1})), None);
    assert_eq!(parse_count(&json!({"count": "3"})), None);
    assert_eq!(parse_count(&json!({})), None);
  }

  fn item(at: i64) -> Item {
    Item { seq: 0, at, what: What::Mention, post: None, path: None, text: "x".into() }
  }

  #[test]
  fn unread_counts_the_server_older_ones_plus_what_came_since() {
    let mut b = Inbox::default();
    b.push(item(10));
    b.push(item(20));
    assert_eq!(b.unread(), 2);
    // the server counts 5, two of them these: three older
    b.hydrate(5, 30);
    assert_eq!(b.unread(), 5);
    b.push(item(40));
    assert_eq!(b.unread(), 6);
    // a look before the count was taken did not see the older ones, but clears what it covers
    b.read(25);
    assert_eq!(b.unread(), 4);
    b.read(50);
    assert_eq!(b.unread(), 0);
    // never moves back
    b.read(1);
    b.push(item(60));
    assert_eq!(b.unread(), 1);
  }

  #[test]
  fn the_server_saying_none_clears_everything() {
    let mut b = Inbox::default();
    b.push(item(10));
    b.hydrate(0, 20);
    assert_eq!(b.unread(), 0);
    // the server's count is capped, and so is our list
    b.hydrate(9999, 30);
    assert_eq!(b.unread(), MAX_COUNT);
    for i in 0..MAX_ITEMS as i64 + 5 {
      b.push(item(100 + i));
    }
    assert_eq!(b.items.len(), MAX_ITEMS);
  }

  #[test]
  fn a_row_finds_its_item() {
    let mut b = Inbox::default();
    let s = b.push(item(1)).seq;
    assert!(b.find(&row_id(s)).is_some());
    assert!(b.find("n999").is_none());
    assert_eq!(&*message(b.find(&row_id(s)).unwrap()).id, &*row_id(s));
  }
}
