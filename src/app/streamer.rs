// streamer mode: nothing private shows on a stream. while it is on, whisper words, link previews and
// inline pictures, popup text and (unless kept) sounds are held back; a small marker says so. `auto`
// follows obs: a process named `obs` is looked for every 10s, and only while auto is the setting, so
// off and on cost nothing. the hiding lives at the chokepoints (previews/media in `Res`, whisper rows,
// toasts, `mention_alert`, `line_sound`); this file decides when it is on and rebuilds what changed.
// tokens are never drawn anywhere, so nothing here has to hide them
use crate::i18n::t;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::App;
use crate::model::settings::{Settings, Streamer as Mode};
use crate::model::streamer::{CHECK_SECS, obs_running, proc_root};
use crate::model::tabs::Kind;
use crate::model::whisper;

pub(super) struct State {
  // where processes are listed (unix: /proc, or a fake one in tests)
  pub(super) root: PathBuf,
  pub(super) obs: bool,
  // when auto looks again (none unless auto)
  pub(super) due: Option<Instant>,
  // what the screen currently follows
  pub(super) on: bool,
  // the whispers tab was shown on purpose (space s); forgotten when the mode comes back on
  pub(super) shown: bool,
}

impl State {
  pub(super) fn boot(set: &Settings) -> State {
    let mut s = State { root: proc_root(), obs: false, due: None, on: false, shown: false };
    s.look(set.streamer, Instant::now());
    s.on = s.hot(set.streamer);
    s
  }

  fn hot(&self, mode: Mode) -> bool {
    match mode {
      Mode::Off => false,
      Mode::On => true,
      Mode::Auto => self.obs,
    }
  }

  // auto looks for obs now and again in 10s; anything else forgets obs and stops the clock
  fn look(&mut self, mode: Mode, now: Instant) {
    if mode == Mode::Auto {
      self.obs = obs_running(&self.root);
      self.due = Some(now + Duration::from_secs(CHECK_SECS));
    } else {
      self.obs = false;
      self.due = None;
    }
  }
}

impl App {
  // streamer mode is up right now
  pub(super) fn streamer_hot(&self) -> bool {
    self.streamer.on
  }

  // whisper words stay off the whispers tab
  pub(super) fn hides_whispers(&self) -> bool {
    self.streamer.on && !self.streamer.shown
  }

  // sounds are muted
  pub(super) fn sound_muted(&self) -> bool {
    self.streamer.on && !self.set.streamer_sounds
  }

  // text for a popup: nothing while hot (the title says who and where)
  pub(super) fn popup_body<'a>(&self, body: &'a str) -> &'a str {
    if self.streamer.on { "" } else { body }
  }

  // link previews and inline pictures follow the mode (and the media switch); previews also follow their own switch
  pub(super) fn sync_media(&mut self) {
    self.res.previews.hush = self.streamer.on || !self.set.link_previews;
    self.res.media = self.set.media && !self.streamer.on;
  }

  // the setting changed, or auto looked again: follow it
  pub(super) fn streamer_follow(&mut self) {
    let was = self.streamer.on;
    let mode = self.set.streamer;
    self.streamer.look(mode, Instant::now());
    self.streamer.on = self.streamer.hot(mode);
    if self.streamer.on != was {
      // coming back on re-hides the whispers tab
      self.streamer.shown = false;
      self.streamer_changed();
    }
  }

  // the timer: auto looks for obs again
  pub(super) fn streamer_tick(&mut self, now: Instant) {
    if self.streamer.due.is_some_and(|t| t <= now) {
      self.streamer_follow();
    }
  }

  // space s: show the whispers (or hide them again) for this session
  pub(super) fn streamer_reveal(&mut self) {
    if !self.streamer.on {
      return;
    }
    self.streamer.shown = !self.streamer.shown;
    self.whispers_rebuild();
    self.say(if self.streamer.shown { t("app.streamer.shown") } else { t("app.streamer.hidden") });
  }

  // a click on a hidden whisper line shows the whispers, same as space s. true = the click was spent
  pub(super) fn hidden_press(&mut self) -> bool {
    if self.kind() != Kind::Whispers || !self.hides_whispers() {
      return false;
    }
    let (chat, y) = (self.chat_area(), self.pointer.1);
    let res = &mut self.res;
    if !self.tabs.get_mut(self.active).is_some_and(|t| t.view.select_at(chat, y, res)) {
      return false;
    }
    self.streamer_reveal();
    true
  }

  // the screen has to follow a flip: layouts, whisper rows, the marker
  fn streamer_changed(&mut self) {
    self.sync_media();
    self.relayout_where(None);
    self.whispers_rebuild();
    self.toasts_rebuild();
    self.hide_media_peek();
    self.bar_dirty = true;
    self.wake();
  }

  // every whispers row again, with or without its words
  pub(super) fn whispers_rebuild(&mut self) {
    let Some(i) = self.tab_of(Kind::Whispers) else { return };
    if self.whisper.timeline.is_empty() {
      return;
    }
    let hide = self.hides_whispers();
    let rows: Vec<_> = self.whisper.timeline.iter().map(|w| whisper::message(w, hide)).collect();
    let v = self.tabs[i].chat();
    v.clear();
    for m in rows {
      v.push(m);
    }
    self.wake();
  }

  // the bar's marker
  pub(super) fn streamer_mark(&self) -> Option<&'static str> {
    self.streamer.on.then_some(t("app.streamer.mark"))
  }
}
