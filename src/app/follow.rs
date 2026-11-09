// heatsync follows (ext pcToggleFollow, /follow /unfollow): from the user card's follow row or typed.
// the follow decides whose posts fill your feed; it doesn't follow them on twitch or kick
use super::App;
use super::card::Load;
use crate::i18n::{f, t};
use crate::model::ident;
use crate::model::message::Platform;
use crate::net::Cmd;
use crate::net::social::{self, FollowFail};

impl App {
  // /follow <name>, /unfollow <name>: on this tab's platform (twitch when it has none)
  pub(super) fn follow_slash(&mut self, name: &str, want: bool) {
    let login = name.trim().trim_start_matches('@').to_ascii_lowercase();
    if !ident::twitch_login(&login) {
      return self.say(if want { t("app.follow.usage_follow") } else { t("app.follow.usage_unfollow") });
    }
    let platform = self.tabs.get(self.active).and_then(|t| t.sources.first()).map_or(Platform::Twitch, |s| s.0);
    self.follow(platform, login.into(), want);
  }

  pub(super) fn follow(&mut self, platform: Platform, login: Box<str>, want: bool) {
    if self.me.is_none() {
      return self.say(t("app.login_first_ctrl_l"));
    }
    let profile_id = match self.cards.get(platform, &login) {
      Some(Load::Done(i)) if i.follow == Some(want) => {
        return self.say(f(if want { "app.follow.already" } else { "app.follow.not_following" }, &[("login", &login)]));
      }
      Some(Load::Done(i)) => i.profile_id.clone(),
      _ => None,
    };
    self.net.send(Cmd::Social(social::Cmd::Follow { platform, login, profile_id, want }));
  }

  pub(super) fn on_followed(&mut self, platform: Platform, login: Box<str>, want: bool, res: Result<(), FollowFail>) {
    let line = match res {
      Ok(()) | Err(FollowFail::Already(_)) => {
        if let Some(Load::Done(i)) = self.cards.get(platform, &login) {
          let mut i = i.clone();
          i.follow = Some(want);
          self.cards.put(platform, &login, Ok(i));
          self.card_refresh(platform, &login);
        }
        match (res, want) {
          (Err(_), true) => f("app.follow.already", &[("login", &login)]),
          (Err(_), false) => f("app.follow.not_following", &[("login", &login)]),
          (Ok(()), true) => f("app.follow.done", &[("login", &login)]),
          (Ok(()), false) => f("app.follow.undone", &[("login", &login)]),
        }
      }
      Err(FollowFail::Auth) => t("app.login_first_ctrl_l").into(),
      Err(FollowFail::NotOnHeatsync) => f("app.follow.not_on_heatsync", &[("login", &login)]),
      Err(FollowFail::Limited) => t("app.follow.limited").into(),
      Err(FollowFail::Other) => t("app.follow.other").into(),
    };
    self.say(line);
    self.wake();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::card::Info;
  use crate::net::{Net, NetEvent};

  #[test]
  fn follow_from_the_box_and_the_card_flips_on_the_answer() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let asked = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| -> Vec<String> {
      std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|c| match c {
          Cmd::Social(social::Cmd::Follow { login, profile_id, want, .. }) => {
            Some(format!("{login} {profile_id:?} {want}"))
          }
          _ => None,
        })
        .collect()
    };
    app.follow_slash("bob", true);
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
    app.follow_slash("@Bob", true);
    assert_eq!(asked(&mut rx), ["bob None true"], "unknown: the net looks them up");
    let info = Info { registered: true, profile_id: Some("u42".into()), follow: Some(false), ..Info::default() };
    app.on_net(NetEvent::Card { platform: Platform::Twitch, login: "bob".into(), res: Ok(info) });
    app.follow_slash("bob", false);
    assert!(asked(&mut rx).is_empty(), "already not following: nothing asked");
    app.follow(Platform::Twitch, "bob".into(), true);
    assert_eq!(asked(&mut rx), ["bob Some(\"u42\") true"]);
    app.on_net(NetEvent::Followed { platform: Platform::Twitch, login: "bob".into(), want: true, res: Ok(()) });
    assert!(matches!(app.cards.get(Platform::Twitch, "bob"), Some(Load::Done(i)) if i.follow == Some(true)));
  }
}
