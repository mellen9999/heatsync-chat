// what heatsync pushes when something changed elsewhere (another device, another person, a moderator): the
// app takes it in live, with no restart. posts hidden / edited / renumbered / moderated, a colour, a
// bookmark, the unread count, plus, and the lists that are read again (follows, blocked emotes, blocks) -
// those through model::livegate so a burst of frames is one request. frames are read by net/pushed.rs
use std::time::Instant;

use super::App;
use crate::cosmetics::id::PaintId;
use crate::model::livegate::Ask;
use crate::model::tabs::Kind;
use crate::net::emote_ops::EmoteCmd;
use crate::net::pushed::{Edit, Meta, Mod, ProfileColor};
use crate::net::{Cmd, NetEvent};

impl App {
  pub(super) fn on_pushed(&mut self, ev: NetEvent) {
    match ev {
      NetEvent::ProfileColor(c) => self.on_profile_color(c),
      NetEvent::PostHidden(id) => self.post_gone(&id),
      NetEvent::PostEdited(e) => self.on_post_edited(e),
      NetEvent::PostMeta(m) => self.on_post_meta(m),
      NetEvent::PostModerated(id, m) => self.on_post_moderated(&id, &m),
      NetEvent::Bookmark(id, on) => {
        if self.me.is_some() {
          self.feed.marks.pushed(&id, on);
        }
      }
      NetEvent::Unread(n) => {
        // read on another device: nothing here is left to tell the account about
        if n == 0 {
          self.notif_unread = false;
        }
      }
      NetEvent::PlusMoved => self.on_plus_moved(),
      NetEvent::ContentPrefs(p) => self.on_content_prefs(&p),
      NetEvent::Refetch(ask) => self.on_refetch(ask),
      _ => {}
    }
  }

  // a name's heatsync colour: their feed posts (list, open thread, inline lines) and dm rows wear it now
  fn on_profile_color(&mut self, c: ProfileColor) {
    self.card_names_changed(&c.names);
    let ids = self.feed.recolor(&c.names, c.color);
    for id in &ids {
      self.refresh_inline(id);
    }
    if !ids.is_empty() {
      self.feed_show();
    }
    for key in self.whisper.timeline.recolor(&c.names, c.color) {
      if let Some(w) = self.whisper.timeline.get(key).cloned() {
        self.put_row(&w);
      }
    }
    self.wake();
  }

  // someone else's edit: the new words in the list, the thread and the chat line
  fn on_post_edited(&mut self, e: Edit) {
    if self.feed.edited_live(&e.id, &e.text, e.refs.as_deref()) {
      self.post_changed(&e.id);
    }
  }

  fn on_post_meta(&mut self, m: Meta) {
    if self.feed.meta(&m.id, m.replies, m.heat) {
      self.post_changed(&m.id);
    }
  }

  fn on_post_moderated(&mut self, id: &str, m: &Mod) {
    if self.feed.moderated(id, m) {
      self.post_changed(id);
    }
  }

  // a post's copy changed: redrawn wherever it shows
  fn post_changed(&mut self, id: &str) {
    self.refresh_inline(id);
    self.feed_show();
  }

  // the site's flagged-content switches, as they are on the account: applied the way flipping them here would
  // be. your own flips here stay on this device (the site is where the age check lives), so the next word
  // from the site wins
  fn on_content_prefs(&mut self, prefs: &[(crate::heatsync::content::Cat, bool)]) {
    if self.me.is_none() {
      return;
    }
    let moved = prefs.iter().fold(false, |m, (c, on)| self.set.set_content(*c, *on) | m);
    if moved {
      self.applied(crate::model::schema::Fx::Content);
      self.set.save();
    }
  }

  // you gained or lost plus: how you look (the `+` after your name) is asked for again
  fn on_plus_moved(&mut self) {
    let mut ids: Vec<PaintId> = Vec::new();
    if let Some(n) = self.me.as_ref().and_then(|m| m.twitch_id) {
      ids.extend([PaintId::Twitch(n), PaintId::Native(n.to_string().into())]);
    }
    if let Some(u) = self.hs_uid.clone() {
      ids.push(PaintId::Native(u));
    }
    ids.iter().for_each(|id| self.res.cosmetics.stale(id));
    self.wake();
  }

  // the account says a list changed: ask now, or (inside the gap of the last ask) once when the gap ends
  pub(super) fn on_refetch(&mut self, ask: Ask) {
    if self.me.is_some() && self.pushed.hit(ask, Instant::now()) {
      self.refetch(ask);
    }
  }

  fn refetch(&mut self, ask: Ask) {
    match ask {
      Ask::Follows => {
        self.following_ask_now();
        self.card_follows_changed();
      }
      Ask::EmoteBlocks => self.net.send(Cmd::Emote(EmoteCmd::Blocked)),
      Ask::Blocks => self.net.send(Cmd::Lists),
      Ask::MentionRules => self.mrules_ask(),
    }
  }

  pub(super) fn pushed_due(&self) -> Option<Instant> {
    self.pushed.due()
  }

  pub(super) fn pushed_tick(&mut self, now: Instant) {
    for ask in self.pushed.take(now) {
      if self.me.is_some() {
        self.refetch(ask);
      }
    }
  }

  // a post is gone for everyone (hidden by moderation): out of every list and thread, its chat line greyed
  // like any delete
  pub(super) fn post_gone(&mut self, id: &str) {
    for t in self.tabs.iter_mut().filter(|t| t.kind == Kind::Chat) {
      t.chat().clear_msg(id);
    }
    self.feed.remove(id);
    if self.edit.as_ref().is_some_and(|e| &*e.id == id) {
      self.edit_end();
    }
    self.feed_show();
  }
}
