// the red dot on a tab whose channel is live (ext data-live + updateLiveStatus): asked of heatsync every
// 90s (the ext's poll) and a moment after a tab or a linked feed opens; a go-live or sign-off on the
// socket and youtube's relay status move it at once. a failed answer keeps the dots it had
use std::time::{Duration, Instant};

use super::App;
use crate::model::live_dots::{Names, askable};
use crate::model::message::Platform;
use crate::net::Cmd;

const EVERY: Duration = Duration::from_secs(90);
const SOON: Duration = Duration::from_secs(2);

impl App {
  pub(super) fn live_due(&self) -> Option<Instant> {
    self.live_due
  }

  // a new tab or feed: ask shortly, unless an ask is already that close
  pub(super) fn live_soon(&mut self) {
    let at = Instant::now() + SOON;
    if self.live_due.is_none_or(|t| t > at) {
      self.live_due = Some(at);
    }
  }

  pub(super) fn live_tick(&mut self, now: Instant) {
    self.live_due = Some(now + EVERY);
    let open = |p| -> Vec<Box<str>> {
      let mut v: Vec<Box<str>> = self
        .tabs
        .iter()
        .flat_map(|t| &t.sources)
        .filter(|s| s.0 == p && askable(p, &s.1))
        .map(|s| s.1.to_ascii_lowercase().into())
        .collect();
      v.sort();
      v.dedup();
      v
    };
    let (twitch, kick) = (open(Platform::Twitch), open(Platform::Kick));
    let tabs = &self.tabs;
    self.live.keep(|p, k| tabs.iter().any(|t| t.sources.iter().any(|s| s.0 == p && s.1.eq_ignore_ascii_case(k))));
    if !twitch.is_empty() || !kick.is_empty() {
      self.net.send(Cmd::LiveDots(twitch, kick));
    }
  }

  pub(super) fn on_live_dots(&mut self, answers: [(Platform, Names, Option<Names>); 2]) {
    let mut changed = false;
    for (p, asked, live) in answers {
      if let Some(live) = live {
        changed |= self.live.answer(p, &asked, &live);
      }
    }
    if changed {
      self.bar_dirty = true;
      self.wake();
    }
  }

  // the socket or the relay said so: now, not at the next poll
  pub(super) fn live_mark(&mut self, p: Platform, key: &str, on: bool) {
    if self.live.set(p, key, on) {
      self.bar_dirty = true;
      self.wake();
    }
  }
}
