// pronouns beside a twitch name on the user card and the name popup (ext pcApplyPronouns /
// applyTooltipPronouns): asked once someone with a twitch id is on screen, drawn when the answer lands.
// the `pronouns` switch off: nobody is asked and nothing shows
use super::App;
use crate::model::card::Info;
use crate::model::message::Platform;
use crate::net::Cmd;
use crate::util::time::now_ms;

// the twitch id pronouns are looked up by: their twitch line's own, else the one linked to their heatsync
// account (a kick or youtube name with twitch linked)
pub(super) fn twitch_id(platform: Platform, line_id: &str, info: Option<&Info>) -> Option<u64> {
  (platform == Platform::Twitch)
    .then(|| line_id.parse::<u64>().ok())
    .flatten()
    .filter(|n| *n > 0)
    .or_else(|| info.and_then(|i| i.twitch_id))
}

impl App {
  pub(super) fn want_pronouns(&mut self, uid: Option<u64>) {
    if let Some(uid) = uid.filter(|_| self.set.pronouns)
      && self.pronouns.want(uid, now_ms())
    {
      self.net.send(Cmd::Pronouns(uid));
    }
  }

  pub(super) fn pronouns_of(&self, uid: Option<u64>) -> Option<&str> {
    uid.filter(|_| self.set.pronouns).and_then(|u| self.pronouns.get(u, now_ms()))
  }

  pub(super) fn on_pronouns(&mut self, uid: u64, res: Result<Option<Box<str>>, ()>) {
    self.pronouns.put(uid, res, now_ms());
    if self.card_twitch_id() == Some(uid) {
      self.card_changed();
    }
    if self.peek_twitch_id() == Some(uid) {
      self.tip_repaint();
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_line_id_first_then_the_linked_account() {
    let linked = Info { twitch_id: Some(7), ..Info::default() };
    assert_eq!(twitch_id(Platform::Twitch, "42", Some(&linked)), Some(42));
    assert_eq!(twitch_id(Platform::Twitch, "", Some(&linked)), Some(7));
    assert_eq!(twitch_id(Platform::Twitch, "0", None), None);
    assert_eq!(twitch_id(Platform::Twitch, "x1", None), None);
    // a kick line's id is kick's: only the linked twitch account counts
    assert_eq!(twitch_id(Platform::Kick, "42", Some(&linked)), Some(7));
    assert_eq!(twitch_id(Platform::Kick, "42", None), None);
  }
}
