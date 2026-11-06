// the ext's small composer commands: /set, /tab, /user, /lclear (the faces live in model::quick)
use super::App;
use super::pop::Subject;
use crate::model::ident;
use crate::model::message::Platform;
use crate::model::quick;
use crate::model::schema::Ctl;
use crate::model::settings::local_cmd;
use crate::model::tabs::Kind;

impl App {
  // /set <setting> <value>: a switch takes on / off, the rest go through their own command
  pub(super) fn set_cmd(&mut self, key: &str, val: &str) {
    if key.is_empty() || val.is_empty() {
      return self.say("usage: /set <setting> <value>  (e.g. /set timestamps on)");
    }
    let Some(d) = quick::setting(key) else {
      return self.say(format!("no setting called {key}: /settings lists them"));
    };
    match d.ctl {
      Ctl::Pill(..) => match quick::switch(val) {
        Some(on) => {
          self.set_switch(d, on);
          self.say(format!("{} {}", d.label, if on { "on" } else { "off" }));
        }
        None => self.say(format!("use on or off for {}", d.label)),
      },
      Ctl::Words(_) | Ctl::Patterns | Ctl::Rules | Ctl::Reasons | Ctl::People | Ctl::Buttons(..) => match d.cmd {
        Some(c) => self.say(format!("{} is a list: {c} adds to it", d.label)),
        None => self.say(format!("{} is a list: change it in /settings", d.label)),
      },
      _ => match d.cmd.and_then(|c| local_cmd(&format!("{c} {val}"))) {
        Some(cmd) => self.local(cmd),
        None => self.say(format!("change {} in /settings", d.label)),
      },
    }
  }

  // /tab <name>: a built-in tab by its name, or a channel tab by its name or any of its channels
  pub(super) fn goto_tab(&mut self, q: &str) {
    let q = q.trim().trim_start_matches('#').to_ascii_lowercase();
    if q.is_empty() {
      return self.say("usage: /tab <channel|feed|mentions|whispers|modlog>");
    }
    let hit = self
      .tabs
      .iter()
      .position(|t| t.name.eq_ignore_ascii_case(&q) || t.sources.iter().any(|s| s.1.eq_ignore_ascii_case(&q)));
    match hit {
      Some(i) if self.visible(i) => self.select(i),
      Some(_) => self.say(format!("the {q} tab is hidden: /show {q}")),
      None => self.say(format!("no tab called {q}")),
    }
  }

  // /user <name>: their card, on this tab's platform (twitch when it has none)
  pub(super) fn user_cmd(&mut self, name: &str) {
    let login = name.trim().trim_start_matches('@').to_ascii_lowercase();
    if !ident::twitch_login(&login) {
      return self.say("usage: /user <name>");
    }
    let Some(t) = self.tabs.get(self.active) else { return };
    let (platform, chan) = t.sources.first().cloned().unwrap_or((Platform::Twitch, "".into()));
    let user_id = t.view.user_id_of(&login).unwrap_or_default();
    let own = self.me.as_ref().is_some_and(|m| m.names.iter().any(|n| n.eq_ignore_ascii_case(&login)));
    let who = Subject {
      platform,
      chan,
      login: login.as_str().into(),
      nick: login.as_str().into(),
      user_id,
      msg_id: "".into(),
      color: 0,
      own,
      paint: None,
    };
    self.open_card(who);
  }

  // /lclear: this channel tab's lines go, here only
  pub(super) fn lclear(&mut self) {
    match self.tabs.get_mut(self.active) {
      Some(t) if t.kind == Kind::Chat && !t.sources.is_empty() => {
        t.chat().clear();
        self.say("cleared this tab's lines (only for you)");
      }
      _ => self.say("/lclear needs a channel tab"),
    }
  }

  // /shrug hi → hi ¯\_(ツ)_/¯, sent as a chat line
  pub(super) fn faces(&mut self) {
    if let Some(t) = quick::face(&self.line.text) {
      self.line.set_text(t);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::settings::Local;
  use crate::net::Net;

  #[test]
  fn small_commands_do_what_the_ext_does() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.set_cmd("timestamps", "on");
    assert!(app.set.timestamps);
    app.set_cmd("timestamps", "off");
    assert!(!app.set.timestamps);
    app.local(Local::Set("zoom".into(), "150".into()));
    assert_eq!(app.set.zoom, 150);
    app.goto_tab("#FORSEN");
    assert_eq!(app.active, tab);
    app.goto_tab("modlog");
    assert_eq!(app.active, tab, "a hidden tab stays put");
    app.set_cmd("mod log tab", "on");
    app.goto_tab("modlog");
    assert_eq!(app.tabs[app.active].kind, Kind::ModLog);
    app.line.set_text("/tableflip ok".into());
    app.faces();
    assert_eq!(app.line.text, "ok (╯°□°)╯︵ ┻━┻");
  }
}
