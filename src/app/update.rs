// the ↑ button: a newer release was found; one click downloads it, checks its signature, swaps the
// binary and starts the app again. a copy owned by the package manager opens the release page instead
use std::time::{Duration, Instant};

use super::App;
use crate::net::Cmd;
use crate::net::update::{self, Release};

// a newer release, once found; installing it; when to look again
pub(super) struct Update {
  pub(super) release: Option<Release>,
  pub(super) installing: bool,
  pub(super) due: Option<Instant>,
}

// the first look waits for the window to settle; then every 6 hours
pub(super) const FIRST_CHECK: Duration = Duration::from_secs(20);
const EVERY: Duration = Duration::from_secs(6 * 3600);

impl App {
  pub(super) fn update_check(&mut self) {
    self.upd.due = Some(Instant::now() + EVERY);
    if self.set.updates && self.upd.release.is_none() {
      self.net.send(Cmd::Update(update::Cmd::Check));
    }
  }

  pub(super) fn update_label(&self) -> Option<String> {
    let r = self.upd.release.as_ref()?;
    Some(if self.upd.installing { "↑ updating…".into() } else { format!("↑ {}", r.version) })
  }

  pub(super) fn on_update(&mut self, r: Release) {
    if self.upd.release.as_ref() != Some(&r) {
      self.upd.release = Some(r);
      self.bar_dirty = true;
      self.wake();
    }
  }

  pub(super) fn update_press(&mut self) {
    let Some(r) = self.upd.release.clone() else { return };
    if self.upd.installing {
      return;
    }
    if !update::self_updates() {
      // a package manager's copy: its own update path, so just show what's new
      let _ = crate::util::open::open_url(update::PAGE);
      return self.say("installed by a package manager: update it there (the release page is open)");
    }
    self.upd.installing = true;
    self.bar_dirty = true;
    self.say(format!("downloading {}…", r.version));
    self.net.send(Cmd::Update(update::Cmd::Install(r)));
  }

  pub(super) fn on_updated(&mut self, res: Result<(), String>) {
    self.upd.installing = false;
    self.bar_dirty = true;
    match res {
      Ok(()) => {
        // tabs and settings are already on disk: start the new copy in this one's place
        self.save_tabs();
        let e = update::restart();
        self.say(format!("updated: restart the app to use it ({e})"));
        self.upd.release = None;
      }
      Err(e) => self.say(&e),
    }
    self.wake();
  }
}
