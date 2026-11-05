// the mentions tab (ext mentions.js): every live line that pings you, from every joined room, plus
// replies to you. in the room's own tab the line gets a red edge (the ext tints the row dark red; a
// fill behind text is off-palette here). click or Enter jumps to the line in its room's tab. while the
// window is away a mention also pings (util::sound) and flashes the title with who it was, like the ext
use std::time::{Duration, Instant};

use super::App;
use crate::model::mention_file::{self as file, Read};
use crate::model::message::{Extra, Message, Platform, flag};
use crate::model::tabs::Kind;
use crate::store;
use crate::util::notify;
use crate::util::time::now_ms;

pub(super) const TITLE: &str = "heatsync";
const FLASH_EVERY: Duration = Duration::from_millis(1200);
// a mention is written this long after it lands (a burst is one write)
const SAVE_AFTER: Duration = Duration::from_millis(1500);

impl App {
  // names that ping you changed (login / logout)
  pub(super) fn set_mention_names(&mut self) {
    self.mentions.set(self.me.iter().flat_map(|m| m.names.iter().map(|n| &**n)));
  }

  // tint a line that pings you; a live one also lands in the mentions tab (and may notify).
  // `room` is the tab key it came from (twitch channel, kick slug, youtube handle)
  pub(super) fn scan_mention(&mut self, p: Platform, room: &str, m: &mut Message) {
    // a blocked person never pings you
    if !self.mentions.hit(m) || crate::model::filter::with(|f| f.blocked(&crate::model::filter::key(&m.login))) {
      return;
    }
    m.flags |= flag::ALERT;
    if m.has(flag::HISTORY) {
      return;
    }
    let mut copy = m.clone();
    copy.x = Some(Box::new(Extra { chan: room.into(), ..Extra::default() }));
    copy.platform = p;
    // a copy of a line is not a new person: the first-line glow never applies here
    copy.flags &= !(flag::NEW | flag::FIRST);
    if let Some(i) = self.tab_of(Kind::Mentions) {
      self.tabs[i].chat().push(copy);
      self.unseen_note(crate::model::unseen::Surface::Mentions);
      self.mentions_changed();
    }
    if self.focused {
      return;
    }
    if self.set.notify {
      let title = format!("{} in #{}", notify::name(&m.nick), notify::name(room));
      notify::send("mention", &title, &m.body);
    }
    crate::util::sound::ping(self.set.mention_volume.percent(), crate::util::time::now_ms());
    if self.set.title_flash {
      // the latest name; an already running flash picks it up on its next turn
      let shown = self.flash.as_ref().is_some_and(|f| f.1);
      self.flash = Some((m.nick.chars().filter(|c| !c.is_control()).take(40).collect::<String>().into(), shown));
      self.flash_due.get_or_insert_with(Instant::now);
    }
  }

  // the heatsync account the saved mentions belong to: its username, lowercased (the ext's owner)
  fn mentions_owner(&self) -> Option<String> {
    self.me.as_ref().map(|m| m.username.to_lowercase()).filter(|u| !u.is_empty())
  }

  // a mention landed: the file is written soon
  fn mentions_changed(&mut self) {
    self.mentions_save.get_or_insert_with(|| Instant::now() + SAVE_AFTER);
  }

  pub(super) fn mentions_save_tick(&mut self, now: Instant) {
    if self.mentions_save.is_some_and(|t| t <= now) {
      self.mentions_flush();
    }
  }

  // write the tab's last lines now (when one is due), tagged with the account. also on the way out
  pub(super) fn mentions_flush(&mut self) {
    if self.mentions_save.take().is_none() {
      return;
    }
    let (Some(owner), Some(i)) = (self.mentions_owner(), self.tab_of(Kind::Mentions)) else { return };
    let v = self.tabs[i].chat();
    if v.len() == 0 {
      return store::remove(file::FILE);
    }
    // a failed save is said once by the store
    let _ = store::write(file::FILE, &file::encode(&owner, now_ms(), v.msgs()));
  }

  // signed in as someone: the tab gets their last mentions back, as history, if the file is theirs and
  // under a day old. another account's file is left alone; a stale or unreadable one is dealt with
  pub(super) fn mentions_restore(&mut self) {
    let (Some(owner), Some(i)) = (self.mentions_owner(), self.tab_of(Kind::Mentions)) else { return };
    let Some(text) = store::read_capped(file::FILE, file::MAX_BYTES) else { return };
    match file::decode(&text, &owner, now_ms()) {
      Read::Rows { rows, junk } => {
        if junk > 0 {
          store::set_aside(file::FILE, Some(junk));
        }
        if !rows.is_empty() {
          self.tabs[i].chat().prepend(rows);
          // a file with unusable lines is written again without them
          if junk > 0 {
            self.mentions_changed();
          }
          self.wake();
        }
      }
      Read::Stale => store::remove(file::FILE),
      Read::Bad => store::set_aside(file::FILE, None),
      Read::NotYours => {}
    }
  }

  // logged out, or another account came in: the file and the tab's lines were the last one's
  pub(super) fn mentions_forget(&mut self) {
    self.mentions_save = None;
    store::remove(file::FILE);
    if let Some(i) = self.tab_of(Kind::Mentions) {
      self.tabs[i].chat().clear();
      self.tabs[i].unread = false;
      self.bar_dirty = true;
      self.wake();
    }
  }

  pub(super) fn flash_tick(&mut self) {
    let Some((name, shown)) = self.flash.as_mut().filter(|_| self.set.title_flash && !self.focused) else {
      return self.flash_stop();
    };
    *shown = !*shown;
    let title = if *shown { format!("@{name} ← {TITLE}") } else { TITLE.to_owned() };
    if let Some(g) = &self.gfx {
      g.window.set_title(&title);
    }
    self.flash_due = Some(Instant::now() + FLASH_EVERY);
  }

  pub(super) fn flash_stop(&mut self) {
    self.flash_due = None;
    if self.flash.take().is_some_and(|f| f.1)
      && let Some(g) = &self.gfx
    {
      g.window.set_title(TITLE);
    }
  }

  // Enter / click on a mention: open its room's tab with the line selected
  pub(super) fn jump_to_mention(&mut self) {
    let Some(t) = self.tabs.get(self.active).filter(|t| t.kind == Kind::Mentions) else { return };
    let Some(m) = t.view.selected() else { return };
    let (p, id) = (m.platform, m.id.clone());
    let Some(room) = m.x.as_ref().map(|x| x.chan.clone()) else { return };
    let Some(i) = self.tabs_with(p, &room).first().copied() else {
      return self.say(format!("{room} is not open as a tab"));
    };
    self.select(i);
    let h = self.chat_area().h;
    let res = &mut self.res;
    if !self.tabs[i].view.select_id(&id, res, h) {
      self.say("that line has scrolled out of its tab");
    }
    self.wake();
  }
}
