// heatsync notifications as rows of the mentions tab (ext mentions.js hsNotifIn / hsNotifText): a reply to
// your post, an @ in a post, a fixed report. a `notification:new` frame is every field hostile: bounded,
// control bytes out, the post id checked. any other kind is not a row. a row carries what it needs in its
// id (`hsn-<kind>-<ms>-<post>`), so it survives the mentions file (model::mention_file) and enter finds
// its post without a side table
use crate::i18n::{f, t};
use serde_json::Value;

use super::message::{Extra, Kind, Message, Platform, Seg};
use crate::ui::theme;

const MAX_TEXT: usize = 300;
const PREFIX: &str = "hsn-";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum What {
  Mention,
  Reply,
  Fixed,
}

impl What {
  fn key(self) -> char {
    match self {
      What::Mention => 'm',
      What::Reply => 'r',
      What::Fixed => 'f',
    }
  }

  fn from_key(c: &str) -> Option<Self> {
    Some(match c {
      "m" => What::Mention,
      "r" => What::Reply,
      "f" => What::Fixed,
      _ => return None,
    })
  }

  // the row's edge, as the ext colors them: a mention red, a reply cyan, a fixed report green
  fn ink(self) -> u32 {
    match self {
      What::Mention => theme::RED,
      What::Reply => theme::CYAN,
      What::Fixed => theme::GREEN,
    }
  }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
  pub at: i64,
  pub what: What,
  // the post it is about (a base36 id), opened in the feed
  pub post: Option<Box<str>>,
  pub text: Box<str>,
}

fn clean(s: &str, max: usize) -> String {
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().to_string()
}

fn valid_post(s: &str) -> bool {
  (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

// "anonymous: …" when the sender hid their name
fn who(anon: bool, text: &str) -> String {
  if anon { f("chat1.inbox.anon", &[("text", text)]) } else { text.to_string() }
}

// a `notification:new` frame; None = not a kind that becomes a row
pub fn parse(v: &Value, at: i64) -> Option<Item> {
  // the envelope's own `type` shadows the notification's, so the kind is the only word that says what it is
  let kind = v.get("kind").and_then(Value::as_str)?;
  let content = clean(v.get("content").and_then(Value::as_str).unwrap_or(""), MAX_TEXT);
  let anon = v.get("from_username").and_then(Value::as_str) == Some("Anonymous");
  let post = v.get("message_id").and_then(Value::as_str).filter(|s| valid_post(s)).map(Box::<str>::from);
  let (what, text, post) = match kind {
    "mention" => {
      let t = if content.is_empty() { t("chat1.inbox.mentioned") } else { &content };
      (What::Mention, who(anon, t), post)
    }
    "reply" => {
      let to = match v.get("parent_is_op").and_then(Value::as_bool) {
        Some(true) => t("chat1.inbox.reply_post"),
        Some(false) => t("chat1.inbox.reply_reply"),
        None => t("chat1.inbox.reply_you"),
      };
      let t = if content.is_empty() {
        to.to_string()
      } else {
        f("chat1.inbox.with_content", &[("lead", to), ("content", &content)])
      };
      (What::Reply, who(anon, &t), post)
    }
    // the id it carries is a report's, not a post's
    "feedback_fixed" => (What::Fixed, if content.is_empty() { t("chat1.inbox.fixed").into() } else { content }, None),
    _ => return None,
  };
  Some(Item { at, what, post, text: text.into() })
}

fn id(what: What, at: i64, post: &str) -> String {
  format!("{PREFIX}{}-{at}-{post}", what.key())
}

// (kind, ms, post) out of a row's id; the post may be empty
fn split(id: &str) -> Option<(What, i64, &str)> {
  let mut p = id.strip_prefix(PREFIX)?.splitn(3, '-');
  let (what, at, post) = (What::from_key(p.next()?)?, p.next()?.parse().ok()?, p.next()?);
  (post.is_empty() || valid_post(post)).then_some((what, at, post))
}

fn build(what: What, at: i64, post: &str, text: &str) -> Message {
  let mut m = Message::system(at, Platform::Twitch, "");
  m.kind = Kind::Chat;
  m.id = id(what, at, post).into();
  m.nick = "heatsync".into();
  m.login = "heatsync".into();
  m.color = theme::HEATSYNC;
  m.body = text.into();
  m.x = Some(Box::new(Extra {
    head: Box::new([Seg { text: "[H] ".into(), ink: theme::HEATSYNC, bold: true }]),
    edge: what.ink(),
    ..Extra::default()
  }));
  m
}

// the mentions tab's row for a notification
pub fn message(it: &Item) -> Message {
  build(it.what, it.at, it.post.as_deref().unwrap_or(""), &it.text)
}

// a row read back from the mentions file: its id says what it was, `text` is what it said (bounded here)
pub fn restore(id: &str, text: &str) -> Option<Message> {
  let (what, at, post) = split(id)?;
  Some(build(what, at, post, &clean(text, MAX_TEXT)))
}

// does an id look like one of these rows' (a file line is held to the rest in `restore`)
pub fn is_id(id: &str) -> bool {
  id.starts_with(PREFIX)
}

// is this row one of these
pub fn is_row(m: &Message) -> bool {
  split(&m.id).is_some()
}

// the post a row opens in the feed, if it has one
pub fn post_of(m: &Message) -> Option<&str> {
  split(&m.id).map(|s| s.2).filter(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn each_kind_reads_in_its_own_words() {
    let m = parse(&json!({"kind": "mention", "message_id": "abc12", "content": "hi @you"}), 5).unwrap();
    assert_eq!((m.what, m.post.as_deref(), &*m.text), (What::Mention, Some("abc12"), "hi @you"));
    let m = parse(&json!({"kind": "mention", "message_id": "abc12"}), 5).unwrap();
    assert_eq!(&*m.text, "you were mentioned in a post");
    let r = parse(&json!({"kind": "reply", "message_id": "x1", "content": "lol", "parent_is_op": true}), 5).unwrap();
    assert_eq!((r.what, &*r.text), (What::Reply, "replied to your post: lol"));
    let r = parse(&json!({"kind": "reply", "message_id": "x1", "parent_is_op": false}), 5).unwrap();
    assert_eq!(&*r.text, "replied to your reply");
    let r = parse(&json!({"kind": "reply", "message_id": "x1"}), 5).unwrap();
    assert_eq!(&*r.text, "replied to you");
    let f = parse(&json!({"kind": "feedback_fixed", "content": "crash on start", "feedback_id": 7}), 5).unwrap();
    assert_eq!((f.what, f.post, &*f.text), (What::Fixed, None, "crash on start"));
    let f = parse(&json!({"kind": "feedback_fixed"}), 5).unwrap();
    assert_eq!(&*f.text, "your report was fixed");
    let a = parse(&json!({"kind": "mention", "message_id": "q", "from_username": "Anonymous", "content": "yo"}), 5);
    assert_eq!(&*a.unwrap().text, "anonymous: yo");
    let a = parse(&json!({"kind": "reply", "from_username": "Anonymous"}), 5);
    assert_eq!(&*a.unwrap().text, "anonymous: replied to you");
  }

  #[test]
  fn any_other_kind_is_not_a_row() {
    for kind in ["system", "follow", "", "Mention"] {
      assert!(parse(&json!({"kind": kind, "content": "x", "message_id": "ab"}), 1).is_none(), "{kind}");
    }
    assert!(parse(&json!({}), 1).is_none());
    assert!(parse(&json!("x"), 1).is_none());
    assert!(parse(&json!({"kind": 5, "content": ["a"], "message_id": 7}), 1).is_none());
  }

  #[test]
  fn hostile_fields_are_bounded() {
    let big = "x".repeat(5000);
    let v = json!({"kind": "mention", "message_id": "../../etc", "content": format!("a\u{7}b\n{big}")});
    let m = parse(&v, 1).unwrap();
    assert!(m.post.is_none(), "not a base36 id");
    assert_eq!(m.text.chars().count(), MAX_TEXT);
    assert!(m.text.starts_with("a b "));
    // a report's id is never a post
    assert!(parse(&json!({"kind": "feedback_fixed", "message_id": "abc"}), 1).unwrap().post.is_none());
  }

  #[test]
  fn a_row_carries_its_post_and_comes_back_from_the_file() {
    let it = parse(&json!({"kind": "reply", "message_id": "ab12", "content": "yo"}), 1_700_000_000_000).unwrap();
    let m = message(&it);
    assert_eq!((&*m.nick, m.ts, post_of(&m), is_row(&m)), ("heatsync", 1_700_000_000_000, Some("ab12"), true));
    let back = restore(&m.id, &m.body).unwrap();
    assert_eq!((&*back.id, &*back.body, post_of(&back)), (&*m.id, &*m.body, Some("ab12")));
    assert_eq!(back.x.as_ref().map(|x| x.edge), Some(theme::CYAN));
    // no post: a row, nothing to open
    let none = message(&parse(&json!({"kind": "feedback_fixed"}), 7).unwrap());
    assert!(is_row(&none) && post_of(&none).is_none());
    // an id that is not ours, or hostile, is no row
    for bad in ["", "hsn-", "hsn-x-1-ab", "hsn-m-soon-ab", "hsn-m-1-../x", "hsn-m-1-123456789", "abc"] {
      assert!(restore(bad, "x").is_none() && split(bad).is_none(), "{bad}");
    }
  }
}
