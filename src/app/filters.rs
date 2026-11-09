// the chat filters (ext hideBots / hideCommands / hideDuplicates, keyword hide + highlight, mutes,
// blocks, /hide). the rules live in model::filter; here are the slash commands, the server sync of
// your mutes, and redoing every tab's lines when a rule changes
use crate::i18n::{f, t};
use std::time::{Duration, Instant};

use super::App;
use crate::model::filter::{self, DEFAULT_MUTE_MS, Filters};
use crate::model::filtercmd::{Cmd, RuleCmd, Span};
use crate::model::tabs::Kind;
use crate::net::Cmd as NetCmd;
use crate::net::mutes::{self, Lists, Op, Sync};
use crate::util::time::now_ms;

impl App {
  pub(super) fn push_flags(&self) {
    let s = &self.set;
    filter::edit(|f| (f.bots, f.cmds, f.dupes) = (s.hide_bots, s.hide_cmds, s.hide_dupes));
  }

  // who counts as you: never hidden by these filters
  pub(super) fn filter_names(&mut self) {
    let names: Vec<Box<str>> = self.me.iter().flat_map(|m| m.names.iter().cloned()).collect();
    filter::edit(|f| f.set_me(&names));
    self.refilter_all();
  }

  // every channel tab judges its lines again
  pub(super) fn refilter_all(&mut self) {
    for t in self.tabs.iter_mut().filter(|t| t.kind == Kind::Chat) {
      t.views().for_each(|v| v.refilter());
    }
    self.wake();
  }

  pub(super) fn saved(&mut self) {
    filter::with(Filters::save);
    self.arm_mute_timer();
    self.refilter_all();
  }

  // a timed mute ending changes how lines look: wake then and redo them
  fn arm_mute_timer(&mut self) {
    let now = now_ms();
    self.mute_due = filter::with(|f| f.next_expiry(now))
      .map(|at| Instant::now() + Duration::from_millis((at - now).max(0) as u64 + 50));
  }

  // another login (or none): the old account's server-synced mutes and blocks leave, yours stay
  pub(super) fn filters_login_changed(&mut self) {
    if filter::edit(|f| f.drop_synced()) {
      self.saved();
    }
  }

  pub(super) fn mute_tick(&mut self) {
    self.mute_due = None;
    if filter::edit(|f| f.prune(now_ms())) {
      filter::with(Filters::save);
    }
    self.arm_mute_timer();
    self.refilter_all();
  }

  pub(super) fn filter_cmd(&mut self, cmd: Cmd) {
    match cmd {
      Cmd::Usage(u) => self.say(u),
      Cmd::Mute(who, span) => {
        let until = match span {
          Span::Default => Some(now_ms() + DEFAULT_MUTE_MS),
          Span::For(ms) => Some(now_ms() + ms),
          Span::Forever => None,
        };
        self.mute(&who, until);
      }
      Cmd::Unmute(who) => self.unmute(&who),
      Cmd::Block(who) => {
        if filter::with(|f| f.blocked(&who)) {
          self.unblock(&who);
        } else {
          self.block(&who);
        }
      }
      Cmd::Unblock(who) => self.unblock(&who),
      Cmd::Hide(who, hide) => {
        let changed = self.tabs.get_mut(self.active).is_some_and(|t| t.view.hide_user(&who, hide));
        self.wake();
        self.say(match (changed, hide) {
          (true, true) => f("mod.filter.hid", &[("who", &who)]),
          (true, false) => f("mod.filter.unhid", &[("who", &who)]),
          (false, true) => f("mod.filter.already_hidden", &[("who", &who)]),
          (false, false) => f("mod.filter.not_hidden", &[("who", &who)]),
        });
      }
      Cmd::Reason { add, text } => self.ban_reason(add, text),
      Cmd::Rule(r) => self.rule(r),
      Cmd::Mutes => {
        self.list_people(true, filter::with(|f| f.mutes().into_iter().map(|(n, _)| n.to_owned()).collect()))
      }
      Cmd::Blocks => self.list_blocks(),
    }
  }

  fn list_blocks(&mut self) {
    // a plain list: blocks have no expiry
    let blocks = filter::with(|f| f.blocks_sorted());
    self.list_people(false, blocks);
  }

  fn list_people(&mut self, muted: bool, names: Vec<String>) {
    let list = names.join(", ");
    self.say(match (names.is_empty(), muted) {
      (true, true) => t("mod.filter.nobody_muted").to_owned(),
      (true, false) => t("mod.filter.nobody_blocked").to_owned(),
      (false, true) => f("mod.filter.muted_list", &[("names", &list)]),
      (false, false) => f("mod.filter.blocked_list", &[("names", &list)]),
    });
  }

  pub(super) fn mute(&mut self, who: &str, until: Option<i64>) {
    if !filter::edit(|f| f.mute(who, until)) {
      return self.say(t("mod.filter.mute_failed"));
    }
    let name = filter::key(who);
    if mutes::syncable(&name) && self.me.is_some() {
      self.net.send(NetCmd::Mute(Op::Add { name: name.clone(), until }));
    }
    self.saved();
    self.say(match until {
      None => f("mod.filter.muted", &[("name", &name)]),
      Some(u) => f("mod.filter.muted_for", &[("name", &name), ("span", &span_text((u - now_ms()).max(0) / 1000))]),
    });
  }

  pub(super) fn unmute(&mut self, who: &str) {
    let name = filter::key(who);
    if !filter::edit(|f| f.unmute(&name)) {
      return self.say(f("mod.filter.not_muted", &[("name", &name)]));
    }
    if mutes::syncable(&name) && self.me.is_some() {
      self.net.send(NetCmd::Mute(Op::Remove(name.clone())));
    }
    self.saved();
    self.say(f("mod.filter.unmuted", &[("name", &name)]));
  }

  pub(super) fn block(&mut self, who: &str) {
    let name = filter::key(who);
    if !filter::edit(|f| f.block(&name)) {
      return self.say(f("mod.filter.already_blocked", &[("name", &name)]));
    }
    self.saved();
    self.say(f("mod.filter.blocked", &[("name", &name)]));
  }

  pub(super) fn unblock(&mut self, who: &str) {
    let name = filter::key(who);
    if !filter::edit(|f| f.unblock(&name)) {
      return self.say(f("mod.filter.not_blocked", &[("name", &name)]));
    }
    self.saved();
    self.say(f("mod.filter.unblocked", &[("name", &name)]));
  }

  fn ban_reason(&mut self, add: bool, text: Option<String>) {
    let Some(text) = text else {
      let list = filter::with(|f| f.reasons().iter().map(|p| p.to_string()).collect::<Vec<_>>());
      return self.say(if list.is_empty() {
        t("mod.filter.no_reasons").to_owned()
      } else {
        f("mod.filter.reasons", &[("list", &list.join("  "))])
      });
    };
    if !filter::edit(|f| f.reason(add, &text)) {
      return self.say(if add {
        t("mod.filter.reason_add_failed").to_owned()
      } else {
        f("mod.filter.not_reason", &[("text", &text)])
      });
    }
    self.texts_edited();
    self.saved();
    self.say(f(if add { "mod.filter.reason_added" } else { "mod.filter.reason_removed" }, &[("text", text.trim())]));
  }

  fn rule(&mut self, cmd: RuleCmd) {
    let done = match cmd {
      RuleCmd::List => {
        let rows = filter::with(|f| f.rules().describe());
        return self.say(if rows.is_empty() { t("mod.filter.no_rules").to_owned() } else { rows.join(" · ") });
      }
      RuleCmd::Add { action, ty, value } => {
        let id = format!("{:x}{:x}", now_ms(), filter::with(|f| f.rules().len()));
        match filter::edit(|f| f.rules_mut().add(id, &action, &ty, &value)) {
          Ok(()) => Some(t("mod.filter.rule_added")),
          Err(why) => return self.say(why),
        }
      }
      RuleCmd::Edit { key, action, ty, value } => {
        match filter::edit(|f| f.rules_mut().edit(&key, &action, &ty, &value)) {
          Ok(()) => Some(t("mod.filter.rule_changed")),
          Err(why) => return self.say(why),
        }
      }
      RuleCmd::Notify(k, on) => match filter::edit(|f| f.rules_mut().set_notify(&k, on)) {
        Ok(()) => Some(t(if on { "mod.filter.notify_on" } else { "mod.filter.notify_off" })),
        Err(why) => return self.say(why),
      },
      RuleCmd::Sound(k, what) => match self.rule_sound_set(&k, &what) {
        Ok(said) => Some(said),
        Err(why) => return self.say(why),
      },
      RuleCmd::Volume(k, pct) => match self.rule_volume_set(&k, pct) {
        Ok(said) => Some(said),
        Err(why) => return self.say(why),
      },
      RuleCmd::Del(k) => filter::edit(|f| f.rules_mut().remove(&k)).then_some(t("mod.filter.rule_removed")),
      RuleCmd::Enable(k, on) => filter::edit(|f| f.rules_mut().enable(&k, on)).then_some(t(if on {
        "mod.filter.rule_on"
      } else {
        "mod.filter.rule_off"
      })),
      RuleCmd::Shift(k, up) => filter::edit(|f| f.rules_mut().shift(&k, up)).then_some(t(if up {
        "mod.filter.moved_up"
      } else {
        "mod.filter.moved_down"
      })),
    };
    let Some(done) = done else { return self.say(t("mod.rule.no_such")) };
    self.texts_edited();
    self.saved();
    self.say(done);
  }

  // your lists on heatsync after a login: merged into ours (the ext never removes on a fetch)
  pub(super) fn on_lists(&mut self, l: Lists) {
    let added = filter::edit(|f| {
      let mut n = 0;
      for (name, until) in &l.mutes {
        if !f.muted(name, now_ms()) && f.mute_synced(name, *until) {
          n += 1;
        }
      }
      for b in &l.blocks {
        n += usize::from(f.block_synced(b));
      }
      // a block lifted on another device (`user:unblocked`) leaves too
      if l.blocks_known {
        n += usize::from(f.blocks_follow(&l.blocks));
      }
      n
    });
    if added > 0 {
      self.saved();
    }
  }

  // a mute changed on another device
  pub(super) fn on_mute_sync(&mut self, s: Sync) {
    filter::edit(|f| match s {
      Sync::Added(n, until) => _ = f.mute_synced(&n, until),
      Sync::Removed(n) => _ = f.unmute(&n),
      Sync::Cleared => f.clear_mutes(),
    });
    self.saved();
  }
}

// 90 → "1m", 7200 → "2h": the length left on a mute
pub(super) fn span_text(secs: i64) -> String {
  match secs {
    s if s >= 86400 => format!("{}d", (s + 43200) / 86400),
    s if s >= 3600 => format!("{}h", (s + 1800) / 3600),
    s if s >= 60 => format!("{}m", (s + 30) / 60),
    s => format!("{s}s"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn spans_read_short() {
    assert_eq!(span_text(86400), "1d");
    assert_eq!(span_text(7200), "2h");
    assert_eq!(span_text(600), "10m");
    assert_eq!(span_text(20), "20s");
  }
}
