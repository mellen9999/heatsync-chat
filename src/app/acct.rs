// your settings follow your heatsync account, like the ext's: what the account says is applied the way
// flipping the switch would, and what you change here goes out (model/sync.rs holds the rules)
use std::time::{Duration, Instant};

use super::App;
use crate::model::filter;
use crate::model::schema::Ctl;
use crate::model::sync::{self, Inbound};
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
    self.on_texts(&st);
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

  // the account's free-text settings (automod patterns, filter rules). the first word from the account about
  // one that says nothing, while we hold some, is not a delete: ours goes up instead
  fn on_texts(&mut self, st: &Inbound) {
    let now = now_ms();
    let mut changed = false;
    for (key, value, first) in self.acct.text_inbound(st, now) {
      let blank = |t: &str| matches!(t.trim(), "" | "[]");
      let mine = filter::with(|f| match key {
        sync::AUTOMOD => f.patterns_text(),
        sync::REASONS => f.reasons_text(),
        _ => f.rules().json(),
      });
      if first && blank(&value) && !blank(&mine) {
        self.acct.text_queue(key, mine, now);
        continue;
      }
      let ok = filter::edit(|f| match key {
        sync::AUTOMOD => f.set_patterns_text(&value),
        sync::REASONS => f.set_reasons_text(&value),
        _ => f.set_rules_text(&value),
      });
      if ok {
        changed = true;
      } else {
        self.acct.ignored += 1;
      }
    }
    if changed {
      self.saved();
    }
  }

  // you edited the patterns or the rules: they go to the account with the next patch
  pub(super) fn texts_edited(&mut self) {
    if self.me.is_none() {
      return;
    }
    let now = now_ms();
    let (patterns, rules, reasons) = filter::with(|f| (f.patterns_text(), f.rules().json(), f.reasons_text()));
    self.acct.text_queue(sync::AUTOMOD, patterns, now);
    self.acct.text_queue(sync::REASONS, reasons, now);
    self.acct.text_queue(sync::RULES, rules, now);
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
