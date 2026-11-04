// the chat logs (ext chat-logs.js): right-click a line (or a card) → chat logs opens one person's archived
// lines over the tab, newest at the bottom, drawn as chat. scroll to the top for older ones, / searches
// them, a swaps this channel for all channels, y copies the line's public link, esc / q / h goes back.
// the tab reads `← back` like a thread does; the data and its caps live in model::logs
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::pop::Subject;
use crate::model::logs::{self, Applied, Logs, Page, Who};
use crate::model::message::{Message, Platform};
use crate::net::{Cmd, social};
use crate::ui::chat_view::ChatView;

// what a failed ask says, in plain words
fn why(status: u16) -> &'static str {
  match status {
    0 => "couldn't reach heatsync — press r to try again",
    404 => "no logged messages",
    410 => "they opted out of the public archive",
    429 => "too many asks — wait a moment, then press r",
    503 => "the archive is busy — try r in half a minute",
    _ => "couldn't load the archive — press r to try again",
  }
}

impl App {
  pub(super) fn logs_open(&mut self, w: &Subject) {
    let Some(login) = logs::archive_name(w.platform, &w.login) else {
      return self.say("no chat logs for that name");
    };
    // youtube keys its rooms differently from the room a line carries: it opens on every channel
    let home = if w.platform == Platform::Youtube { None } else { logs::room(&w.chan) };
    self.takeover_close();
    let width = self.body().w;
    let Some(t) = self.tabs.get_mut(self.active) else { return };
    let mut v = ChatView::new(logs::CAP + 1, "".into());
    v.glow = false;
    v.timestamps = false;
    v.resize(width);
    t.under = Some(std::mem::replace(&mut t.view, v));
    let who = Who { platform: w.platform, login, nick: w.nick.clone(), user_id: w.user_id.clone() };
    self.logs = Some(Logs::new(who, home));
    self.bar_dirty = true;
    self.input_dirty = true;
    self.logs_fetch();
  }

  // the logs while they are on screen. they belong to the tab they took over (it reads `← back` and
  // keeps them while another tab is up), so nothing of theirs (keys, the line menu, the box's refusal)
  // reaches another tab
  pub(super) fn logs_shown(&self) -> Option<&Logs> {
    self.logs.as_ref().filter(|_| self.takeover_tab() == Some(self.active))
  }

  // esc / q / h / the `← back` tab: the tab's own lines return as they were left
  pub(super) fn logs_close(&mut self) {
    self.takeover_close();
  }

  // redraw the view from the state, keeping scroll and pick
  fn logs_show(&mut self) {
    let (Some(l), Some(i)) = (&self.logs, self.takeover_tab()) else { return };
    let (lines, empty, room) = (l.lines(), l.empty_text(), l.channel().unwrap_or(""));
    let v = &mut self.tabs[i].view;
    v.channel = room.into();
    v.set_empty(&empty);
    if lines.is_empty() {
      v.clear();
    } else {
      v.reload(lines);
    }
    self.input_dirty = true;
    self.wake();
  }

  // ask for the next older page, if one is due
  fn logs_fetch(&mut self) {
    if let Some(req) = self.logs.as_mut().and_then(Logs::next) {
      self.net.send(Cmd::Social(social::Cmd::Logs(req)));
    }
    self.logs_show();
  }

  pub(super) fn on_logs(&mut self, epoch: u32, res: Result<Page, u16>) {
    let Some(l) = self.logs.as_mut() else { return };
    let said = match l.apply(epoch, res) {
      Applied::Stale => return,
      Applied::More => return self.logs_fetch(),
      Applied::Shown => None,
      Applied::Failed(status) => Some(why(status)),
    };
    self.logs_show();
    if let Some(s) = said {
      self.say(s);
    }
    self.logs_edge();
  }

  // the oldest line is on screen: the next page is wanted (called after anything that scrolls)
  pub(super) fn logs_edge(&mut self) {
    let due = self.logs.as_ref().is_some_and(|l| !(l.loading || l.done || l.trimmed || l.failed));
    let (Some(i), true) = (self.takeover_tab(), due) else { return };
    let (h, res) = (self.chat_area().h, &mut self.res);
    if self.tabs[i].view.at_oldest(res, h) {
      self.logs_fetch();
    }
  }

  // a line the person says while it is open lands at the bottom (any room when it shows every channel)
  pub(super) fn logs_live(&mut self, platform: Platform, channel: &str, m: &Message) {
    let Some(l) = self.logs.as_mut().filter(|l| l.who.platform == platform) else { return };
    if logs::archive_name(platform, &m.login).as_deref() != Some(&*l.who.login) {
      return;
    }
    let Some(row) = l.push_live(channel, m.clone()) else { return };
    let (line, i) = (l.view_msg(&row), self.takeover_tab());
    if let Some(i) = i {
      self.tabs[i].view.push(line);
      if i == self.active {
        self.wake();
      }
    }
  }

  // the prompt took a search (enter); empty = back to the timeline
  pub(super) fn logs_search(&mut self, q: &str) {
    if let Some(l) = &mut self.logs {
      l.set_query(q);
    }
    self.logs_fetch();
  }

  // what the prompt starts with: the search in effect
  pub(super) fn logs_query(&self) -> Option<&str> {
    self.logs.as_ref().and_then(|l| l.query.as_deref())
  }

  fn logs_scope(&mut self) {
    let Some(l) = &mut self.logs else { return };
    if !l.toggle_scope() {
      return self.say("opened with no channel: this is already every channel");
    }
    let note = match l.channel() {
      Some(c) => format!("#{c} only"),
      None => "every channel".into(),
    };
    self.logs_fetch();
    self.say(note);
  }

  // y: the picked line's public link
  fn logs_copy(&mut self) {
    let url = {
      let (Some(l), Some(i)) = (&self.logs, self.takeover_tab()) else { return };
      let Some(m) = self.tabs[i].view.selected() else {
        return self.say("pick a line first (j or k)");
      };
      l.link_of(m)
    };
    match url {
      Some(u) => self.copy(&u, "copied the link"),
      None => self.say("that line has no link"),
    }
  }

  // j / k move the pick; stuck at the oldest one means ask for older
  fn logs_move(&mut self, by: i32) {
    let (Some(i), h) = (self.takeover_tab(), self.chat_area().h) else { return };
    let (v, res) = (&mut self.tabs[i].view, &mut self.res);
    let was = v.selected().map(|m| (m.ts, m.id.clone()));
    if was.is_none() {
      v.select_start(res, h);
    } else {
      v.select_move(by, res, h);
    }
    if by < 0 && was.is_some() && was == v.selected().map(|m| (m.ts, m.id.clone())) {
      v.scroll_to_top(res);
    }
    self.logs_edge();
    self.wake();
  }

  // g: the oldest line on screen, and older ones asked for
  fn logs_top(&mut self) {
    if let Some(i) = self.takeover_tab() {
      self.tabs[i].view.scroll_to_top(&mut self.res);
    }
    self.logs_edge();
  }

  // the view's own keys, ahead of the box's: it has no use for typing. false = not one of them
  pub(super) fn logs_key(&mut self, key: Key<&str>) -> bool {
    if self.logs_shown().is_none() || self.mods.control_key() || self.mods.alt_key() {
      return false;
    }
    match key {
      Key::Named(NamedKey::Escape) if self.logs_query().is_some() => self.logs_search(""),
      Key::Named(NamedKey::Escape | NamedKey::Backspace) | Key::Character("q" | "h") => self.logs_close(),
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => self.logs_move(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => self.logs_move(-1),
      Key::Character("g") => self.logs_top(),
      Key::Character("G") => {
        if let Some(i) = self.takeover_tab() {
          self.tabs[i].view.scroll_to_bottom();
        }
      }
      Key::Character("a") => self.logs_scope(),
      Key::Character("y") => self.logs_copy(),
      Key::Character("r") => {
        if let Some(l) = &mut self.logs {
          l.refresh();
        }
        self.logs_fetch();
      }
      Key::Character("/") => self.find_open(),
      _ => return false,
    }
    self.wake();
    true
  }
}
