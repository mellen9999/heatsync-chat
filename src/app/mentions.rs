// the mentions tab (ext mentions.js): every live line that pings you, from every joined room, plus
// replies to you. in the room's own tab the line gets a red edge (the ext tints the row dark red; a
// fill behind text is off-palette here). click or Enter jumps to the line in its room's tab. while the
// window is away a mention also pings (util::sound) and flashes the title with who it was, like the ext, and
// lights the taskbar entry (a web page has no taskbar, so that part has no ext key)
use crate::i18n::{f, t};
use std::time::{Duration, Instant};

use super::App;
use crate::model::filter::{self, Cue};
use crate::model::inbox::{self, Item};
use crate::model::mention_file::{self as file, Read};
use crate::model::message::{Extra, Message, Platform, flag};
use crate::model::settings::{MentionWhen, PingSound};
use crate::model::tabs::Kind;
use crate::model::unseen::Surface;
use crate::net::{Cmd, social};
use crate::store;
use crate::util::notify;
use crate::util::sound::Voice;
use crate::util::time::now_ms;
use winit::window::UserAttentionType;

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
    // a platform every tab of this room hides pings nobody
    if m.kind.said() && self.plat_silenced(p, room) {
      return;
    }
    let muted = self.pings.muted(p, room);
    // a shared-chat line already taken in from another room: this copy is that room's to show, not a second ping
    let again = self.mentions.again(m);
    // ...but the ping is once per line across the rooms that are not muted: a muted copy leaves it for them
    let pinged = self.mentions.pinged(m, muted);
    let cue = self.line_cue(room, m);
    if let Some((voice, own)) = cue.sound.filter(|_| !muted && !pinged) {
      crate::util::sound::mention(voice, own.unwrap_or_else(|| self.set.mention_volume.percent()), now_ms());
    }
    // a blocked person never pings you (a highlight rule with notify on is a ping too, and line_cue has
    // already left a blocked person out)
    if !(self.mentions.hit(m) || cue.notify)
      || crate::model::filter::with(|f| f.blocked(&crate::model::filter::key(&m.login)))
    {
      return;
    }
    m.flags |= flag::ALERT;
    // backlog is not a new ping, and a line a mention rule already put in the tab is one row
    if m.has(flag::HISTORY) || self.mention_dup(p, room, &m.login, &m.body) {
      return;
    }
    if !again {
      self.mention_row(p, room, m);
    }
    let title = format!("{} in #{}", notify::name(&m.nick), notify::name(room));
    // a muted room's mention is in the tab, unread, but makes no sound, flash or popup
    if !muted && !pinged {
      self.mention_alert(&title, &m.nick, &m.body);
    }
  }

  // the mentions tab's row for a live ping
  fn mention_row(&mut self, p: Platform, room: &str, m: &Message) {
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
  }

  // what a highlight rule gives a live line (ext playFilterRuleSound): its sound at its own volume, else at the
  // mention volume, and a ping into the mentions tab when the rule says notify. nothing for backlog, your own
  // lines or a blocked person; a line a hide rule takes judges to nothing. the sound is held back in streamer mode
  pub(super) fn line_cue(&self, room: &str, m: &Message) -> Cue {
    if m.has(flag::HISTORY) || m.has(flag::OWN) || filter::with(|f| f.blocked(&filter::key(&m.login))) {
      return Cue::default();
    }
    let mut cue = filter::with(|f| f.rule_cue(m, room));
    if self.sound_muted() {
      cue.sound = None;
    }
    cue
  }

  // a mention while the window is away: the desktop notification (if on), the ping, the taskbar and the title
  // flash. the ping and the taskbar go together: whatever holds the sound back (streamer mode) holds the light
  pub(super) fn mention_alert(&mut self, title: &str, nick: &str, body: &str) {
    let away = !self.focused;
    // the ping can also sound while you are looking at the window (mention_sound_when); everything else needs you away
    let sounds = away || self.set.mention_sound_when == MentionWhen::Always;
    if !sounds {
      return;
    }
    if away && self.set.notify && self.notify_pace.offer(Instant::now()) {
      notify::send("mention", title, self.popup_body(body));
    }
    if !self.sound_muted() {
      crate::util::sound::mention(self.ping_voice(), self.set.mention_volume.percent(), now_ms());
      if away && self.set.taskbar_flash {
        self.light_taskbar();
      }
    }
    if away && self.set.title_flash {
      // the latest name; an already running flash picks it up on its next turn
      let shown = self.flash.as_ref().is_some_and(|f| f.1);
      self.flash = Some((nick.chars().filter(|c| !c.is_control()).take(40).collect::<String>().into(), shown));
      self.flash_due.get_or_insert_with(Instant::now);
    }
  }

  // a heatsync notification (a reply to your post, an @ in a post, a fixed report) is a row of the mentions
  // tab like any mention: it lights the tab, pings, is kept across restarts and counts against the same cap
  // (ext hsNotifIn). only a signed-in account is sent these
  pub(super) fn on_notif(&mut self, it: Item) {
    let Some(i) = self.tab_of(Kind::Mentions).filter(|_| self.me.is_some()) else { return };
    self.tabs[i].chat().push(inbox::message(&it));
    self.notif_unread = true;
    self.unseen_note(Surface::Mentions);
    self.mentions_changed();
    self.mention_alert(&notify::name("heatsync"), "heatsync", &it.text);
  }

  // you looked at the mentions tab: the account marks its heatsync notifications read, for every device
  // (ext hsNotifsRead). once, and only if one came that it hasn't been told of
  pub(super) fn notifs_read(&mut self) {
    if std::mem::take(&mut self.notif_unread) && self.me.is_some() {
      self.net.send(Cmd::Social(social::Cmd::NotifsRead));
    }
  }

  // the heatsync account the saved mentions belong to: its username, lowercased (the ext's owner)
  fn mentions_owner(&self) -> Option<String> {
    self.me.as_ref().map(|m| m.username.to_lowercase()).filter(|u| !u.is_empty())
  }

  // a mention landed: the file is written soon
  pub(super) fn mentions_changed(&mut self) {
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

  // everything that waits for its turn to be written, written now: on the way out or into a restart
  pub(super) fn flush_to_disk(&mut self) {
    self.mentions_flush();
    self.chatlog.flush();
    store::flush();
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
          // restored notification rows were never marked read here: the next look tells the account
          self.notif_unread |= rows.iter().any(inbox::is_row);
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
    self.notif_unread = false;
    store::remove(file::FILE);
    if let Some(i) = self.tab_of(Kind::Mentions) {
      self.tabs[i].chat().clear();
      self.tabs[i].unread = false;
      self.bar_dirty = true;
      self.wake();
    }
  }

  // a raid's worth of mentions held back by the 3s limit: one line says how many
  pub(super) fn notify_pace_tick(&mut self, now: Instant) {
    if let Some(text) = self.notify_pace.tick(now) {
      notify::send("mention", "heatsync", &text);
    }
  }

  // the sound a mention plays: the one picked, or your wav file when the pick is custom
  pub(super) fn ping_voice(&self) -> Voice {
    match self.set.ping_sound.preset() {
      Some(p) => Voice::Preset(p),
      None => Voice::File(self.set.ping_file.as_str().to_owned()),
    }
  }

  // the mention sound as it would play now, heard when you pick it (not while streamer mode holds sounds back).
  // custom with no file picked yet says what is missing instead
  pub(super) fn ping_preview(&mut self) {
    if self.set.ping_sound == PingSound::Custom && self.set.ping_file.is_empty() {
      return self.say(t("app.mentions.custom_hint"));
    }
    if !self.sound_muted() {
      crate::util::sound::preview(self.ping_voice(), self.set.mention_volume.percent());
    }
  }

  // a sound file was set or cleared: a file means custom, none means back to the ping; then it is heard
  pub(super) fn ping_file_follow(&mut self) {
    if !self.set.ping_file.is_empty() {
      self.set.ping_sound = PingSound::Custom;
    } else if self.set.ping_sound == PingSound::Custom {
      self.set.ping_sound = PingSound::Ping;
    }
    self.ping_preview();
  }

  // the taskbar entry lights up once per time away (on dwl the window's tag turns urgent); the compositor puts it
  // out when the window is looked at, and `on_window_focus` lets the next time away light it again
  fn light_taskbar(&mut self) {
    if std::mem::replace(&mut self.attention, true) {
      return;
    }
    if let Some(g) = &self.gfx {
      g.window.request_user_attention(Some(UserAttentionType::Informational));
    }
  }

  // the window gained or lost focus: coming back puts the title flash and the taskbar light down, leaving
  // shuts the tooltip and marks what was read
  pub(super) fn on_window_focus(&mut self, focused: bool) {
    self.focused = focused;
    if focused {
      self.flash_stop();
      self.attention = false;
    } else {
      self.hide_tip();
      self.mark_read();
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

  // the feed tab is where posts open; bring it back if it was hidden
  fn notif_open(&mut self, post: &str) {
    if !self.set.shown(Kind::Feed) {
      self.set_shown(Kind::Feed, true);
    }
    let Some(i) = self.tab_of(Kind::Feed) else { return };
    self.select(i);
    self.feed_open(post);
    self.wake();
  }

  // Enter / click on a mention: open its room's tab with the line selected
  pub(super) fn jump_to_mention(&mut self) {
    let Some(t) = self.tabs.get(self.active).filter(|t| t.kind == Kind::Mentions) else { return };
    let Some(m) = t.view.selected() else { return };
    // a heatsync notification: its post opens in the feed
    if inbox::is_row(m) {
      return match inbox::post_of(m).map(Box::<str>::from) {
        Some(post) => self.notif_open(&post),
        None => self.say(crate::i18n::t("app.mentions.nothing_to_open")),
      };
    }
    let (p, id) = (m.platform, m.id.clone());
    let Some(room) = m.x.as_ref().map(|x| x.chan.clone()) else { return };
    let Some(i) = self.tabs_with(p, &room).first().copied() else {
      return self.say(f("app.mentions.not_open", &[("room", &room)]));
    };
    self.select(i);
    let h = self.chat_area().h;
    let res = &mut self.res;
    if !self.tabs[i].view.select_id(&id, res, h) {
      self.say(crate::i18n::t("app.mentions.scrolled_out"));
    }
    self.wake();
  }
}
