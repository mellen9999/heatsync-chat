// the feed tab's state (ext social.js feedMessages / activeThread): newest-first posts, one open thread,
// and the rows they become. pure, so the rules (cap, dedupe, live replies, votes) are tested without a window
use crate::i18n::t;
use std::collections::{HashMap, VecDeque};

use super::engage::Marks;
use super::message::{HsEmote, Message, Platform};
use super::post::{self, Post, Reveal};
use super::reaction::{self, Change, Frame, Reaction};
use crate::net::pushed::Mod;
use crate::net::social::Page;

// the ext keeps 150 posts
pub const CAP: usize = 150;
// a feed older than this reloads when the tab is looked at (ext FEED_STALE_MS)
pub const STALE_MS: i64 = 120_000;

pub const MORE_ID: &str = "more";
// posts shown as lines in the chat tabs, kept so their >>id, tag and peek still resolve
const INLINE: usize = 200;

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
  // posts a vote is out for, with what your vote was before it (a refusal puts it back)
  voting: HashMap<Box<str>, bool>,
  // reaction presses out (post, emote), with the chip as it was: one request per chip at a time
  reacting: HashMap<(Box<str>, u64), Reaction>,
  // which posts you bookmarked
  pub marks: Marks,
  // held media the viewer asked to see, this session only (ext: never persisted)
  reveal: HashMap<Box<str>, Reveal>,
  // posts put into the chat tabs, newest last
  inline: VecDeque<Post>,
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
    self.posts.iter_mut().chain(self.inline.iter_mut()).filter(|p| &*p.id == id).for_each(&mut f);
    if let Some(t) = &mut self.thread {
      t.op.iter_mut().chain(t.replies.iter_mut()).filter(|p| &*p.id == id).for_each(&mut f);
    }
  }

  pub fn find(&self, id: &str) -> Option<&Post> {
    let t = self.thread.as_ref();
    let open = t.into_iter().flat_map(|t| t.op.iter().chain(t.replies.iter()));
    open.chain(self.posts.iter()).chain(self.inline.iter().rev()).find(|p| &*p.id == id)
  }

  // a post the chat tabs should show: true the first time it is heard (the server and your own send
  // can both report it)
  pub fn heard(&mut self, p: &Post) -> bool {
    if self.inline.iter().any(|x| x.id == p.id) {
      return false;
    }
    if self.inline.len() >= INLINE {
      self.inline.pop_front();
    }
    self.inline.push_back(p.clone());
    true
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

  // run `f` on the chips of every copy of a post
  fn each_chips(&mut self, id: &str, mut f: impl FnMut(&mut Vec<Reaction>)) {
    self.each_copy(id, |p| {
      let mut list = std::mem::take(&mut p.reactions).into_vec();
      f(&mut list);
      p.reactions = list.into();
    });
  }

  // a press on a chip: your reaction flips now, counted (ext feedReactionToggle). Some((wanted, chip)) = send
  // that. None = the chip is not here, or one is already out for it
  pub fn react_flip(&mut self, id: &str, emote: u64) -> Option<(bool, Reaction)> {
    let chip = self.find(id)?.reactions.iter().find(|r| r.id == emote)?.clone();
    if self.reacting.contains_key(&(id.into(), emote)) {
      return None;
    }
    let on = !chip.mine;
    self.react_set(id, &chip, on);
    self.reacting.insert((id.into(), emote), chip.clone());
    Some((on, chip))
  }

  // the answer is in (or never came): the chip as it was before the press
  pub fn react_end(&mut self, id: &str, emote: u64) -> Option<Reaction> {
    self.reacting.remove(&(id.into(), emote))
  }

  // your reaction on or off, on every copy of the post
  pub fn react_set(&mut self, id: &str, chip: &Reaction, on: bool) {
    self.each_chips(id, |l| {
      reaction::set_mine(l, chip, on);
    });
  }

  // the server said you had it already: the count it gave us already held you
  pub fn react_dup(&mut self, id: &str, emote: u64) {
    self.each_chips(id, |l| {
      if let Some(x) = l.iter_mut().find(|x| x.id == emote) {
        x.count = x.count.saturating_sub(1).max(1);
        x.mine = true;
      }
    });
  }

  // someone else's reaction, live, on every copy. true = a copy of the post is here
  pub fn react_frame(&mut self, f: &Frame) -> bool {
    let had = self.find(&f.post).is_some();
    self.each_chips(&f.post, |l| match &f.change {
      Change::Added(r) => reaction::add(l, r),
      Change::Removed(id) => reaction::sub(l, *id),
    });
    had
  }

  // a post's heat moved (a vote, or the site's heat beat), on every copy. true = a copy here changed
  pub fn heat_set(&mut self, id: &str, heat: u32) -> bool {
    let mut moved = false;
    self.each_copy(id, |x| {
      moved |= x.heat != heat;
      x.heat = heat;
    });
    moved
  }

  // a press: flip your vote now (ext feedVoteOptimistic). false = one is already out for this post,
  // or it is not here. one request per post at a time: two fast presses would toggle back
  pub fn begin_vote(&mut self, id: &str) -> bool {
    let Some(was) = self.find(id).map(|p| p.voted) else { return false };
    if self.voting.contains_key(id) {
      return false;
    }
    self.voting.insert(id.into(), was);
    self.voted(id, !was, None);
    true
  }

  // the answer is in (or never came): what your vote was before the press, for a rollback
  pub fn end_vote(&mut self, id: &str) -> Option<bool> {
    self.voting.remove(id)
  }

  // a post is gone (you deleted it): out of the list, the inline copies and the open thread. true = the
  // visible rows changed
  pub fn remove(&mut self, id: &str) -> bool {
    let parent = self.find(id).and_then(|p| p.reply_to.clone());
    let had = self.find(id).is_some();
    self.posts.retain(|p| &*p.id != id);
    self.inline.retain(|p| &*p.id != id);
    if let Some(t) = &mut self.thread {
      t.replies.retain(|p| &*p.id != id);
      if &*t.id == id {
        t.op = None;
        t.error = true;
      }
    }
    if let Some(parent) = parent {
      self.each_copy(&parent, |x| x.replies = x.replies.saturating_sub(1));
    }
    had
  }

  // your edit landed: the new words on every copy
  pub fn edited(&mut self, id: &str, text: &str) {
    self.each_copy(id, |x| {
      x.text = text.into();
      x.edited = true;
    });
  }

  // someone else's edit, pushed: the new words (and refs, when sent) on every copy. true = a copy changed
  pub fn edited_live(&mut self, id: &str, text: &str, refs: Option<&[HsEmote]>) -> bool {
    let mut hit = false;
    self.each_copy(id, |x| {
      hit |= &*x.text != text || !x.edited;
      x.text = text.into();
      x.edited = true;
      if let Some(r) = refs {
        x.refs = r.into();
      }
    });
    hit
  }

  // a thread root's numbers moved (`message-updated`): the reply count and heat, on every copy. true = changed
  pub fn meta(&mut self, id: &str, replies: Option<u32>, heat: Option<u32>) -> bool {
    let mut hit = false;
    self.each_copy(id, |x| {
      if let Some(r) = replies.filter(|r| *r != x.replies) {
        x.replies = r;
        hit = true;
      }
      if let Some(h) = heat.filter(|h| *h != x.heat) {
        x.heat = h;
        hit = true;
      }
    });
    hit
  }

  // moderation settled (`message:approved`) or flagged the post's media (`message:flagged`). true = changed
  pub fn moderated(&mut self, id: &str, m: &Mod) -> bool {
    let mut hit = false;
    self.each_copy(id, |x| match m {
      Mod::Approved => {
        hit |= x.pending;
        x.pending = false;
      }
      Mod::Flagged(cats) => {
        hit |= *x.cats != **cats;
        x.cats = cats.as_slice().into();
      }
    });
    hit
  }

  // a name's colour changed on heatsync (`profile:color`): their posts wear it, every copy. the ids of the
  // posts that changed
  pub fn recolor(&mut self, names: &[Box<str>], rgb: u32) -> Vec<Box<str>> {
    let mut ids: Vec<Box<str>> = Vec::new();
    let mut one = |x: &mut Post| {
      if !x.anon && x.color != rgb && names.iter().any(|n| n.eq_ignore_ascii_case(&x.user)) {
        x.color = rgb;
        if !ids.contains(&x.id) {
          ids.push(x.id.clone());
        }
      }
    };
    self.posts.iter_mut().chain(self.inline.iter_mut()).for_each(&mut one);
    if let Some(t) = &mut self.thread {
      t.op.iter_mut().chain(t.replies.iter_mut()).for_each(&mut one);
    }
    ids
  }

  // the thread an `/re` answers: the newest [OP] shown in chat, as its root post (an OP's own reply
  // answers the thread it is in)
  pub fn last_op(&self) -> Option<Box<str>> {
    let p = self.inline.iter().rev().find(|p| p.is_op || p.thread_op)?;
    Some(p.reply_to.clone().unwrap_or_else(|| p.id.clone()))
  }

  // the ids to ask the server about: every post on screen not known yet
  pub fn marks_wanted(&mut self) -> Vec<Box<str>> {
    let t = self.thread.as_ref();
    let open = t.into_iter().flat_map(|t| t.op.iter().chain(t.replies.iter()));
    let ids: Vec<Box<str>> = open.chain(self.posts.iter()).map(|p| p.id.clone()).collect();
    self.marks.ask_about(ids.iter().map(|i| &**i))
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

  pub fn reveal(&self, id: &str) -> Option<Reveal> {
    self.reveal.get(id).copied()
  }

  // show / hide / retry on a held post. Some(id) = its files were withheld: ask the server for them
  pub fn toggle_reveal(&mut self, id: &str) -> Option<Box<str>> {
    let p = self.find(id)?;
    let fetch = p.withheld && p.media.is_empty();
    match self.reveal(id) {
      Some(Reveal::Open) => {
        self.reveal.remove(id);
        None
      }
      Some(Reveal::Asking) => None,
      _ if fetch => {
        self.reveal.insert(id.into(), Reveal::Asking);
        Some(id.into())
      }
      _ => {
        self.reveal.insert(id.into(), Reveal::Open);
        None
      }
    }
  }

  // the withheld files came back (or didn't: 403 = not cleared for adult media)
  pub fn media_loaded(&mut self, id: &str, res: Result<Box<[post::Media]>, u16>) {
    let state = match res {
      Ok(m) if !m.is_empty() => {
        self.each_copy(id, |p| p.media = m.clone());
        Reveal::Open
      }
      Err(403) => Reveal::Denied,
      Err(451) => Reveal::Unavailable,
      _ => Reveal::Failed,
    };
    if self.reveal.len() >= CAP * 4 {
      self.reveal.clear();
    }
    self.reveal.insert(id.into(), state);
  }

  pub fn close(&mut self) {
    self.thread = None;
  }

  // the open thread's rows: the OP, then its replies, media drawn under the words
  pub fn thread_rows(&self, me: Option<&str>, signed_in: bool) -> Vec<Message> {
    let Some(th) = &self.thread else { return Vec::new() };
    let line = |t: &str| Message::system(0, Platform::Twitch, t);
    let msg = |p: &Post| {
      let mut m = post::message(p, me, signed_in);
      if let Some(x) = &mut m.x {
        x.pic = post::pic(p, true, self.reveal(&p.id)).map(Box::new);
      }
      m
    };
    let mut out: Vec<Message> = th.op.iter().map(msg).collect();
    if th.loading {
      out.push(line(t("chat1.feed.loading")));
    } else if th.error {
      out.push(line(t("chat1.feed.thread_gone")));
    } else if th.replies.is_empty() {
      out.push(line(t("chat1.feed.no_replies")));
    } else {
      out.extend(th.replies.iter().map(msg));
    }
    out
  }

  // the list, newest first
  pub fn rows(&self, me: Option<&str>, signed_in: bool) -> Vec<Message> {
    let line = |t: &str| Message::system(0, Platform::Twitch, t);
    let msg = |p: &Post| post::message(p, me, signed_in);
    let mut out = Vec::new();
    if let Some(cold) = self.hot {
      out.push(line(match (signed_in, cold) {
        (false, _) => t("chat1.feed.hot_signed_out"),
        (true, true) => t("chat1.feed.newest"),
        (true, false) => t("chat1.feed.hot_month"),
      }));
    }
    out.extend(self.posts.iter().map(msg));
    if self.more {
      let mut m = line(t("chat1.feed.more"));
      m.id = MORE_ID.into();
      out.push(m);
    }
    out
  }

  // what the tab says when there is nothing to list
  pub fn empty_text(&self) -> &'static str {
    match self.failed {
      Some(401) => t("chat1.feed.fail_login"),
      Some(_) => t("chat1.feed.fail"),
      None if self.loaded => t("chat1.feed.empty"),
      None => t("chat1.feed.loading"),
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
    f.thread_loaded("a", Ok((post("a", 100, None), vec![])));
    assert!(f.begin_vote("a"));
    assert!(f.posts[0].voted, "the press shows at once");
    assert!(!f.begin_vote("a"));
    f.voted("a", true, Some(40));
    assert_eq!(f.end_vote("a"), Some(false));
    assert!(f.posts[0].voted && f.posts[0].heat == 40);
    assert!(f.thread.as_ref().unwrap().op.as_ref().unwrap().voted);
    assert!(f.begin_vote("a"));
    assert!(!f.posts[0].voted, "a second press takes it back");
    assert!(!f.begin_vote("nope"), "a post that is not here has no vote");
  }

  #[test]
  fn a_refused_vote_goes_back_to_what_it_was() {
    let mut f = Feed::default();
    f.apply(page(1, vec![post("a", 100, None)], false), 1);
    assert!(f.begin_vote("a"));
    let prior = f.end_vote("a").unwrap();
    f.voted("a", prior, None);
    assert!(!f.posts[0].voted && f.posts[0].heat == 3);
    f.voted("a", true, None);
    assert!(f.begin_vote("a"));
    let prior = f.end_vote("a").unwrap();
    f.voted("a", prior, None);
    assert!(f.posts[0].voted, "a refused un-vote keeps the vote");
  }

  #[test]
  fn a_deleted_post_leaves_every_list() {
    let mut f = Feed::default();
    f.apply(page(1, vec![post("a", 100, None)], false), 1);
    f.open("a", None);
    f.thread_loaded("a", Ok((post("a", 100, None), vec![post("r", 200, Some("a"))])));
    f.posts[0].replies = 1;
    assert!(f.heard(&post("r", 200, Some("a"))));
    assert!(f.remove("r"));
    assert!(f.thread.as_ref().unwrap().replies.is_empty() && f.find("r").is_none());
    assert_eq!(f.posts[0].replies, 0);
    assert!(!f.remove("r"), "twice is nothing");
    assert!(f.remove("a"));
    assert!(f.posts.is_empty() && f.thread.as_ref().unwrap().op.is_none() && f.thread.as_ref().unwrap().error);
  }

  #[test]
  fn an_edit_changes_every_copy() {
    let mut f = Feed::default();
    f.apply(page(1, vec![post("a", 100, None)], false), 1);
    f.open("a", None);
    f.thread_loaded("a", Ok((post("a", 100, None), vec![])));
    f.edited("a", "new words");
    let t = f.thread.as_ref().unwrap().op.as_ref().unwrap();
    assert!(f.posts[0].edited && &*f.posts[0].text == "new words" && &*t.text == "new words");
    let m = f.rows(None, false);
    assert_eq!(&*m[0].body, "new words (edited)");
  }

  #[test]
  fn re_answers_the_newest_op_in_chat_by_its_root() {
    let mut f = Feed::default();
    assert_eq!(f.last_op(), None);
    assert!(f.heard(&post("a", 1, None)));
    assert!(f.heard(&post("b", 2, Some("a"))));
    assert_eq!(f.last_op().as_deref(), Some("a"), "a plain reply is not an [OP]");
    let mut own = post("c", 3, Some("a"));
    own.thread_op = true;
    assert!(f.heard(&own));
    assert_eq!(f.last_op().as_deref(), Some("a"), "the thread author's reply answers its thread");
    assert!(f.heard(&post("d", 4, None)));
    assert_eq!(f.last_op().as_deref(), Some("d"));
  }

  #[test]
  fn bookmarks_are_asked_for_what_is_on_screen_once() {
    let mut f = Feed::default();
    f.apply(page(1, vec![post("a", 100, None), post("b", 90, None)], false), 1);
    assert_eq!(f.marks_wanted(), vec![Box::<str>::from("a"), "b".into()]);
    assert!(f.marks_wanted().is_empty());
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
  fn show_asks_for_withheld_files_once_and_hide_folds_them() {
    let v = json!({"base36_id": "4a", "content": "x", "username": "a", "user_id": "1", "has_media": true,
      "media_held": ["nsfw"]});
    let mut f = Feed::default();
    f.apply(page(1, vec![post::parse(&v).unwrap()], false), 1);
    assert_eq!(f.toggle_reveal("4a").as_deref(), Some("4a"));
    assert_eq!(f.toggle_reveal("4a"), None, "a second press while asking asks nothing");
    f.media_loaded("4a", Err(403));
    assert_eq!(f.reveal("4a"), Some(Reveal::Denied));
    assert!(f.toggle_reveal("4a").is_some(), "a refused show can be pressed again");
    f.media_loaded("4a", Ok(post::media_of(&json!({"media_url": "/uploads/a.webp"}))));
    assert_eq!(f.reveal("4a"), Some(Reveal::Open));
    assert_eq!(f.posts[0].media.len(), 1);
    assert_eq!(f.toggle_reveal("4a"), None);
    assert_eq!(f.reveal("4a"), None, "hide");
    assert_eq!(f.toggle_reveal("4a"), None, "the files are here now: show needs no ask");
    assert_eq!(f.reveal("4a"), Some(Reveal::Open));
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
