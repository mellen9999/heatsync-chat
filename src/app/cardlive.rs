// the open profile card, kept true while it is on screen: a colour, a flair or a follow that changes for the
// person it shows is read again (the card is otherwise a fetch on open). the rows already drawn stay until the
// answer lands, so nothing flickers; a card asked for within the last few seconds is not asked again
use std::time::Duration;

use super::App;
use super::pop::Subject;
use crate::cosmetics::id::PaintId;
use crate::net::{Cmd, social};

// the soonest a card is asked again: a burst of pushes is one request
const GAP: Duration = Duration::from_secs(5);

impl App {
  // ask for the open card again when `concerns` says the push is about its person (given who it is and the
  // twitch id they are known by, when it is known)
  fn card_reask(&mut self, concerns: impl FnOnce(&Subject, Option<u64>) -> bool) {
    let Some(c) = self.card() else { return };
    if !concerns(&c.who, self.card_twitch_id()) {
      return;
    }
    let (platform, login, chan) = (c.who.platform, c.who.login.clone(), c.who.chan.clone());
    if self.cards.age(platform, &login).is_some_and(|a| a < GAP) {
      return;
    }
    self.cards.forget(platform, &login);
    if self.cards.want(platform, &login) {
      let channel = (platform == crate::model::message::Platform::Twitch).then_some(chan);
      self.net.send(Cmd::Social(social::Cmd::Card { platform, login, channel }));
    }
  }

  // `profile:color` named these (lowercase) logins
  pub(super) fn card_names_changed(&mut self, names: &[Box<str>]) {
    self.card_reask(|who, _| names.iter().any(|n| n.eq_ignore_ascii_case(&who.login)));
  }

  // `cosmetic:changed` named these ids
  pub(super) fn card_paints_changed(&mut self, ids: &[PaintId]) {
    self.card_reask(|who, twitch| {
      let mine = PaintId::of(who.platform.name(), &who.user_id).into_iter().chain(twitch.map(PaintId::Twitch));
      mine.into_iter().any(|m| ids.contains(&m))
    });
  }

  // `follows:changed`: whether you follow them may have flipped on another device
  pub(super) fn card_follows_changed(&mut self) {
    self.card_reask(|_, _| true);
  }
}
