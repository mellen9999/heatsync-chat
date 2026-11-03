// the feed tab (ext social.js + feed-engage.js): heatsync posts newest first, read from the top.
// enter (or a click on its >>id) opens a thread, u upvotes, r replies, typing posts (a reply while a thread is open), esc or h goes back.
// logged out it still reads: the hot posts, with the nudge to log in
use super::App;
use crate::model::engage;
use crate::model::feed::{Feed, MORE_ID};
use crate::model::message::Message;
use crate::model::post::{self, Post};
use crate::model::tabs::Kind;
use crate::net::Cmd;
use crate::net::social::{self, Page};
use crate::ui::chat_view::ChatView;
use crate::ui::theme;
use crate::util::time::now_ms;

// a thread's rows at most (the OP and its replies)
const THREAD_ROWS: usize = 1000;

pub(super) const LOGIN_FIRST: &str = "log in at heatsync.org first";
pub(super) const TRY_AGAIN: &str = "that didn’t work — try again";

impl App {
  fn feed_tab(&self) -> Option<usize> {
    self.tab_of(Kind::Feed)
  }

  pub(super) fn my_name(&self) -> Option<Box<str>> {
    self.me.as_ref().map(|m| m.username.clone())
  }

  // the tab something (a thread, the chat logs) has taken over, if one is open
  pub(super) fn takeover_tab(&self) -> Option<usize> {
    self.tabs.iter().position(|t| t.under.is_some())
  }

  // the tab a thread has taken over (the one it was opened from), if one is open
  pub(super) fn thread_tab(&self) -> Option<usize> {
    self.takeover_tab().filter(|_| self.logs.is_none())
  }

  // the active tab reads the feed: the feed tab, or any tab a thread has taken over
  pub(super) fn feedish(&self) -> bool {
    self.kind() == Kind::Feed || (self.logs.is_none() && self.tabs.get(self.active).is_some_and(|t| t.under.is_some()))
  }

  // redraw the list and the open thread from the state, keeping scroll and selection
  pub(super) fn feed_show(&mut self) {
    let me = self.my_name();
    let signed_in = self.me.is_some();
    if let Some(i) = self.feed_tab() {
      let rows = self.feed.rows(me.as_deref(), signed_in);
      let v = self.tabs[i].chat();
      v.set_empty(self.feed.empty_text());
      if rows.is_empty() {
        v.clear();
      } else {
        v.reload(rows);
      }
    }
    if let Some(i) = self.thread_tab() {
      let rows = self.feed.thread_rows(me.as_deref(), signed_in);
      self.tabs[i].view.reload(rows);
    }
    if self.feedish() || self.kind() == Kind::Feed {
      self.input_dirty = true;
    }
    self.wake();
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
    self.takeover_close();
    self.edit_end();
    self.feed = Feed::default();
    self.feed_show();
    if self.kind() == Kind::Feed {
      self.feed_load(1);
    }
  }

  pub(super) fn on_feed(&mut self, p: Page) {
    self.feed.apply(p, now_ms());
    self.feed_show();
    self.marks_load();
  }

  pub(super) fn on_feed_failed(&mut self, status: u16) {
    self.feed.failed(status);
    self.feed_show();
  }

  // a feed post as a chat line: its kind's edge (ext injectInlineNotif), no ▲
  pub(super) fn inline_line(&self, p: &Post) -> Message {
    let edge = if p.thread_op {
      theme::MAGENTA
    } else if p.is_op {
      theme::RED
    } else {
      theme::CYAN
    };
    let mut m = post::message(p, self.my_name().as_deref(), false);
    if let Some(x) = &mut m.x {
      x.edge = edge;
    }
    m
  }

  // a feed post as a line in every chat tab (ext injectInlineNotif): the >>id opens its thread right
  // there. `force` = your own post, shown whatever the switches say (a receipt, not news)
  fn feed_inline(&mut self, p: &Post, force: bool) {
    let on = if p.thread_op {
      self.set.inline_mop
    } else if p.is_op {
      self.set.inline_op
    } else {
      self.set.inline_re
    };
    if p.anon || !(on || force) || !self.feed.heard(p) {
      return;
    }
    let m = self.inline_line(p);
    for i in 0..self.tabs.len() {
      if self.tabs[i].kind == Kind::Chat && !self.tabs[i].sources.is_empty() {
        self.tabs[i].chat().push(m.clone());
      }
    }
    // news about the feed, not about the channel: no tab turns unread over it
    self.wake();
  }

  pub(super) fn on_feed_live(&mut self, p: Post) {
    self.feed_inline(&p, false);
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
    self.marks_load();
    let Some(focus) = self.feed.thread.as_mut().and_then(|t| t.focus.take()) else { return };
    let (Some(i), h) = (self.thread_tab(), self.chat_area().h) else { return };
    let res = &mut self.res;
    self.tabs[i].view.select_id(&focus, res, h);
    self.wake();
  }

  pub(super) fn on_voted(&mut self, id: Box<str>, res: Result<(bool, Option<u32>), u16>) {
    let prior = self.feed.end_vote(&id);
    match res {
      Ok((voted, heat)) => self.feed.voted(&id, voted, heat),
      Err(status) => {
        // the press showed at once: a refusal puts your vote back
        if let Some(was) = prior {
          self.feed.voted(&id, was, None);
        }
        self.say(if status == 401 { LOGIN_FIRST } else { TRY_AGAIN });
      }
    }
    self.refresh_inline(&id);
    self.feed_show();
  }

  pub(super) fn on_posted(&mut self, res: Result<Post, u16>) {
    let text = self.pending_post.take().unwrap_or_default();
    match res {
      Ok(p) => {
        self.say("posted to feed");
        self.feed_inline(&p, true);
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
    let t = self.tabs.get(self.active).filter(|_| self.feedish())?;
    let id = t.view.selected()?.id.clone();
    (!id.is_empty()).then_some(id)
  }

  // a reply opens its parent's thread, picked out once it lands (ext openThread's highlightId). it
  // takes over the tab it was opened from, which reads `← back` until it closes
  pub(super) fn feed_open(&mut self, id: &str) {
    if self.logs_shown().is_some() {
      return;
    }
    let root = self.feed_root(id);
    // one takeover at a time: a thread or the logs left on another tab go back first
    if self.takeover_tab().is_some_and(|i| i != self.active) {
      self.takeover_close();
    }
    let w = self.body().w;
    if let Some(t) = self.tabs.get_mut(self.active)
      && t.under.is_none()
    {
      let mut v = ChatView::new(THREAD_ROWS, "".into());
      v.glow = false;
      v.top = true;
      v.timestamps = false;
      v.resize(w);
      t.under = Some(std::mem::replace(&mut t.view, v));
      self.bar_dirty = true;
    }
    self.feed.open(&root, (*root != *id).then_some(id));
    self.net.send(Cmd::Social(social::Cmd::Thread { id: root }));
    self.feed_show();
    let res = &mut self.res;
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.deselect();
      t.view.scroll_to_top(res);
    }
  }

  // back out of whatever took the active tab over: its own lines return as they were left
  pub(super) fn takeover_close(&mut self) -> bool {
    let Some(i) = self.takeover_tab() else { return false };
    self.logs = None;
    let t = &mut self.tabs[i];
    let Some(under) = t.under.take() else { return false };
    t.view = under;
    t.view.invalidate();
    self.feed.close();
    self.bar_dirty = true;
    self.input_dirty = true;
    self.feed_show();
    true
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

  // a click on a held post's show / hide / retry. true = the click was spent
  pub(super) fn reveal_press(&mut self) -> bool {
    if !self.feedish() {
      return false;
    }
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    let res = &mut self.res;
    let Some(id) = self.tabs.get_mut(self.active).and_then(|t| t.view.reveal_at(chat, x, y, res)) else {
      return false;
    };
    if let Some(id) = self.feed.toggle_reveal(&id) {
      self.net.send(Cmd::Social(social::Cmd::Media { id }));
    }
    self.feed_show();
    true
  }

  // a click on a row's ▲ votes on that row. true = the click was a vote press (spent either way)
  pub(super) fn vote_press(&mut self) -> bool {
    if !self.feedish() {
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

  // u, the ▲, or the menu: flip your vote now; the answer settles it
  pub(super) fn feed_vote(&mut self) {
    let Some(id) = self.selected_row_post_id() else { return };
    if self.me.is_none() {
      return self.say(LOGIN_FIRST);
    }
    let own = self.feed.find(&id).is_some_and(|p| self.my_name().is_some_and(|n| n.eq_ignore_ascii_case(&p.user)));
    if own || !self.feed.begin_vote(&id) {
      return;
    }
    self.net.send(Cmd::Social(social::Cmd::Vote { id: id.clone() }));
    self.refresh_inline(&id);
    self.feed_show();
  }

  // esc / h / the `← back` tab: out of the thread, back to where it was opened. false = nothing to leave
  pub(super) fn feed_back(&mut self) -> bool {
    if !self.tabs.get(self.active).is_some_and(|t| t.under.is_some()) {
      return false;
    }
    self.takeover_close()
  }

  // a typed line in the feed tab: a new post, or a reply to the open thread
  pub(super) fn feed_post(&mut self) {
    let Some(text) = self.line.take() else { return };
    let reply_to = self.feed.thread.as_ref().map(|t| t.id.clone());
    self.publish(text.clone(), reply_to, text);
  }

  // send a post. `restore` is what goes back in the box if it can't go (logged out, too long) or fails
  pub(super) fn publish(&mut self, text: String, reply_to: Option<Box<str>>, restore: String) {
    let why = if self.me.is_none() {
      Some(LOGIN_FIRST)
    } else if engage::too_long(&text) {
      Some("posts are 500 characters at most")
    } else {
      None
    };
    if let Some(why) = why {
      self.line.load(restore);
      self.input_dirty = true;
      return self.say(why);
    }
    self.pending_post = Some(restore);
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
