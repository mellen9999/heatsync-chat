// your settings follow your heatsync account, like the ext's: what the account says is applied the way
// flipping the switch would, and what you change here goes out (model/sync.rs holds the rules)
use std::time::{Duration, Instant};

use super::App;
use crate::model::schema::Ctl;
use crate::model::sync::Inbound;
use crate::net::Cmd;
use crate::net::hs_ws::Cmd as HsCmd;
use crate::net::social;
use crate::util::time::now_ms;

// a fetch just made already holds what a second one would
const FETCH_GAP: Duration = Duration::from_secs(2);

impl App {
  // ask the account for its settings (at login, and each time the heatsync socket signs in)
  pub(super) fn ui_state_fetch(&mut self) {
    let now = Instant::now();
    if self.me.is_none() || self.ui_get_at.is_some_and(|t| now.duration_since(t) < FETCH_GAP) {
      return;
    }
    self.ui_get_at = Some(now);
    self.net.send(Cmd::Social(social::Cmd::UiStateGet));
  }

  // the account's values: the same apply a flip in the settings screen does, saved, never sent back
  pub(super) fn on_ui_state(&mut self, st: Inbound) {
    let moved = self.acct.inbound(&st, &mut self.set, now_ms());
    if self.prof && self.acct.ignored > 0 {
      eprintln!("ui-state: {} values ignored", self.acct.ignored);
    }
    if moved.is_empty() {
      return;
    }
    let mut fx = Vec::new();
    for d in moved {
      match d.ctl {
        Ctl::Side => self.relayout_body(),
        Ctl::Counter => self.bar_dirty = true,
        _ => {}
      }
      if !fx.contains(&d.fx) {
        fx.push(d.fx);
      }
    }
    for f in fx {
      self.applied(f);
    }
    self.set.save();
    self.input_dirty = true;
    self.bar_dirty = true;
    self.wake();
  }

  // every wake: send what you changed (once the heatsync socket is signed in), ask again after a write
  // the server never echoed
  pub(super) fn ui_state_tick(&mut self) {
    let now = now_ms();
    if self.acct.stale(now) {
      self.ui_state_fetch();
    }
    let ready = self.me.is_some() && self.hs_authed;
    if let Some(patch) = self.acct.outbound(&self.set, now, ready) {
      self.net.send(Cmd::Hs(HsCmd::UiSync(patch)));
    }
  }

  pub(super) fn ui_state_due(&self) -> Option<Instant> {
    self.acct.due().map(|t| Instant::now() + Duration::from_millis((t - now_ms()).max(0) as u64))
  }
}
