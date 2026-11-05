// your channel tabs follow your heatsync account, like the ext's (model/tabsync.rs holds the rules): the
// account's list is asked for once you and the heatsync socket are both signed in and laid over your tabs
// whenever it changes them, and every change you make here goes up. logged out nothing changes; a run given
// its channels on the command line keeps out of it both ways
use super::tabs::{make_tab, seat_hosts, spec_of};
use super::{App, Tab};
use crate::model::chan::Spec;
use crate::model::sendto::Targets;
use crate::model::tabs::Kind;
use crate::model::tabsync::{Frame, TabSync};
use crate::net::Cmd;
use crate::net::hs_ws::Cmd as HsCmd;
use crate::store;
use crate::util::time::now_ms;

// the account's list as last agreed, kept between runs (so what other clients store survives a change made
// before the account answers)
const SAVED: &str = "tabsync";

fn is_channel(t: &Tab) -> bool {
  // the temp tab is yours to look at, not part of the list that follows your account
  t.kind == Kind::Chat && !t.sources.is_empty() && !t.temp
}

// a tab, by what it is rather than where it is
type Key = (Kind, Box<str>, Spec);

fn key(t: &Tab) -> Key {
  (t.kind, t.name.clone(), spec_of(t))
}

impl App {
  fn tab_sync_user(&self) -> Option<Box<str>> {
    if store::tabs_pinned() {
      return None;
    }
    self.me.as_ref().map(|m| m.username.clone())
  }

  pub(super) fn channel_specs(&self) -> Vec<Spec> {
    self.tabs.iter().filter(|t| is_channel(t)).map(spec_of).collect()
  }

  // another login (or none): start over, from what was kept for that account
  pub(super) fn tab_sync_login(&mut self, user: Option<&str>) {
    self.tabsync = TabSync::default();
    if let (Some(u), Some(t)) = (user, store::read(SAVED)) {
      self.tabsync.load(u, &t);
    }
  }

  // signed in at both ends: a change made while the socket was down goes up, and the list is asked for (the
  // server sends it after sign-in too, but that can land before we know who we are)
  pub(super) fn tab_sync_ready(&mut self) {
    if self.tab_sync_user().is_none() || !self.hs_authed {
      return;
    }
    if std::mem::take(&mut self.tabsync.owed) {
      self.tab_sync_push();
    }
    self.net.send(Cmd::Hs(HsCmd::TabPull));
  }

  // your tabs changed here (save_tabs): they go to the account
  pub(super) fn tab_sync_push(&mut self) {
    let Some(user) = self.tab_sync_user() else { return };
    if !self.hs_authed {
      self.tabsync.owed = true;
      return;
    }
    let mut sent = false;
    if let Some(list) = self.tabsync.outbound(&self.channel_specs(), now_ms()) {
      self.net.send(Cmd::Hs(HsCmd::TabSync(list)));
      sent = true;
    }
    // a tab's send choice goes into its entry, a new one included
    if let Some(list) = self.tabsync.apply_targets(&self.channel_targets()) {
      self.net.send(Cmd::Hs(HsCmd::TabSync(list)));
      sent = true;
    }
    if sent {
      let _ = store::write(SAVED, &self.tabsync.saved(&user));
    }
  }

  fn channel_targets(&self) -> Vec<(Spec, Targets)> {
    self.tabs.iter().filter(|t| is_channel(t)).map(|t| (spec_of(t), t.send)).collect()
  }

  // the active tab's send choice changed (ctrl-n, alt-t/k/y): into its entry and up to the account
  pub(super) fn send_targets_up(&mut self) {
    self.tab_sync_push();
  }

  // the account's choices onto the tabs we have, whatever else its frame changed
  fn pull_targets(&mut self) {
    let sync = &self.tabsync;
    for t in self.tabs.iter_mut().filter(|t| is_channel(t)) {
      let got = sync.targets_of(&spec_of(t));
      if got != t.send {
        t.send = got;
        t.target = t.legs().first().copied().unwrap_or(0);
      }
    }
  }

  // the account's list arrived: laid over your tabs unless it is stale or our own echo
  pub(super) fn on_tab_sync(&mut self, f: Frame) {
    let Some(user) = self.tab_sync_user() else { return };
    let Some(want) = self.tabsync.inbound(&user, f) else { return };
    let _ = store::write(SAVED, &self.tabsync.saved(&user));
    self.lay_tabs(&want);
    self.pull_targets();
    self.input_dirty = true;
  }

  // make the channel tabs these, in this order: a tab already open keeps its lines, a new one joins, a gone
  // one leaves. built-in tabs stay where they are; the active tab, tab mode's cursor and an open edit form
  // follow their tab
  fn lay_tabs(&mut self, want: &[Spec]) {
    if self.channel_specs() == want {
      return;
    }
    let active = self.tabs.get(self.active).map(key);
    let rover = self.rover.as_ref().and_then(|r| self.tabs.get(r.at)).map(key);
    let editing = self.form.as_ref().and_then(|f| f.edit).and_then(|i| self.tabs.get(i)).map(key);
    self.close_tab_menu();
    let mut pool = Vec::new();
    let mut i = 0;
    while i < self.tabs.len() {
      if is_channel(&self.tabs[i]) {
        pool.push(self.tabs.remove(i));
      } else {
        i += 1;
      }
    }
    let w = self.body().w;
    let mut laid = Vec::with_capacity(want.len());
    for s in want {
      match pool.iter().position(|t| spec_of(t) == *s) {
        Some(j) => laid.push(pool.remove(j)),
        None => {
          let mut t = make_tab(&self.net, s);
          t.view.resize(w);
          seat_hosts(&mut self.res.chatters, &t.sources);
          laid.push(t);
        }
      }
    }
    // channel tabs sit after the built-in ones, before the test tab
    let at = self.tabs.iter().position(|t| t.kind == Kind::Chat).unwrap_or(self.tabs.len());
    self.tabs.splice(at..at, laid);
    for t in pool {
      if t.under.is_some() {
        self.logs = None;
        self.chain = None;
      }
      for (p, k) in t.sources {
        self.leave(p, k);
      }
    }
    let find = |app: &App, k: Option<Key>| k.and_then(|k| app.tabs.iter().position(|t| key(t) == k));
    let at = find(self, rover).unwrap_or(self.active).min(self.tabs.len().saturating_sub(1));
    if let Some(r) = self.rover.as_mut() {
      r.at = at;
    }
    match (self.form.as_ref().map(|f| (f.edit.is_some(), f.welcome)), find(self, editing)) {
      (Some((true, _)), Some(i)) => self.form.as_mut().into_iter().for_each(|f| f.edit = Some(i)),
      (Some((true, _)), None) => self.close_form(),
      (Some((false, true)), _) if !want.is_empty() => self.close_form(),
      _ => {}
    }
    // the account had channels: the first window no longer asks for one
    self.welcome &= want.is_empty();
    self.store_tabs();
    match find(self, active) {
      Some(i) => self.active = i,
      None => self.activate(self.active.min(self.tabs.len().saturating_sub(1))),
    }
    self.fix_active();
    self.input_dirty = true;
  }
}
