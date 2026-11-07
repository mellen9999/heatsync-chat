// twitch's mod suite in a twitch tab you moderate: /warn <user> <reason>, /shoutout <user> (/so), /shield
// [on|off]. a line's menu offers warn (the command typed, for the reason) and shoutout. asks for the
// twitch permission when it's missing
use super::App;
use crate::model::message::Platform;
use crate::model::modsuite::Cmd;
use crate::net::Cmd as NetCmd;
use crate::net::blocked::Fail;

impl App {
  pub(super) fn suite_slash(&mut self, cmd: Result<Cmd, &'static str>) {
    match (cmd, self.twitch_here()) {
      (Err(usage), _) => self.say(usage),
      (Ok(_), None) => self.say("that needs a twitch channel tab"),
      (Ok(c), Some(chan)) => self.suite_run(chan, c),
    }
  }

  pub(super) fn suite_run(&mut self, chan: Box<str>, cmd: Cmd) {
    if self.me.is_none() {
      return self.say("log in first (ctrl-l)");
    }
    if !self.can_mod(Platform::Twitch, &chan) {
      return self.say(format!("that needs mod on {chan}"));
    }
    self.net.send(NetCmd::Suite(chan, cmd));
  }

  pub(super) fn on_suite(&mut self, chan: Box<str>, cmd: Cmd, res: Result<Option<bool>, Fail>) {
    let what = match &cmd {
      Cmd::Warn { .. } => "warn",
      Cmd::Shoutout(_) => "shoutout",
      Cmd::Shield(_) => "shield mode",
    };
    let line = match (res, &cmd) {
      (Ok(_), Cmd::Warn { user, .. }) => format!("warned {user} in {chan}"),
      (Ok(_), Cmd::Shoutout(user)) => format!("shouted out {user} in {chan}"),
      (Ok(on), Cmd::Shield(want)) => {
        format!("shield mode {} in {chan}", if on.or(*want).unwrap_or(false) { "on" } else { "off" })
      }
      (Err(Fail::Auth), _) => "log in first (ctrl-l)".into(),
      (Err(Fail::Relink), _) => {
        self.net.send(NetCmd::Relink("modsuite"));
        super::feed::needs_twitch(what)
      }
      (Err(Fail::NotMod), _) => format!("you are not a moderator of {chan}"),
      (Err(Fail::Gone), Cmd::Warn { user, .. } | Cmd::Shoutout(user)) => format!("no twitch user named {user}"),
      (Err(Fail::Limited), _) => "slow down: try again in a moment".into(),
      // twitch refuses a shoutout while offline, or twice inside its cooldown
      (Err(_), Cmd::Shoutout(_)) => "shoutout failed: the channel must be live, and twitch spaces shoutouts out".into(),
      (Err(_), _) => format!("{what} failed: try again"),
    };
    self.say(line);
    self.wake();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::irc::Chat;
  use crate::net::{Net, NetEvent};

  fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<NetCmd>) -> Vec<String> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| match c {
        NetCmd::Suite(c, cmd) => Some(format!("{c} {cmd:?}")),
        NetCmd::Relink(p) => Some(format!("relink {p}")),
        _ => None,
      })
      .collect()
  }

  #[test]
  fn a_mod_warns_shouts_out_and_shields_from_the_box() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
    let typed = |app: &mut App, s: &str| {
      app.line.set_text(s.into());
      app.send();
    };
    typed(&mut app, "/warn bob spam");
    assert!(asked(&mut rx).is_empty(), "logged out");
    app.me = Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(1),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
    typed(&mut app, "/so bob");
    assert!(asked(&mut rx).is_empty(), "not a mod");
    app.on_chat("forsen", Chat::Mod(true));
    typed(&mut app, "/warn @Bob stop it");
    typed(&mut app, "/so bob");
    typed(&mut app, "/shield");
    assert_eq!(
      asked(&mut rx),
      ["forsen Warn { user: \"bob\", reason: \"stop it\" }", "forsen Shoutout(\"bob\")", "forsen Shield(None)",]
    );
    app.on_net(NetEvent::Suite { chan: "forsen".into(), cmd: Cmd::Shield(None), res: Err(Fail::Relink) });
    assert_eq!(asked(&mut rx), ["relink modsuite"]);
    app.on_net(NetEvent::Suite { chan: "forsen".into(), cmd: Cmd::Shield(None), res: Ok(Some(true)) });
  }
}
