// the feed post actions (ext feed-engage.js + input.js row menu): upvote, bookmark, edit and delete
// your own, `op to feed` from a chat line, and `/op` `/re`. they all act on the line picked, wherever a
// post shows (the feed tab, an open thread, an inline line in a chat tab). logged out none of the rows
// show. every change shows at once and a refusal puts it back with a plain line saying so
use super::App;
use super::feed::{login_first, try_again};
use super::overlay::Overlay;
use super::pop::{Act, Pop};
use crate::i18n::{f, t};
use crate::model::engage::{self, CHECK_CHUNK};
use crate::model::feed::MORE_ID;
use crate::model::message::Message;
use crate::model::post::Post;
use crate::model::postcmd::{self, Cmd as Slash};
use crate::model::tabs::Kind;
use crate::net::Cmd;
use crate::net::social;
use crate::util::time::now_ms;

// a post being edited: its words are in the box, and the draft that was there waits to come back
pub struct Edit {
  pub id: Box<str>,
  busy: bool,
  draft: String,
}

// what the post rows of the line menu depend on (None when logged out: no rows)
pub struct PostCtx {
  pub own: bool,
  pub voted: bool,
  pub marked: bool,
  // ms left to edit it; None = the window is shut
  pub left: Option<i64>,
}

// after the note row: upvote (never your own post) and bookmark, worded as the ext words them
pub fn engage_rows(c: &PostCtx) -> Vec<(Act, String)> {
  let mut rows = Vec::new();
  if !c.own {
    rows.push((Act::Upvote, if c.voted { t("app.engage.remove_upvote") } else { t("app.engage.upvote") }.into()));
  }
  rows.push((Act::Bookmark, if c.marked { t("app.engage.remove_bookmark") } else { t("app.engage.bookmark") }.into()));
  rows
}

// last, for your own post: edit with its clock, then delete
pub fn own_rows(c: &PostCtx) -> Vec<(Act, String)> {
  if !c.own {
    return Vec::new();
  }
  let edit = match c.left {
    Some(ms) => f("app.engage.edit_left", &[("left", &engage::clock(ms))]),
    None => t("app.engage.edit_expired").into(),
  };
  vec![(Act::Edit, edit), (Act::Delete, t("app.engage.delete").into())]
}

impl App {
  // the post a line is, if it is one: a feed row, or an inline line the feed put in a chat tab (its
  // `>>id` head is what tells it from a chat line whose id happens to look like one)
  pub(super) fn row_post(&self, m: &Message) -> Option<&Post> {
    let head = m.x.as_ref()?.head.first()?;
    if m.id.is_empty() || &*m.id == MORE_ID || !head.text.starts_with(">>") {
      return None;
    }
    self.feed.find(&m.id)
  }

  fn selected_row_post(&self) -> Option<&Post> {
    let m = self.tabs.get(self.active)?.view.selected()?;
    self.row_post(m)
  }

  pub(super) fn selected_row_post_id(&self) -> Option<Box<str>> {
    self.selected_row_post().map(|p| p.id.clone())
  }

  fn is_mine(&self, p: &Post) -> bool {
    self.my_name().is_some_and(|n| n.eq_ignore_ascii_case(&p.user))
  }

  // None logged out
  pub(super) fn post_ctx(&self, p: &Post) -> Option<PostCtx> {
    self.me.as_ref()?;
    Some(PostCtx {
      own: self.is_mine(p),
      voted: p.voted,
      marked: self.feed.marks.get(&p.id),
      left: engage::edit_left(p.ts, now_ms()),
    })
  }

  // a post's line in a chat tab changed (a vote, an edit): redraw it where it sits
  pub(super) fn refresh_inline(&mut self, id: &str) {
    let Some(p) = self.feed.find(id).cloned() else { return };
    let m = self.inline_line(&p);
    for t in self.tabs.iter_mut().filter(|t| t.kind == Kind::Chat) {
      t.chat().replace(id, m.clone());
    }
    self.wake();
  }

  // ask the server which of the posts on screen you bookmarked (once each, at most 100 a trip)
  pub(super) fn marks_load(&mut self) {
    if self.me.is_none() {
      return;
    }
    let ids = self.feed.marks_wanted();
    for c in ids.chunks(CHECK_CHUNK) {
      self.net.send(Cmd::Social(social::Cmd::BookmarkCheck { ids: c.to_vec() }));
    }
  }

  pub(super) fn on_marks(&mut self, ids: Vec<Box<str>>, res: Result<Vec<(Box<str>, bool)>, u16>) {
    self.feed.marks.checked(&ids, res.ok());
  }

  pub(super) fn feed_bookmark(&mut self) {
    let Some(id) = self.selected_row_post_id() else { return };
    if self.me.is_none() {
      return self.say(login_first());
    }
    if let Some(on) = self.feed.marks.begin(&id) {
      self.net.send(Cmd::Social(social::Cmd::Bookmark { id, on }));
    }
  }

  pub(super) fn on_bookmarked(&mut self, id: Box<str>, on: bool, res: Result<(), u16>) {
    self.feed.marks.end(&id, on, res.is_ok());
    match res {
      Ok(()) => self.say(if on { t("app.engage.bookmarked") } else { t("app.engage.bookmark_removed") }),
      Err(401) => self.say(login_first()),
      Err(_) => self.say(try_again()),
    }
  }

  // edit: the post's words go into the box, enter saves, esc drops it (like a reply)
  pub(super) fn edit_begin(&mut self) {
    let Some(p) = self.selected_row_post().cloned() else { return };
    if !self.is_mine(&p) {
      return;
    }
    if engage::edit_left(p.ts, now_ms()).is_none() {
      return self.say(t("app.engage.edit_too_late"));
    }
    let draft = match self.edit.take() {
      Some(e) => e.draft,
      None => std::mem::take(&mut self.line.text),
    };
    self.line.load(p.text.replace('\n', " "));
    self.edit = Some(Edit { id: p.id, busy: false, draft });
    self.complete = None;
    self.reply = None;
    self.enter_insert();
    self.input_dirty = true;
    self.wake();
  }

  // out of edit mode, the draft that was in the box coming back
  pub(super) fn edit_end(&mut self) {
    if let Some(e) = self.edit.take() {
      self.line.load(e.draft);
      self.after_send();
    }
  }

  // enter while editing
  pub(super) fn edit_send(&mut self) {
    let Some(e) = self.edit.as_mut().filter(|e| !e.busy) else { return };
    let id = e.id.clone();
    let text = self.line.text.trim().to_owned();
    if text.is_empty() {
      return self.say(t("app.engage.empty"));
    }
    if engage::too_long(&text) {
      return self.say(t("app.engage.too_long"));
    }
    if self.feed.find(&id).is_some_and(|p| p.text.replace('\n', " ") == text) {
      return self.edit_end();
    }
    if let Some(e) = &mut self.edit {
      e.busy = true;
    }
    self.net.send(Cmd::Social(social::Cmd::Edit { id, text }));
  }

  pub(super) fn on_edited(&mut self, id: Box<str>, res: Result<Box<str>, u16>) {
    let mine = self.edit.as_ref().is_some_and(|e| e.id == id);
    match res {
      Ok(text) => {
        self.feed.edited(&id, &text);
        self.refresh_inline(&id);
        self.feed_show();
        if mine {
          self.edit_end();
        }
        self.say(t("app.engage.edited"));
      }
      Err(status) => {
        // the words stay in the box so a refused edit is never lost
        if let Some(e) = self.edit.as_mut().filter(|e| e.id == id) {
          e.busy = false;
        }
        self.say(match status {
          401 => login_first(),
          400 => t("app.engage.edit_refused"),
          403 => t("app.engage.edit_late"),
          404 | 410 => t("app.engage.edit_gone"),
          429 => t("app.engage.edit_slow"),
          _ => try_again(),
        });
      }
    }
  }

  // delete asks first: it can't be undone
  pub(super) fn delete_ask(&mut self) {
    let Some(p) = self.selected_row_post().cloned() else { return };
    if !self.is_mine(&p) {
      return;
    }
    self.pending_delete = Some(p.id);
    let chat = self.chat_area();
    let rows = vec![(Act::DeletePost, t("app.engage.yes_delete").to_owned()), (Act::No, t("app.cancel").to_owned())];
    self.overlay =
      Some(Overlay::Pop(Pop::ask(t("app.engage.delete_ask").into(), (chat.x + 24, chat.y + chat.h / 4), rows)));
    self.pop_changed();
  }

  pub(super) fn delete_go(&mut self) {
    if let Some(id) = self.pending_delete.take() {
      self.net.send(Cmd::Social(social::Cmd::Delete { id }));
    }
  }

  pub(super) fn on_deleted(&mut self, id: Box<str>, res: Result<(), u16>) {
    match res {
      Ok(()) => {
        // an inline line in a chat tab reads as a deleted line, like a mod delete
        self.post_gone(&id);
        self.say(t("app.engage.deleted"));
      }
      Err(401) => self.say(login_first()),
      Err(403) => self.say(t("app.engage.not_yours")),
      Err(_) => self.say(t("app.engage.delete_failed")),
    }
  }

  // op to feed on a chat line: `/op @name: words` in the box, to read and send
  pub(super) fn op_to_feed(&mut self) {
    let Some(m) = self.tabs.get(self.active).and_then(|t| t.view.selected()) else { return };
    match engage::op_line(&m.nick, &m.body) {
      Some(line) => self.type_into_line(&line),
      None => self.say(t("app.engage.nothing")),
    }
  }

  // `/op <text>` and `/re <text>`; the line stays recoverable in the box if the post fails
  pub(super) fn post_slash(&mut self, cmd: Slash) {
    let (word, text, usage) = match cmd {
      Slash::Op(t) => ("/op", t, postcmd::OP_USAGE),
      Slash::Re(t) => ("/re", t, postcmd::RE_USAGE),
    };
    if text.is_empty() {
      return self.say(crate::i18n::t(usage));
    }
    let reply_to = if word == "/re" {
      match self.feed.last_op() {
        Some(id) => Some(id),
        None => return self.say(t("app.engage.no_post")),
      }
    } else {
      None
    };
    self.publish(text.clone(), reply_to, format!("{word} {text}"));
  }
}
