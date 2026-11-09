// twitch's mod suite in a twitch tab you moderate: /warn <user> <reason>, /shoutout <user> (/so), /shield
// [on|off]. a line's menu offers warn (the command typed, for the reason) and shoutout. asks for the
// twitch permission when it's missing
use super::App;
use crate::i18n::{f, t};
use crate::model::message::Platform;
use crate::model::modsuite::Cmd;
use crate::net::Cmd as NetCmd;
use crate::net::blocked::Fail;

impl App {
  pub(super) fn suite_slash(&mut self, cmd: Result<Cmd, &'static str>) {
    match (cmd, self.twitch_here()) {
      (Err(usage), _) => self.say(usage),
      (Ok(_), None) => self.say(t("mod.suite.need_tab")),
      (Ok(c), Some(chan)) => self.suite_run(chan, c),
    }
  }

  pub(super) fn suite_run(&mut self, chan: Box<str>, cmd: Cmd) {
    if self.me.is_none() {
      return self.say(t("mod.common.login_first"));
    }
    if !self.can_mod(Platform::Twitch, &chan) {
      return self.say(f("mod.suite.needs_mod", &[("chan", &chan)]));
    }
    self.net.send(NetCmd::Suite(chan, cmd));
  }

  pub(super) fn on_suite(&mut self, chan: Box<str>, cmd: Cmd, res: Result<Option<bool>, Fail>) {
    let what = match &cmd {
      Cmd::Warn { .. } => "mod.suite.noun_warn",
      Cmd::Shoutout(_) => "mod.suite.noun_shoutout",
      Cmd::Shield(_) => "mod.tools.shield",
    };
    let line = match (res, &cmd) {
      (Ok(_), Cmd::Warn { user, .. }) => f("mod.suite.warned", &[("user", user), ("chan", &chan)]),
      (Ok(_), Cmd::Shoutout(user)) => f("mod.suite.shouted", &[("user", user), ("chan", &chan)]),
      (Ok(on), Cmd::Shield(want)) => f(
        if on.or(*want).unwrap_or(false) { "mod.suite.shield_on" } else { "mod.suite.shield_off" },
        &[("chan", &chan)],
      ),
      (Err(Fail::Auth), _) => t("mod.common.login_first").into(),
      (Err(Fail::Relink), _) => {
        self.net.send(NetCmd::Relink("modsuite"));
        super::feed::needs_twitch(t(what))
      }
      (Err(Fail::NotMod), _) => f("mod.suite.not_mod", &[("chan", &chan)]),
      (Err(Fail::Gone), Cmd::Warn { user, .. } | Cmd::Shoutout(user)) => f("mod.suite.no_user", &[("user", user)]),
      (Err(Fail::Limited), _) => t("mod.common.slow_down").into(),
      // twitch refuses a shoutout while offline, or twice inside its cooldown
      (Err(_), Cmd::Shoutout(_)) => t("mod.suite.shout_failed").into(),
      (Err(_), _) => f(
        match &cmd {
          Cmd::Warn { .. } => "mod.suite.warn_failed",
          Cmd::Shoutout(_) => "mod.suite.shout_failed",
          Cmd::Shield(_) => "mod.suite.shield_failed",
        },
        &[],
      ),
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
