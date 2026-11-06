// the way in and out of your heatsync account by name: /login [platform], /logout and /account. /account is a
// list screen of who you are on each platform, with the next step on a row: log in with a platform while you
// are out, link one you haven't, log out. (acct.rs is the other thing: your settings following the account)
use super::App;
use super::pop::Act;
use super::screen::{Page, Target};
use crate::model::message::Platform;
use crate::net::Cmd;
use crate::net::auth::{Me, Provider};
use crate::ui::sheet::L;

const PROVIDERS: [Provider; 3] = [Provider::Twitch, Provider::Kick, Provider::Youtube];

// what picking a row of the account screen does
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
  Login(Provider),
  Link(Provider),
  Logout,
  // your twitch follows, picked into tabs (what /import does)
  Import,
  // your twitch chat color: types /color ready to finish
  Color,
}

impl Step {
  // the dim line under the list while the row is picked
  pub(super) fn tip(self) -> &'static str {
    match self {
      Step::Login(_) => "opens your browser to log in",
      Step::Link(_) => "opens your browser to link it to this account",
      Step::Import => "lists the twitch channels you follow so you can add them as tabs (same as /import)",
      Step::Color => "types /color for you: a name like coral, or a #hex. twitch turbo or prime may be needed for hex",
      Step::Logout => "logs you out of heatsync on this computer; ctrl-l or /login gets you back in",
    }
  }
}

fn platform(p: Provider) -> Platform {
  match p {
    Provider::Twitch => Platform::Twitch,
    Provider::Kick => Platform::Kick,
    Provider::Youtube => Platform::Youtube,
  }
}

// the screen's rows and what each does. out: one way in per platform. in: your name on the account and on each
// platform (a missing one links), then the way out
pub(super) fn rows(me: Option<&Me>) -> (Vec<L>, Vec<Target>) {
  let (mut lines, mut marks) = (Vec::new(), Vec::new());
  let mut put = |l: L, t: Target| {
    lines.push(l);
    marks.push(t);
  };
  let row = |a: String, b: String| L::Row { a, b, pill: None };
  let Some(me) = me else {
    put(L::Head("not logged in".into()), Target::None);
    for p in PROVIDERS {
      put(row(format!("log in with {}", p.name()), String::new()), Target::Account(Step::Login(p)));
    }
    return (lines, marks);
  };
  put(row("heatsync".into(), me.username.to_string()), Target::None);
  for p in PROVIDERS {
    match me.name_on(platform(p)) {
      Some(name) => put(row(p.name().into(), name.to_owned()), Target::None),
      None => put(row(p.name().into(), "not linked · enter links it".into()), Target::Account(Step::Link(p))),
    }
  }
  put(row("import your twitch follows".into(), "/import".into()), Target::Account(Step::Import));
  if me.twitch.is_some() {
    put(row("chat color…".into(), "/color".into()), Target::Account(Step::Color));
  }
  put(row("log out".into(), "/logout".into()), Target::Account(Step::Logout));
  (lines, marks)
}

impl App {
  // /account
  pub(super) fn account_open(&mut self) {
    self.open_screen(Page::Account);
    self.account_refresh();
  }

  // the screen's rows, read fresh from who is logged in (a login or logout lands while it is open)
  pub(super) fn account_refresh(&mut self) {
    if self.screen.as_ref().is_none_or(|s| s.page() != Page::Account) {
      return;
    }
    let (lines, marks) = rows(self.me.as_ref());
    if let Some(s) = &mut self.screen {
      (s.shown, s.marks) = (lines, marks);
      let (l, _) = s.lines(&self.set);
      s.st.settle(&l);
    }
  }

  // a picked row: the screen steps aside, the browser step or the logout shows in the chat
  pub(super) fn account_step(&mut self, step: Step) {
    self.close_screen();
    match step {
      Step::Import => self.toggle_import(),
      Step::Color => {
        self.type_into_line("/color ");
        self.say("type a color name (coral, red, blue violet…) or a #hex, then enter");
      }
      Step::Login(p) => self.act(Act::Login(p), None, None),
      Step::Link(p) => self.act(Act::Link(p), None, None),
      Step::Logout => self.act(Act::Logout, None, None),
    }
  }

  // /login [platform]: alone it asks which (or opens the account menu when you are in); with a platform it goes
  // straight there: log in, or link it to the account you are in
  pub(super) fn login_cmd(&mut self, arg: &str) {
    let p = match arg.trim() {
      "" => return self.login(),
      "twitch" => Provider::Twitch,
      "kick" => Provider::Kick,
      "youtube" | "yt" => Provider::Youtube,
      _ => return self.say("usage: /login twitch, kick or youtube"),
    };
    let name = self.me.as_ref().map(|me| me.name_on(platform(p)).map(str::to_owned));
    match name {
      None => self.act(Act::Login(p), None, None),
      Some(None) => self.act(Act::Link(p), None, None),
      Some(Some(name)) => self.say(format!("{} is already linked as {name}", p.name())),
    }
  }

  // /logout
  pub(super) fn logout_cmd(&mut self) {
    if self.me.is_none() {
      return self.say("you're not logged in: ctrl-l or /login");
    }
    self.net.send(Cmd::Logout);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn me(twitch: Option<&str>, kick: Option<&str>) -> Me {
    Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: twitch.map(Into::into),
      twitch_id: None,
      names: vec!["mellen".into()],
      kick: kick.map(Into::into),
      youtube: None,
      packs: vec![],
    }
  }

  fn text(lines: &[L]) -> Vec<String> {
    lines
      .iter()
      .map(|l| match l {
        L::Head(h) => format!("# {h}"),
        L::Row { a, b, .. } | L::Tint { a, b, .. } => format!("{a} | {b}"),
      })
      .collect()
  }

  #[test]
  fn out_it_offers_each_platform_to_log_in_with() {
    let (lines, marks) = rows(None);
    assert_eq!(lines.len(), marks.len());
    assert_eq!(text(&lines)[..2], ["# not logged in", "log in with twitch | "]);
    let steps: Vec<Step> =
      marks.iter().filter_map(|t| if let Target::Account(s) = t { Some(*s) } else { None }).collect();
    assert_eq!(steps, PROVIDERS.map(Step::Login));
  }

  #[test]
  fn in_it_names_you_on_each_platform_and_offers_to_link_the_rest_and_to_log_out() {
    let m = me(Some("mellen_tv"), None);
    let (lines, marks) = rows(Some(&m));
    assert_eq!(lines.len(), marks.len());
    let t = text(&lines);
    assert!(t.contains(&"heatsync | mellen".to_owned()) && t.contains(&"twitch | mellen_tv".to_owned()));
    assert!(t.contains(&"kick | not linked · enter links it".to_owned()));
    let steps: Vec<Step> =
      marks.iter().filter_map(|t| if let Target::Account(s) = t { Some(*s) } else { None }).collect();
    assert_eq!(
      steps,
      [Step::Link(Provider::Kick), Step::Link(Provider::Youtube), Step::Import, Step::Color, Step::Logout]
    );
  }

  #[test]
  fn an_account_founded_on_a_platform_is_linked_there_under_its_own_name() {
    let mut m = me(None, None);
    m.platform = "kick".into();
    let (lines, _) = rows(Some(&m));
    assert!(text(&lines).contains(&"kick | mellen".to_owned()), "{:?}", text(&lines));
  }

  // heatsync says the account changed (linked or unlinked elsewhere): it is read again
  #[test]
  fn a_changed_account_is_read_again() {
    let (net, mut rx) = crate::net::Net::tap();
    let mut app = crate::app::App::new(net, vec![], false);
    while rx.try_recv().is_ok() {}
    app.on_net(crate::net::NetEvent::AccountChanged);
    assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, crate::net::Cmd::Recheck)));
  }
}
