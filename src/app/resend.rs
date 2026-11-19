// a send that never came back (ext input.js pendingSends). the echoes of your own lines confirm each feed's leg
// (model::pending); twenty seconds on, a send with a leg still unconfirmed gets a notice row in the tabs that
// carry it, and alt-enter sends again only the legs that never confirmed, under the same send id. a heatsync leg
// whose POST may have landed is never posted again
use std::time::{Duration, Instant};

use super::App;
use crate::i18n::{f, t};
use crate::model::message::{Message, Platform};
use crate::net::Cmd;
use crate::util::time::now_ms;

// "twitch, kick"
fn named(legs: &[(Platform, Box<str>)]) -> String {
  let mut names: Vec<&str> = Vec::new();
  for (p, _) in legs {
    if !names.contains(&p.name()) {
      names.push(p.name());
    }
  }
  names.join(", ")
}

impl App {
  // one of your own lines arrived in (p, key): it confirms the leg that sent it
  pub(super) fn leg_echo(&mut self, p: Platform, key: &str, m: &Message) {
    if m.kind.said() && self.mentions.mine(m) {
      self.pending.echo(p, key, &m.body);
    }
  }

  // the moment the oldest send waiting on its echo runs out of time
  pub(super) fn resend_due(&self) -> Option<Instant> {
    let at = self.pending.next_due()?;
    Some(Instant::now() + Duration::from_millis((at - now_ms()).max(0) as u64))
  }

  // sends past the timeout: a notice row in each tab that carries a leg still unconfirmed
  pub(super) fn resend_tick(&mut self, now: i64) {
    for (_, missed) in self.pending.due(now) {
      let key = if missed.retry { "app.resend.missed" } else { "app.resend.kept" };
      let line = f(key, &[("where", &named(&missed.legs))]);
      let mut tabs: Vec<usize> = Vec::new();
      for (p, chan) in &missed.legs {
        for i in self.tabs_with(*p, chan) {
          if !tabs.contains(&i) {
            tabs.push(i);
          }
        }
      }
      for i in tabs {
        self.tabs[i].chat().push(Message::system(now_ms(), missed.legs[0].0, line.clone()));
        self.touched(i);
      }
    }
  }

  // alt-enter: send the latest unconfirmed send's missing legs again. false = nothing waiting on a retry
  pub(super) fn retry_send(&mut self) -> bool {
    let Some(r) = self.pending.retry(now_ms()) else { return false };
    if !self.net.alive() {
      self.say(t("app.input.net_down"));
      return true;
    }
    let sent: Vec<(Platform, Box<str>)> = r.legs.iter().map(|l| (l.platform, l.chan.clone())).collect();
    for l in r.legs {
      self.net.send(Cmd::Send {
        platform: l.platform,
        chan: l.chan,
        text: l.text,
        reply: l.reply,
        send: Some(r.id.clone()),
      });
    }
    if sent.is_empty() {
      self.say(t("app.resend.nothing"));
    } else {
      self.say(f("app.resend.again", &[("where", &named(&sent))]));
    }
    self.wake();
    true
  }
}
