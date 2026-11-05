// the user card's `clip` row (ext profile-card.js pcDoClip, hotkey c): shown while the card's person is
// live on twitch. press: heatsync makes a clip of the channel (`clipping…`), its link goes to the
// clipboard (`url copied`); press again and the edit page opens. the card stays up the whole time
use super::App;
use super::card::Load;
use super::pop::Act;
use crate::net::Cmd;
use crate::net::twclip::{Fail, Made};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Clip {
  #[default]
  Idle,
  Busy,
  Made(Made),
}

impl Clip {
  // the row's words
  pub(super) fn label(&self) -> &'static str {
    match self {
      Clip::Idle => "clip",
      Clip::Busy => "clipping\u{2026}",
      Clip::Made(m) if m.clip_url.is_some() => "url copied",
      Clip::Made(_) => "clip created",
    }
  }
}

impl App {
  // the row, when the card's person is a live twitch channel
  pub(super) fn clip_row(&self, who: &super::pop::Subject, info: &Load, clip: &Clip) -> Option<(Act, String)> {
    let live = matches!(info, Load::Done(i) if i.live);
    (live && who.platform == crate::model::message::Platform::Twitch).then(|| (Act::Clip, clip.label().to_owned()))
  }

  // enter on the row, or `c`: make a clip; once made, open its edit page (else copy its link again)
  pub(super) fn clip_press(&mut self) {
    let Some(c) = self.card() else { return };
    if !c.menu.items.iter().any(|r| r.0 == Act::Clip) {
      return;
    }
    match c.clip.clone() {
      Clip::Busy => {}
      Clip::Made(m) => match (m.edit_url, m.clip_url) {
        (Some(edit), _) => {
          if let Err(e) = crate::util::open::open_url(&edit) {
            self.say(e);
          }
        }
        (None, Some(url)) => self.net.send(Cmd::Copy { text: url.into(), done: "url copied".into() }),
        (None, None) => {}
      },
      Clip::Idle => {
        if self.me.is_none() {
          return self.say("log in first (ctrl-l)");
        }
        let (login, chan) = (c.who.login.clone(), c.who.chan.clone());
        // the ext clips the channel the card was opened in; a card from nowhere clips the person's own
        let chan = if chan.is_empty() { login.clone() } else { chan };
        self.card_clip_set(Clip::Busy);
        self.net.send(Cmd::Clip(login, chan));
      }
    }
  }

  fn card_clip_set(&mut self, clip: Clip) {
    if let Some(c) = self.card_mut() {
      c.clip = clip;
    }
    self.card_rows_refresh();
  }

  pub(super) fn on_clip(&mut self, login: &str, res: Result<Made, Fail>) {
    // the card may have moved on or closed; only the card of this person takes the answer
    let here = self.card().is_some_and(|c| *c.who.login == *login && c.clip == Clip::Busy);
    match res {
      Ok(m) => {
        match &m.clip_url {
          Some(url) => self.net.send(Cmd::Copy { text: url.to_string(), done: "url copied".into() }),
          None => self.say("clip created"),
        }
        if here {
          self.card_clip_set(Clip::Made(m));
        }
      }
      Err(f) => {
        if here {
          self.card_clip_set(Clip::Idle);
        }
        match f {
          Fail::Auth => self.say("log in first (ctrl-l)"),
          Fail::Relink => {
            self.say("clipping needs a twitch permission. finish allowing it in your browser\u{2026}");
            self.net.send(Cmd::Relink("clips"));
          }
          Fail::Other(e) => self.say(format!("clip failed: {e}")),
        }
      }
    }
    self.wake();
  }
}
