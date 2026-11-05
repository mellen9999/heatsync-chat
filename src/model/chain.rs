// reply threads (ext main.js walkThreadMembers): the lines of one reply chain, found in a tab's own
// buffer. a line knows the one it answers (`parent`) and the first line of its chain (`root`) from
// twitch's reply tags or the kick relay's replyTo; a chain is every line of the buffer that is the root,
// names it, or answers a member. the ids are hostile: bounded and free of control bytes on the way in
use std::collections::HashSet;

use super::message::Message;

// lines kept each side of the one you opened it on (the ext's 128)
pub const SIDE: usize = 128;
// a message id never runs this long (twitch uses 36 chars, kick 36, youtube 60 or so)
pub const ID_MAX: usize = 128;
// hops up a chain looked for its root when a line names only its parent
const HOPS: usize = 64;

// an id off the wire: whole, short, printable. else none
pub fn clean_id(s: &str) -> Option<Box<str>> {
  (!s.is_empty() && s.len() <= ID_MAX && !s.chars().any(char::is_control)).then(|| s.into())
}

fn parent(m: &Message) -> &str {
  m.x.as_ref().map_or("", |x| &x.parent)
}

fn named_root(m: &Message) -> &str {
  m.x.as_ref().map_or("", |x| &x.root)
}

// the first line of the chain `m` is in: the root it names, else its parent's root (followed up the
// buffer), else the parent itself when that has scrolled away, else itself
pub fn root_of<'a>(m: &'a Message, buf: &'a [&'a Message]) -> &'a str {
  let mut cur = m;
  for _ in 0..HOPS {
    let r = named_root(cur);
    if !r.is_empty() {
      return r;
    }
    let p = parent(cur);
    if p.is_empty() {
      return &cur.id;
    }
    match buf.iter().find(|b| &*b.id == p) {
      Some(up) => cur = up,
      None => return p,
    }
  }
  &cur.id
}

// the line joins the chain `root` whose members so far carry `ids`
pub fn joins(m: &Message, root: &str, ids: &HashSet<Box<str>>) -> bool {
  let (p, r) = (parent(m), named_root(m));
  m.kind.said()
    && !m.id.is_empty()
    && (&*m.id == root || r == root || (!p.is_empty() && (p == root || ids.contains(p))))
}

pub struct Found {
  pub root: Box<str>,
  // oldest first: up to SIDE before the picked line, it, up to SIDE after
  pub lines: Vec<Message>,
  pub ids: HashSet<Box<str>>,
}

// the chain of the line with id `pick` among `buf` (oldest first)
pub fn find(buf: &[&Message], pick: &str) -> Option<Found> {
  let at = buf.iter().position(|m| &*m.id == pick)?;
  let root: Box<str> = root_of(buf[at], buf).into();
  let mut ids: HashSet<Box<str>> = HashSet::new();
  let mut all: Vec<(usize, &Message)> = Vec::new();
  for (i, m) in buf.iter().enumerate() {
    if i == at || joins(m, &root, &ids) {
      ids.insert(m.id.clone());
      all.push((i, m));
    }
  }
  let p = all.iter().position(|(i, _)| *i == at)?;
  let from = p.saturating_sub(SIDE);
  let to = (p + 1 + SIDE).min(all.len());
  let lines: Vec<Message> = all[from..to].iter().map(|(_, m)| (*m).clone()).collect();
  Some(Found { root, lines, ids })
}

// copy thread: one `name: text` per line, oldest first. none unless it has 2+ lines
pub fn copy_text(lines: &[Message]) -> Option<String> {
  let flat = |s: &str| s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>();
  (lines.len() >= 2)
    .then(|| lines.iter().map(|m| format!("{}: {}", m.nick, flat(&m.body))).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::{Kind, Platform};

  fn line(id: &str, parent: &str, root: &str) -> Message {
    let mut m = Message::system(0, Platform::Twitch, "");
    m.kind = Kind::Chat;
    m.id = id.into();
    m.nick = format!("n{id}").into();
    m.body = format!("t{id}").into();
    if !parent.is_empty() || !root.is_empty() {
      let x = m.extra();
      x.parent = parent.into();
      x.root = root.into();
    }
    m
  }

  fn refs(v: &[Message]) -> Vec<&Message> {
    v.iter().collect()
  }

  fn ids(f: &Found) -> Vec<&str> {
    f.lines.iter().map(|m| &*m.id).collect()
  }

  #[test]
  fn ids_are_bounded_and_printable() {
    assert_eq!(clean_id("abc-1").as_deref(), Some("abc-1"));
    assert!(clean_id("").is_none());
    assert!(clean_id(&"a".repeat(ID_MAX + 1)).is_none());
    assert!(clean_id(&"a".repeat(ID_MAX)).is_some());
    assert!(clean_id("a\nb").is_none() && clean_id("a\u{7}").is_none());
  }

  #[test]
  fn a_chain_is_the_root_and_everything_answering_it() {
    let v = [line("a", "", ""), line("x", "", ""), line("b", "a", "a"), line("c", "b", "a"), line("y", "x", "x")];
    let f = find(&refs(&v), "b").unwrap();
    assert_eq!(&*f.root, "a");
    assert_eq!(ids(&f), ["a", "b", "c"]);
    // the same chain from any of its lines, and from the root with replies under it
    assert_eq!(ids(&find(&refs(&v), "a").unwrap()), ["a", "b", "c"]);
    assert_eq!(ids(&find(&refs(&v), "c").unwrap()), ["a", "b", "c"]);
  }

  #[test]
  fn a_line_naming_only_its_parent_follows_it_up_to_the_root() {
    // our own echo carries a parent and no root
    let v = [line("a", "", ""), line("b", "a", "a"), line("me", "b", "")];
    let f = find(&refs(&v), "me").unwrap();
    assert_eq!((&*f.root, ids(&f)), ("a", vec!["a", "b", "me"]));
  }

  #[test]
  fn a_parent_that_scrolled_away_roots_the_chain() {
    let v = [line("b", "gone", "gone"), line("c", "b", "gone")];
    let f = find(&refs(&v), "c").unwrap();
    assert_eq!((&*f.root, ids(&f)), ("gone", vec!["b", "c"]));
    let lone = [line("z", "", "")];
    assert_eq!(ids(&find(&refs(&lone), "z").unwrap()), ["z"]);
    assert!(find(&refs(&lone), "nope").is_none());
  }

  #[test]
  fn each_side_is_capped_and_a_cycle_ends() {
    let mut v = vec![line("r", "", "")];
    for i in 0..300 {
      v.push(line(&format!("m{i}"), "r", "r"));
    }
    let f = find(&refs(&v), "m150").unwrap();
    assert_eq!(f.lines.len(), SIDE * 2 + 1);
    assert_eq!(&*f.lines[SIDE].id, "m150");
    assert_eq!(&*f.lines[0].id, "m22", "oldest first, 128 before");
    // two lines naming each other: no hang
    let c = [line("p", "q", ""), line("q", "p", "")];
    assert!(find(&refs(&c), "p").is_some());
  }

  #[test]
  fn live_lines_join_by_root_or_by_answering_a_member() {
    let ids: HashSet<Box<str>> = ["a".into(), "b".into()].into();
    assert!(joins(&line("c", "b", "a"), "a", &ids));
    assert!(joins(&line("d", "b", ""), "a", &ids));
    assert!(!joins(&line("e", "zz", "zz"), "a", &ids));
    assert!(!joins(&line("f", "", ""), "a", &ids));
  }

  #[test]
  fn copy_thread_is_oldest_first_one_line_each_and_needs_two() {
    let v = [line("a", "", ""), line("b", "a", "a")];
    assert_eq!(copy_text(&v).as_deref(), Some("na: ta\nnb: tb"));
    assert!(copy_text(&v[..1]).is_none());
  }
}
