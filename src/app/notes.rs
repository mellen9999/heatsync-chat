// private notes on people (ext user-notes.js): `/note <user> [text]`, the line menu's add / edit note
// row (types that command so you edit and press enter), and the card's note line. signed in, a note on
// a real heatsync account syncs: the server's note wins when the card opens, a local-only one uploads
// once, a save or delete writes through. the text is never logged
use super::App;
use crate::i18n::{f, t};
use crate::model::card::Info;
use crate::model::message::Platform;
use crate::model::notes::{self, Cmd};
use crate::net::{Cmd as NetCmd, social};

impl App {
  pub(super) fn note_cmd(&mut self, cmd: Cmd) {
    let (platform, login, text) = match cmd {
      Cmd::Set(p, l, t) => (p, l, t),
      Cmd::Del(p, l) => return self.note_delete(p, &l, true),
      Cmd::Usage(u) => return self.say(u),
    };
    let tag = notes::tag(platform, &login);
    let pid = self.card_pid(platform, &login);
    if notes::clean(&text).is_empty() {
      return self.note_delete(platform, &login, false);
    }
    if !self.notes.set(platform, &login, &text, pid) {
      return self.say(t("app.notes.too_many"));
    }
    self.notes.save();
    let n = self.notes.get(platform, &login).cloned();
    if let Some((id, n)) = n.and_then(|n| Some((n.pid.clone()?, n))) {
      self.put_note(platform, &login, id, n.text);
    }
    self.card_changed();
    self.say(f("app.notes.saved", &[("tag", &tag)]));
  }

  // drop a note here and, signed in, on the server (an empty put). `del` = the /delnote wording (ext copy)
  fn note_delete(&mut self, platform: Platform, login: &str, del: bool) {
    let pid = self.card_pid(platform, login);
    let had = self.notes.delete(platform, login);
    self.notes.save();
    let existed = had.is_some();
    // the server keeps its copy until told: clear it too
    if let Some(id) = had.and_then(|n| n.pid).or(pid) {
      self.put_note(platform, login, id, String::new());
    }
    self.card_changed();
    let tag = notes::tag(platform, login);
    self.say(match (existed, del) {
      (true, true) => f("app.notes.removed", &[("tag", &tag)]),
      (false, true) => f("app.notes.none_for", &[("tag", &tag)]),
      (true, false) => t("app.notes.deleted").to_owned(),
      (false, false) => f("app.notes.none_on", &[("tag", &tag)]),
    })
  }

  // the heatsync id their card told us, when it is a real account and we are signed in
  fn card_pid(&self, platform: Platform, login: &str) -> Option<Box<str>> {
    self.me.as_ref()?;
    match self.cards.get(platform, &login.to_ascii_lowercase()) {
      Some(super::card::Load::Done(i)) => i.profile_id.clone(),
      _ => None,
    }
  }

  fn put_note(&mut self, platform: Platform, login: &str, profile_id: Box<str>, text: String) {
    if self.me.is_some() {
      self.net.send(NetCmd::Social(social::Cmd::PutNote { platform, login: login.into(), profile_id, text }));
    }
  }

  // a card answered: the server's note wins, or a local-only one goes up once (ext hsNoteSyncOnOpen)
  pub(super) fn note_sync(&mut self, platform: Platform, login: &str, info: &Info) {
    let (Some(id), true) = (info.profile_id.clone(), self.me.is_some()) else { return };
    let server = notes::clean(info.note.as_deref().unwrap_or(""));
    let local = self.notes.get(platform, login).cloned();
    if !server.is_empty() {
      if local.as_ref().is_none_or(|n| n.text != server || !n.synced) {
        self.notes.adopt(platform, login, &server, &id);
        self.notes.save();
        self.card_changed();
      }
    } else if let Some(n) = local.filter(|n| !n.synced) {
      self.notes.set(platform, login, &n.text, Some(id.clone()));
      self.put_note(platform, login, id, n.text);
    }
  }

  pub(super) fn note_saved(&mut self, platform: Platform, login: &str, text: &str, ok: bool) {
    if ok && self.notes.acked(platform, login, text) {
      self.notes.save();
    }
  }

  // the row's action: the command in the box, the old text after it, to edit
  pub(super) fn note_to_line(&mut self, platform: Platform, login: &str) {
    let old = self.notes.text(platform, login).to_owned();
    self.type_into_line(&format!("/note {} {old}", notes::tag(platform, login)));
  }

  // what the card's note section says: the text, or nothing
  pub(super) fn note_of(&self, platform: Platform, login: &str) -> Option<String> {
    Some(self.notes.text(platform, login)).filter(|t| !t.is_empty()).map(str::to_owned)
  }
}
