// the channel commands in a twitch tab: /announce, /vip /unvip /mod /unmod, /color, /poll /endpoll and
// the prediction verbs (model::chantools). a line's menu offers vip / mod to the broadcaster. the answer
// lands on the status line. asks for the twitch permission when it's missing
use super::App;
use crate::model::chantools::Cmd;
use crate::net::Cmd as NetCmd;
use crate::net::blocked::Fail;
use crate::net::chantools::{Done, Oops};

impl App {
  // you are the owner of this twitch room
  pub(super) fn is_caster(&self, chan: &str) -> bool {
    self.me.as_ref().and_then(|m| m.twitch.as_deref()).is_some_and(|t| t.eq_ignore_ascii_case(chan))
  }

  pub(super) fn chan_tools_slash(&mut self, cmd: Result<Cmd, &'static str>) {
    match cmd {
      Err(usage) => self.say(usage),
      // your color is yours: any tab will do
      Ok(c) if c.roomless() => self.chan_tools_run("".into(), c),
      Ok(c) => match self.twitch_here() {
        Some(chan) => self.chan_tools_run(chan, c),
        None => self.say("that needs a twitch channel tab"),
      },
    }
  }

  pub(super) fn chan_tools_run(&mut self, chan: Box<str>, cmd: Cmd) {
    if self.me.is_none() {
      return self.say("log in first (ctrl-l)");
    }
    if matches!(cmd, Cmd::Announce { .. })
      && !self.can_mod(crate::model::message::Platform::Twitch, &chan)
      && !self.is_caster(&chan)
    {
      return self.say(format!("that needs mod on {chan}"));
    }
    self.net.send(NetCmd::Chan(chan, cmd));
  }

  pub(super) fn on_chan_tools(&mut self, chan: Box<str>, cmd: Cmd, res: Result<Done, Oops>) {
    let what = match &cmd {
      Cmd::Announce { .. } => "announce",
      Cmd::Vip(..) => "vip",
      Cmd::Mod(..) => "mod",
      Cmd::Color(_) => "color",
      Cmd::Poll { .. } => "poll",
      Cmd::EndPoll => "end poll",
      Cmd::Prediction { .. } => "prediction",
      Cmd::Lock => "lock",
      Cmd::Resolve(_) => "resolve",
      Cmd::Cancel => "cancel",
    };
    let line = match (res, &cmd) {
      (Ok(Done::Color(c)), _) => format!("chat color set to {c}"),
      (Ok(Done::Poll(t)), _) => format!("poll started: {t}"),
      (Ok(Done::Ended(t)), _) => format!("poll ended: {t}"),
      (Ok(Done::Predicted(t)), _) => format!("prediction started: {t}"),
      (Ok(Done::Locked(t)), _) => format!("prediction locked: {t}"),
      (Ok(Done::Resolved(t)), _) => format!("prediction resolved: {t} won"),
      (Ok(Done::Canceled(t)), _) => format!("prediction canceled: {t}"),
      (Ok(Done::Plain), Cmd::Vip(u, add)) => {
        format!("{u} {} in {chan}", if *add { "is now a vip" } else { "is no longer a vip" })
      }
      (Ok(Done::Plain), Cmd::Mod(u, add)) => {
        format!("{u} {} in {chan}", if *add { "is now a moderator" } else { "is no longer a moderator" })
      }
      (Ok(Done::Plain), _) => format!("announced in {chan}"),
      (Err(Oops(Fail::Auth, _)), _) => "log in first (ctrl-l)".into(),
      (Err(Oops(Fail::Relink, _)), _) => {
        self.net.send(NetCmd::Relink(cmd.pack()));
        format!("{what} needs a twitch permission. finish allowing it in your browser, then try again")
      }
      (Err(Oops(Fail::NotMod, _)), Cmd::Announce { .. }) => format!("you are not a moderator of {chan}"),
      (Err(Oops(Fail::NotMod, _)), _) => "only the broadcaster can do this".into(),
      (Err(Oops(Fail::Gone, _)), Cmd::Vip(u, _) | Cmd::Mod(u, _)) => format!("no twitch user named {u}"),
      (Err(Oops(Fail::Gone, _)), Cmd::EndPoll) => "no poll is running".into(),
      (Err(Oops(Fail::Gone, _)), Cmd::Lock) => "no prediction is open".into(),
      (Err(Oops(Fail::Gone, _)), Cmd::Resolve(_) | Cmd::Cancel) => "no prediction is open or locked".into(),
      (Err(Oops(Fail::Limited, _)), _) => "slow down: try again in a moment".into(),
      (Err(Oops(_, why)), _) if !why.is_empty() => format!("{what} failed: {why}"),
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
        NetCmd::Chan(c, cmd) => Some(format!("{c} {cmd:?}")),
        NetCmd::Relink(p) => Some(format!("relink {p}")),
        _ => None,
      })
      .collect()
  }

  fn me() -> crate::net::auth::Me {
    crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("forsen".into()),
      twitch_id: Some(1),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    }
  }

  fn typed(app: &mut App, s: &str) {
    app.line.set_text(s.into());
    app.send();
  }

  fn said(app: &App) -> String {
    app.tabs[app.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
  }

  #[test]
  fn the_commands_reach_the_net_with_their_shapes() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
    typed(&mut app, "/vip bob");
    assert!(asked(&mut rx).is_empty(), "logged out");
    app.me = Some(me());
    typed(&mut app, "/vip");
    typed(&mut app, "/poll only a title");
    assert!(asked(&mut rx).is_empty(), "bad parses never leave");
    // the owner of the room announces without the mod flag; the roles and polls are the server's call
    typed(&mut app, "/announceblue hello");
    typed(&mut app, "/unvip @Bob");
    typed(&mut app, "/mod bob");
    typed(&mut app, "/color orangered");
    typed(&mut app, "/poll best | a | b | 60");
    typed(&mut app, "/prediction win? | yes | no");
    typed(&mut app, "/endpoll");
    typed(&mut app, "/lockprediction");
    typed(&mut app, "/resolveprediction 2");
    typed(&mut app, "/cancelprediction");
    assert_eq!(
      asked(&mut rx),
      [
        "forsen Announce { color: \"blue\", text: \"hello\" }",
        "forsen Vip(\"bob\", false)",
        "forsen Mod(\"bob\", true)",
        " Color(\"orange_red\")",
        "forsen Poll { title: \"best\", choices: [\"a\", \"b\"], secs: 60 }",
        "forsen Prediction { title: \"win?\", outcomes: [\"yes\", \"no\"], secs: 120 }",
        "forsen EndPoll",
        "forsen Lock",
        "forsen Resolve(2)",
        "forsen Cancel",
      ]
    );
  }

  #[test]
  fn announce_needs_mod_and_a_twitch_tab() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["xqc".into()], false);
    let tab = app.tabs.iter().position(|t| &*t.name == "xqc").unwrap();
    app.select(tab);
    app.me = Some(me());
    typed(&mut app, "/announce hi");
    assert!(asked(&mut rx).is_empty(), "not a mod in xqc");
    app.on_chat("xqc", Chat::Mod(true));
    typed(&mut app, "/announce hi");
    assert_eq!(asked(&mut rx), ["xqc Announce { color: \"primary\", text: \"hi\" }"]);
  }

  #[test]
  fn a_missing_permission_asks_for_the_right_pack() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let relink = |app: &mut App, cmd: Cmd| {
      app.on_net(NetEvent::Chan { chan: "forsen".into(), cmd, res: Err(Oops(Fail::Relink, "".into())) })
    };
    relink(&mut app, Cmd::Color("red".into()));
    relink(&mut app, Cmd::Vip("bob".into(), true));
    relink(&mut app, Cmd::Announce { color: "primary", text: "x".into() });
    relink(&mut app, Cmd::EndPoll);
    assert_eq!(asked(&mut rx), ["relink chatcolor", "relink channeltools", "relink announce", "relink channeltools"]);
  }

  #[test]
  fn answers_read_plainly() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
    let go = |app: &mut App, cmd: Cmd, res: Result<Done, Oops>| {
      app.on_net(NetEvent::Chan { chan: "forsen".into(), cmd, res });
      said(app)
    };
    assert_eq!(go(&mut app, Cmd::Vip("bob".into(), true), Ok(Done::Plain)), "bob is now a vip in forsen");
    assert_eq!(go(&mut app, Cmd::Mod("bob".into(), false), Ok(Done::Plain)), "bob is no longer a moderator in forsen");
    assert_eq!(
      go(&mut app, Cmd::Vip("bob".into(), true), Err(Fail::NotMod.into())),
      "only the broadcaster can do this"
    );
    assert_eq!(go(&mut app, Cmd::EndPoll, Err(Fail::Gone.into())), "no poll is running");
    assert_eq!(
      go(&mut app, Cmd::Color("#ff8800".into()), Err(Oops(Fail::Other, "needs turbo or prime".into()))),
      "color failed: needs turbo or prime"
    );
    assert_eq!(go(&mut app, Cmd::Color("red".into()), Ok(Done::Color("#ff0000".into()))), "chat color set to #ff0000");
  }

  #[test]
  fn the_broadcaster_is_you_in_your_own_room() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    assert!(!app.is_caster("forsen"));
    app.me = Some(me());
    assert!(app.is_caster("Forsen") && !app.is_caster("xqc"));
  }
}
