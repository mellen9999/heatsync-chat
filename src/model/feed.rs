// the feed tab's state (ext social.js feedMessages / activeThread): newest-first posts, one open thread,
// and the rows they become. pure, so the rules (cap, dedupe, live replies, votes) are tested without a window
use std::collections::HashSet;

use super::message::{Message, Platform};
use super::post::{self, Post};
use crate::net::social::Page;

// the ext keeps 150 posts
pub const CAP: usize = 150;
// a feed older than this reloads when the tab is looked at (ext FEED_STALE_MS)
pub const STALE_MS: i64 = 120_000;

pub const MORE_ID: &str = "more";

pub struct Thread {
  pub id: Box<str>,
  pub op: Option<Post>,
  pub replies: Vec<Post>,
  pub loading: bool,
  pub error: bool,
  // the reply it was opened for, picked out once the thread lands
  pub focus: Option<Box<str>>,
}

#[derive(Default)]
pub struct Feed {
  pub posts: Vec<Post>,
  pub page: u32,
  pub more: bool,
  pub loaded: bool,
  pub loading: bool,
  pub fetched: i64,
  // Some(cold) = the follows feed was empty and this is the hot (cold: newest) fallback
  pub hot: Option<bool>,
  // why the last load failed (http status; 0 = no answer)
  pub failed: Option<u16>,
  pub thread: Option<Thread>,
  voting: HashSet<Box<str>>,
}

impl Feed {
  // a page landed. page 1 replaces the list but keeps posts that arrived live and are newer than it
  pub fn apply(&mut self, p: Page, now: i64) {
    self.loading = false;
    self.failed = None;
    self.fetched = now;
    if p.page <= 1 {
      let newest = p.posts.iter().map(|x| x.ts).max().unwrap_or(0);
      let fresh: Vec<Post> =
        self.posts.drain(..).filter(|x| x.ts > newest && !p.posts.iter().any(|y| y.id == x.id)).collect();
      self.posts = fresh.into_iter().chain(p.posts).collect();
      self.page = 1;
      self.hot = p.hot.then_some(p.cold);
    } else {
      for x in p.posts {
        if !self.posts.iter().any(|y| y.id == x.id) {
          self.posts.push(x);
        }
      }
      self.page = p.page;
    }
    self.posts.truncate(CAP);
    // the hot fallback is ranked by heat: never page it. at the cap, more would be fetched and dropped
    self.more = p.more && !p.hot && self.posts.len() < CAP;
    self.loaded = true;
  }

  pub fn failed(&mut self, status: u16) {
    self.loading = false;
    self.failed = Some(status);
  }

  fn each_copy(&mut self, id: &str, mut f: impl FnMut(&mut Post)) {
    self.posts.iter_mut().filter(|p| &*p.id == id).for_each(&mut f);
    if let Some(t) = &mut self.thread {
      t.op.iter_mut().chain(t.replies.iter_mut()).filter(|p| &*p.id == id).for_each(&mut f);
    }
  }

  pub fn find(&self, id: &str) -> Option<&Post> {
    let t = self.thread.as_ref();
    t.into_iter().flat_map(|t| t.op.iter().chain(t.replies.iter())).chain(self.posts.iter()).find(|p| &*p.id == id)
  }

  // a post pushed by the server (or your own just posted). true = the visible rows changed
  pub fn live(&mut self, p: Post) -> bool {
    if p.anon || self.find(&p.id).is_some() {
      return false;
    }
    if let Some(parent) = p.reply_to.clone() {
      self.each_copy(&parent, |x| x.replies += 1);
      if let Some(t) = &mut self.thread
        && *t.id == *parent
      {
        t.replies.push(p);
      }
      return self.loaded;
    }
    if !self.loaded {
      return false;
    }
    self.posts.insert(0, p);
    self.posts.truncate(CAP);
    true
  }

  // your vote settled (or a heat number refreshed)
  pub fn voted(&mut self, id: &str, voted: bool, heat: Option<u32>) {
    self.each_copy(id, |x| {
      x.voted = voted;
      if let Some(h) = heat {
        x.heat = h;
      }
    });
  }

  // one request per post at a time: two fast presses would toggle back
  pub fn begin_vote(&mut self, id: &str) -> bool {
    self.voting.insert(id.into())
  }

  pub fn end_vote(&mut self, id: &str) {
    self.voting.remove(id);
  }

  pub fn open(&mut self, id: &str, focus: Option<&str>) {
    let op = self.posts.iter().find(|p| &*p.id == id).cloned();
    let focus = focus.map(Into::into);
    self.thread = Some(Thread { id: id.into(), op, replies: Vec::new(), loading: true, error: false, focus });
  }

  // the thread's fetch landed (ignored when another thread is open by now)
  pub fn thread_loaded(&mut self, id: &str, res: Result<(Post, Vec<Post>), u16>) -> bool {
    let Some(t) = self.thread.as_mut().filter(|t| *t.id == *id) else { return false };
    t.loading = false;
    match res {
      Ok((op, replies)) => {
        t.op = Some(op);
        t.replies = replies;
      }
      Err(_) => t.error = t.op.is_none(),
    }
    true
  }

  pub fn close(&mut self) {
    self.thread = None;
  }

  // everything the tab shows, top to bottom
  pub fn rows(&self, me: Option<&str>, signed_in: bool) -> Vec<Message> {
    let line = |t: &str| Message::system(0, Platform::Twitch, t);
    let msg = |p: &Post| post::message(p, me, signed_in);
    let mut out = Vec::new();
    if let Some(t) = &self.thread {
      out.extend(t.op.iter().map(msg));
      if t.loading {
        out.push(line("loading..."));
      } else if t.error {
        out.push(line("thread unavailable — post may have been removed"));
      } else if t.replies.is_empty() {
        out.push(line("no replies yet"));
      } else {
        out.extend(t.replies.iter().map(msg));
      }
      return out;
    }
    if let Some(cold) = self.hot {
      out.push(line(match (signed_in, cold) {
        (false, _) => "hot posts · log in at heatsync.org to follow people",
        (true, true) => "newest posts · follow people to fill your feed",
        (true, false) => "hot this month · follow people to fill your feed",
      }));
    }
    out.extend(self.posts.iter().map(msg));
    if self.more {
      let mut m = line("more");
      m.id = MORE_ID.into();
      out.push(m);
    }
    out
  }

  // what the tab says when there is nothing to list
  pub fn empty_text(&self) -> &'static str {
    match self.failed {
      Some(401) => "failed to load feed — log in at heatsync.org",
      Some(_) => "failed to load feed",
      None if self.loaded => "no posts yet",
      None => "loading...",
    }
  }

  pub fn stale(&self, now: i64) -> bool {
    !self.loading && (!self.loaded || now - self.fetched > STALE_MS)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn post(id: &str, ts: i64, reply_to: Option<&str>) -> Post {
    let mut v = json!({"base36_id": id, "content": "x", "username": "bob", "user_id": "1", "heat": 3});
    if let Some(r) = reply_to {
      v["reply_to"] = json!(r);
    }
    let mut p = post::parse(&v).unwrap();
    p.ts = ts;
    p
  }

  fn page(n: u32, posts: Vec<Post>, more: bool) -> Page {
    Page { page: n, posts, hot: false, cold: false, more }
  }

  #[test]
  fn pages_append_dedupe_and_cap() {
    let mut f = Feed::default();
    f.apply(page(1, (0..30).map(|i| post(&format!("a{i}"), 1000 - i, None)).collect(), true), 5);
    assert!(f.more && f.loaded);
    f.apply(page(2, (25..60).map(|i| post(&format!("a{i}"), 1000 - i, None)).collect(), true), 6);
    assert_eq!(f.posts.len(), 60);
    for n in 3..9 {
      f.apply(page(n, (0..30).map(|i| post(&format!("b{n}x{i}"), 1, None)).collect(), true), 7);
    }
    assert_eq!(f.posts.len(), CAP);
    assert!(!f.more, "no paging at the cap");
  }

  #[test]
  fn hot_fallback_never_pages() {
    let mut f = Feed::default();
    f.apply(Page { page: 1, posts: vec![post("a", 1, None)], hot: true, cold: true, more: true }, 1);
    assert_eq!(f.hot, Some(true));
    assert!(!f.more);
    let rows = f.rows(None, false);
    assert_eq!(&*rows[0].body, "hot posts · log in at heatsync.org to follow people");
  }

  #[test]
  fn refresh_keeps_newer_live_posts() {
    let mut f = Feed::default();
    f.apply(page(1, vec![post("a", 100, None)], false), 1);
    assert!(f.live(post("b", 200, None)));
    assert!(!f.live(post("b", 200, None)), "the same post twice is one post");
    f.apply(page(1, vec![post("a", 100, None)], false), 2);
    assert_eq!(f.posts.iter().map(|p| &*p.id).collect::<Vec<_>>(), ["b", "a"]);
  }

  #[test]
  fn replies_bump_the_parent_and_the_open_thread() {
    let mut f = Feed::default();
    f.apply(page(1, vec![post("a", 100, None)], false), 1);
    f.open("a", None);
    f.thread_loaded("a", Ok((post("a", 100, None), vec![])));
    assert!(f.live(post("r1", 300, Some("a"))));
    assert_eq!(f.posts[0].replies, 1);
    let t = f.thread.as_ref().unwrap();
    assert_eq!((t.op.as_ref().unwrap().replies, t.replies.len()), (1, 1));
    assert_eq!(f.posts.len(), 1, "a reply never becomes a top-level row");
  }

  #[test]
  fn votes_reach_every_copy_and_dont_stack() {
    let mut f = Feed::default();
    f.apply(page(1, vec![post("a", 100, None)], false), 1);
    f.open("a", None);
    assert!(f.begin_vote("a"));
    assert!(!f.begin_vote("a"));
    f.voted("a", true, Some(40));
    f.end_vote("a");
    assert!(f.begin_vote("a"));
    assert!(f.posts[0].voted && f.posts[0].heat == 40);
    assert!(f.thread.as_ref().unwrap().op.as_ref().unwrap().voted);
  }

  #[test]
  fn a_stale_thread_answer_is_ignored() {
    let mut f = Feed::default();
    f.open("a", None);
    f.open("b", None);
    assert!(!f.thread_loaded("a", Ok((post("a", 1, None), vec![]))));
    assert!(f.thread_loaded("b", Err(404)));
    assert!(f.thread.as_ref().unwrap().error);
  }

  #[test]
  fn empty_text_follows_the_failure() {
    let mut f = Feed::default();
    assert_eq!(f.empty_text(), "loading...");
    f.failed(401);
    assert_eq!(f.empty_text(), "failed to load feed — log in at heatsync.org");
    f.failed(500);
    assert_eq!(f.empty_text(), "failed to load feed");
  }
}
