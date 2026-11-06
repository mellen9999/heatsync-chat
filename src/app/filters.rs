// the chat filters (ext hideBots / hideCommands / hideDuplicates, keyword hide + highlight, mutes,
// blocks, /hide). the rules live in model::filter; here are the slash commands, the server sync of
// your mutes, and redoing every tab's lines when a rule changes
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
    filter::edit(|f| (f.bots, f.cmds, f.dupes, f.caps) = (s.hide_bots, s.hide_cmds, s.hide_dupes, s.automod_caps));
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
          (true, true) => format!("hid {who} in this tab"),
          (true, false) => format!("{who} is back in this tab"),
          (false, true) => format!("{who} is already hidden here"),
          (false, false) => format!("{who} isn't hidden here"),
        });
      }
      Cmd::Reason { add, text } => self.ban_reason(add, text),
      Cmd::Rule(r) => self.rule(r),
      Cmd::Mutes => {
        self.list_people("muted", filter::with(|f| f.mutes().into_iter().map(|(n, _)| n.to_owned()).collect()))
      }
      Cmd::Blocks => self.list_blocks(),
    }
  }

  fn list_blocks(&mut self) {
    // a plain list: blocks have no expiry
    let blocks = filter::with(|f| f.blocks_sorted());
    self.list_people("blocked", blocks);
  }

  fn list_people(&mut self, what: &str, names: Vec<String>) {
    self.say(if names.is_empty() { format!("nobody {what}") } else { format!("{what}: {}", names.join(", ")) });
  }

  pub(super) fn mute(&mut self, who: &str, until: Option<i64>) {
    if !filter::edit(|f| f.mute(who, until)) {
      return self.say("couldn't mute that name");
    }
    let name = filter::key(who);
    if mutes::syncable(&name) && self.me.is_some() {
      self.net.send(NetCmd::Mute(Op::Add { name: name.clone(), until }));
    }
    self.saved();
    self.say(match until {
      None => format!("muted {name}"),
      Some(u) => format!("muted {name} ({})", span_text((u - now_ms()).max(0) / 1000)),
    });
  }

  pub(super) fn unmute(&mut self, who: &str) {
    let name = filter::key(who);
    if !filter::edit(|f| f.unmute(&name)) {
      return self.say(format!("{name} isn't muted"));
    }
    if mutes::syncable(&name) && self.me.is_some() {
      self.net.send(NetCmd::Mute(Op::Remove(name.clone())));
    }
    self.saved();
    self.say(format!("unmuted {name}"));
  }

  pub(super) fn block(&mut self, who: &str) {
    let name = filter::key(who);
    if !filter::edit(|f| f.block(&name)) {
      return self.say(format!("{name} is already blocked"));
    }
    self.saved();
    self.say(format!("blocked {name}"));
  }

  pub(super) fn unblock(&mut self, who: &str) {
    let name = filter::key(who);
    if !filter::edit(|f| f.unblock(&name)) {
      return self.say(format!("{name} isn't blocked"));
    }
    self.saved();
    self.say(format!("unblocked {name}"));
  }

  fn ban_reason(&mut self, add: bool, text: Option<String>) {
    let Some(text) = text else {
      let list = filter::with(|f| f.reasons().iter().map(|p| p.to_string()).collect::<Vec<_>>());
      return self.say(if list.is_empty() {
        "no ban reasons. /banreason <reason> adds one".to_owned()
      } else {
        format!("ban reasons: {}", list.join("  "))
      });
    };
    if !filter::edit(|f| f.reason(add, &text)) {
      return self.say(if add {
        "couldn't add that reason (empty, a repeat, too long, or too many)".to_owned()
      } else {
        format!("{text} isn't a ban reason")
      });
    }
    self.texts_edited();
    self.saved();
    self.say(format!("{} ban reason {}", if add { "added" } else { "removed" }, text.trim()));
  }

  fn rule(&mut self, cmd: RuleCmd) {
    let done = match cmd {
      RuleCmd::List => {
        let rows = filter::with(|f| f.rules().describe());
        return self.say(if rows.is_empty() {
          "no rules. /rule add <hide|highlight> <keyword|regex|user|badge|msgtype|expr> <value>".to_owned()
        } else {
          rows.join(" · ")
        });
      }
      RuleCmd::Add { action, ty, value } => {
        let id = format!("{:x}{:x}", now_ms(), filter::with(|f| f.rules().len()));
        match filter::edit(|f| f.rules_mut().add(id, &action, &ty, &value)) {
          Ok(()) => Some("added rule"),
          Err(why) => return self.say(why),
        }
      }
      RuleCmd::Del(k) => filter::edit(|f| f.rules_mut().remove(&k)).then_some("removed rule"),
      RuleCmd::Enable(k, on) => {
        filter::edit(|f| f.rules_mut().enable(&k, on)).then_some(if on { "rule on" } else { "rule off" })
      }
      RuleCmd::Shift(k, up) => {
        filter::edit(|f| f.rules_mut().shift(&k, up)).then_some(if up { "moved up" } else { "moved down" })
      }
    };
    let Some(done) = done else { return self.say("no such rule. /rule list shows the numbers") };
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
