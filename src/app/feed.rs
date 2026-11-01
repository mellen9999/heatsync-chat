// the feed tab (ext social.js + feed-engage.js): heatsync posts newest first, read from the top.
// enter (or a click on its >>id) opens a thread, u upvotes, r replies, typing posts (a reply while a thread is open), esc or h goes back.
// logged out it still reads: the hot posts, with the nudge to log in
use super::App;
use crate::model::feed::{Feed, MORE_ID};
use crate::model::post::Post;
use crate::model::tabs::Kind;
use crate::net::Cmd;
use crate::net::social::{self, Page};
use crate::util::time::now_ms;

const LOGIN_FIRST: &str = "log in at heatsync.org first";
const TRY_AGAIN: &str = "that didn’t work — try again";

impl App {
  fn feed_tab(&self) -> Option<usize> {
    self.tab_of(Kind::Feed)
  }

  fn my_name(&self) -> Option<Box<str>> {
    self.me.as_ref().map(|m| m.username.clone())
  }

  // redraw the tab from the state, keeping scroll and selection
  pub(super) fn feed_show(&mut self) {
    let Some(i) = self.feed_tab() else { return };
    let me = self.my_name();
    let rows = self.feed.rows(me.as_deref(), self.me.is_some());
    let none = rows.is_empty();
    let v = &mut self.tabs[i].view;
    v.set_empty(self.feed.empty_text());
    if none {
      v.clear();
    } else {
      v.reload(rows);
    }
    if i == self.active {
      self.input_dirty = true;
      self.wake();
    }
  }

  fn feed_load(&mut self, page: u32) {
    if self.feed.loading {
      return;
    }
    self.feed.loading = true;
    self.net.send(Cmd::Social(social::Cmd::Feed { page }));
  }

  // the tab came into view: first load, or a refresh once it is two minutes old
  pub(super) fn feed_entered(&mut self) {
    if self.feed.thread.is_none() && self.feed.stale(now_ms()) {
      self.feed_load(1);
    }
  }

  // a different login reads a different (following) feed
  pub(super) fn feed_reset(&mut self) {
    self.feed = Feed::default();
    self.feed_show();
    if self.kind() == Kind::Feed {
      self.feed_load(1);
    }
  }

  pub(super) fn on_feed(&mut self, p: Page) {
    self.feed.apply(p, now_ms());
    self.feed_show();
  }

  pub(super) fn on_feed_failed(&mut self, status: u16) {
    self.feed.failed(status);
    self.feed_show();
  }

  pub(super) fn on_feed_live(&mut self, p: Post) {
    let mine = self.my_name().is_some_and(|n| n.eq_ignore_ascii_case(&p.user));
    let top = p.is_op;
    if !self.feed.live(p) {
      return;
    }
    self.feed_show();
    if top && !mine {
      self.unseen_note(crate::model::unseen::Surface::Live);
    }
  }

  pub(super) fn on_thread(&mut self, id: Box<str>, res: Result<(Post, Vec<Post>), u16>) {
    if !self.feed.thread_loaded(&id, res) {
      return;
    }
    self.feed_show();
    let Some(focus) = self.feed.thread.as_mut().and_then(|t| t.focus.take()) else { return };
    let (Some(i), h) = (self.feed_tab(), self.chat_area().h) else { return };
    let res = &mut self.res;
    self.tabs[i].view.select_id(&focus, res, h);
    self.wake();
  }

  pub(super) fn on_voted(&mut self, id: Box<str>, res: Result<(bool, Option<u32>), u16>) {
    self.feed.end_vote(&id);
    match res {
      Ok((voted, heat)) => {
        self.feed.voted(&id, voted, heat);
        self.feed_show();
      }
      Err(401) => self.say(LOGIN_FIRST),
      Err(_) => self.say(TRY_AGAIN),
    }
  }

  pub(super) fn on_posted(&mut self, res: Result<Post, u16>) {
    let text = self.pending_post.take().unwrap_or_default();
    match res {
      Ok(p) => {
        self.say("posted to feed");
        self.feed.live(p);
        self.feed_show();
      }
      Err(status) => {
        // the words stay in the box so a failed post is never lost
        if self.line.text.is_empty() && !text.is_empty() {
          self.line.set_text(text);
        }
        self.input_dirty = true;
        self.say(match status {
          401 => "log in first",
          429 => "slow down",
          409 => "duplicate message",
          _ => "failed to post",
        });
      }
    }
  }

  // the thread a post belongs to: itself, or its parent when it is a reply
  fn feed_root(&self, id: &str) -> Box<str> {
    self.feed.find(id).and_then(|p| p.reply_to.clone()).unwrap_or_else(|| id.into())
  }

  fn selected_post_id(&self) -> Option<Box<str>> {
    let t = self.tabs.get(self.active).filter(|t| t.kind == Kind::Feed)?;
    let id = t.view.selected()?.id.clone();
    (!id.is_empty()).then_some(id)
  }

  // a reply opens its parent's thread, picked out once it lands (ext openThread's highlightId)
  fn feed_open(&mut self, id: &str) {
    let root = self.feed_root(id);
    self.feed.open(&root, (*root != *id).then_some(id));
    self.net.send(Cmd::Social(social::Cmd::Thread { id: root }));
    self.feed_show();
    let res = &mut self.res;
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.deselect();
      t.view.scroll_to_top(res);
    }
  }

  // enter on a row: a post opens its thread, "more" loads the next page
  pub(super) fn feed_activate(&mut self) {
    let Some(id) = self.selected_post_id() else { return };
    if &*id == MORE_ID {
      return self.feed_load(self.feed.page + 1);
    }
    if self.feed.thread.is_none() {
      self.feed_open(&id);
    }
    self.wake();
  }

  // r: read the thread and answer it
  pub(super) fn feed_reply(&mut self) {
    let Some(id) = self.selected_post_id().filter(|i| &**i != MORE_ID) else { return };
    if self.feed.thread.is_none() {
      self.feed_open(&id);
    }
    self.enter_insert();
    self.wake();
  }

  // a click on a post's own >>id opens its thread; on the open thread's own, back to the list
  // (ext toggleThread). true = the click was spent
  pub(super) fn thread_press(&mut self) -> bool {
    if self.kind() != Kind::Feed {
      return false;
    }
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    let res = &mut self.res;
    let Some(id) = self.tabs.get_mut(self.active).and_then(|t| t.view.thread_at(chat, x, y, res)) else {
      return false;
    };
    if self.feed.thread.as_ref().is_some_and(|t| t.id == id) {
      self.feed_back();
    } else {
      self.feed_open(&id);
    }
    self.wake();
    true
  }

  // a click on a row's ▲ votes on that row. true = the click was a vote press (spent either way)
  pub(super) fn vote_press(&mut self) -> bool {
    if self.kind() != Kind::Feed {
      return false;
    }
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    let res = &mut self.res;
    let Some(t) = self.tabs.get_mut(self.active) else { return false };
    if t.view.vote_at(chat, x, y, res).is_none() {
      return false;
    }
    t.view.select_at(chat, y, res);
    self.feed_vote();
    self.wake();
    true
  }

  pub(super) fn feed_vote(&mut self) {
    let Some(id) = self.selected_post_id().filter(|i| &**i != MORE_ID) else { return };
    if self.me.is_none() {
      return self.say(LOGIN_FIRST);
    }
    let own = self.feed.find(&id).zip(self.my_name()).is_some_and(|(p, n)| n.eq_ignore_ascii_case(&p.user));
    if own || !self.feed.begin_vote(&id) {
      return;
    }
    self.net.send(Cmd::Social(social::Cmd::Vote { id }));
  }

  // esc / h: out of the thread, back to the list. false = nothing to leave
  pub(super) fn feed_back(&mut self) -> bool {
    if self.kind() != Kind::Feed || self.feed.thread.is_none() {
      return false;
    }
    self.feed.close();
    self.feed_show();
    if let Some(v) = self.view() {
      v.deselect();
    }
    self.wake();
    true
  }

  // a typed line in the feed tab: a new post, or a reply to the open thread
  pub(super) fn feed_post(&mut self) {
    let Some(text) = self.line.take() else { return };
    if self.me.is_none() {
      self.line.set_text(text);
      return self.say(LOGIN_FIRST);
    }
    let reply_to = self.feed.thread.as_ref().map(|t| t.id.clone());
    self.pending_post = Some(text.clone());
    self.net.send(Cmd::Social(social::Cmd::Post { text, reply_to }));
  }

  pub(super) fn feed_hint(&self) -> String {
    match (&self.feed.thread, &self.me) {
      (_, None) => "ctrl-l log in to post".into(),
      (Some(t), _) => format!("type to reply to >>{}", crate::model::post::short(&t.id)),
      (None, _) => "post to heatsync...".into(),
    }
  }
}
