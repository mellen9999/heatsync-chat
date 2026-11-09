// upload your own emote from the picker (alt-u): a file dialog, a name, one upload. the form lives on the
// picker's top row (`model::emote_up`); the grid then shows the emote at once. the net side is `net::upload::emote`
use crate::i18n::{f, t};
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::emote_up::{EmoteUp, Stage};
use crate::model::picker::{Focus, Item, Sec, Tab};
use crate::net::Cmd;
use crate::net::emote_ops::EmoteCmd;
use crate::net::upload::emote::{self, Done, Failed};
use crate::net::upload::{Upload, expand};

fn home() -> Option<String> {
  crate::util::dirs::home().and_then(|h| h.into_os_string().into_string().ok())
}

impl App {
  fn up_stage(&self) -> Option<Stage> {
    self.picker.as_ref()?.up.as_ref().map(|u| u.stage)
  }

  fn up_note(&mut self, note: impl Into<String>) {
    if let Some(p) = &mut self.picker {
      p.set_note(note.into());
    }
    self.picker_changed();
  }

  // alt-u in the picker: the file dialog (or, with none on this machine, a path to type)
  pub(super) fn emote_up_start(&mut self) {
    if self.up_stage().is_some() {
      return;
    }
    if self.picker.as_ref().is_some_and(|p| p.tab != Tab::Emotes) {
      self.picker_tab(Some(Tab::Emotes));
    }
    if self.me.is_none() {
      return self.up_note(t("app.emote_up.login"));
    }
    if self.uploading {
      return self.up_note(t("app.upload.one_at_a_time"));
    }
    self.uploading = true;
    if let Some(p) = &mut self.picker {
      p.up = Some(EmoteUp::choosing());
      p.focus = Focus::Search;
      p.set_note(String::new());
    }
    self.net.send(Cmd::Emote(EmoteCmd::PickFile));
    self.picker_changed();
  }

  // the form's keys: type, enter goes on, esc backs out (the picker itself stays). false = not the form's
  pub(super) fn emote_up_key(&mut self, key: &Key<&str>, text: Option<&str>) -> bool {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    if self.up_stage().is_none() {
      return false;
    }
    match key {
      Key::Character(";") if ctrl => return false,
      Key::Named(NamedKey::Escape) => self.emote_up_cancel(),
      Key::Named(NamedKey::Enter) => self.emote_up_enter(),
      Key::Named(NamedKey::Backspace) if ctrl => self.up_edit(|u| u.delete_word()),
      Key::Named(NamedKey::Backspace) => self.up_edit(|u| u.backspace()),
      Key::Character("w") if ctrl => self.up_edit(|u| u.delete_word()),
      Key::Character("u") if ctrl => self.up_edit(|u| u.text.clear()),
      Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
        // mid-composition the input method owns the text; its commit arrives as an ime event
        if let Some(t) = text.filter(|_| self.ime.accepts_key_text()) {
          let t = t.to_owned();
          self.up_edit(|u| u.type_str(&t));
        }
      }
      // every other key is the form's too: nothing else may act on a picker that is waiting on a name
      _ => {}
    }
    true
  }

  fn up_edit(&mut self, f: impl FnOnce(&mut EmoteUp)) {
    if let Some(u) = self.picker.as_mut().and_then(|p| p.up.as_mut()) {
      f(u);
    }
    self.up_note(String::new());
  }

  // esc: the form closes. a dialog or an upload already out can't be taken back, so esc waits for its answer
  fn emote_up_cancel(&mut self) {
    if !matches!(self.up_stage(), Some(Stage::Path | Stage::Name)) {
      return;
    }
    if let Some(p) = &mut self.picker {
      p.up = None;
    }
    self.uploading = false;
    self.up_note(String::new());
  }

  fn emote_up_enter(&mut self) {
    let Some(u) = self.picker.as_mut().and_then(|p| p.up.as_mut()) else { return };
    match u.stage {
      Stage::Path => {
        let path = expand(&u.text, home().as_deref());
        if u.text.trim().is_empty() {
          return self.emote_up_cancel();
        }
        u.named(path);
        self.uploading = false;
        self.up_note(String::new());
      }
      Stage::Name => {
        let name = u.text.trim().to_owned();
        if let Some(why) = emote::name_problem(&name) {
          return self.up_note(why);
        }
        let cmd = match (u.kept.clone(), u.file.clone()) {
          (Some(kept), _) => EmoteCmd::AddKept { kept, name },
          (None, Some(path)) => EmoteCmd::Upload { path, name },
          (None, None) => return self.emote_up_cancel(),
        };
        u.stage = Stage::Busy;
        self.uploading = true;
        self.net.send(Cmd::Emote(cmd));
        self.up_note(String::new());
      }
      Stage::Choosing | Stage::Busy => {}
    }
  }

  // what the net thread says about the form. true = it was one of the emote upload's events
  pub(super) fn emote_up_event(&mut self, u: &Upload) -> bool {
    match u {
      Upload::EmotePicked(path) => self.emote_picked(path.clone()),
      // the dialog closed or none exists: only the form's own dialog is answered here
      Upload::Cancelled if self.up_stage() == Some(Stage::Choosing) => {
        self.uploading = false;
        if let Some(p) = &mut self.picker {
          p.up = None;
        }
        self.up_note(String::new());
      }
      Upload::NoChooser if self.up_stage() == Some(Stage::Choosing) => {
        self.uploading = false;
        if let Some(up) = self.picker.as_mut().and_then(|p| p.up.as_mut()) {
          up.stage = Stage::Path;
        }
        self.up_note(t("app.emote_up.no_dialog"));
      }
      Upload::EmoteDone(d) => self.emote_done(d),
      Upload::EmoteFailed(f) => self.emote_failed(f),
      _ => return false,
    }
    self.input_dirty = true;
    true
  }

  fn emote_picked(&mut self, path: std::path::PathBuf) {
    self.uploading = false;
    match self.picker.as_mut().and_then(|p| p.up.as_mut()).filter(|u| u.stage == Stage::Choosing) {
      Some(u) => u.named(path),
      // esc'd meanwhile: nothing is waiting on this file
      None => return,
    }
    self.up_note(String::new());
  }

  fn emote_done(&mut self, d: &Done) {
    self.uploading = false;
    let note = if d.kept.flagged.is_empty() {
      f("app.emote_up.added", &[("name", &d.name)])
    } else {
      f("app.emote_up.added_flagged", &[("name", &d.name), ("flags", &d.kept.flagged.join(", "))])
    };
    let Some(p) = &mut self.picker else { return self.say(note) };
    p.up = None;
    p.add_mine(Item {
      sec: Sec::Mine,
      name: d.name.as_str().into(),
      url: d.kept.url.as_str().into(),
      w: d.kept.w,
      h: d.kept.h,
    });
    p.focus = Focus::Grid;
    self.picker_reveal();
    self.picker_changed();
    self.up_note(note);
  }

  fn emote_failed(&mut self, f: &Failed) {
    self.uploading = false;
    let told = crate::i18n::f("app.emote_up.failed", &[("why", &f.why.to_string())]);
    let Some(p) = &mut self.picker else { return self.say(told) };
    let Some(u) = p.up.as_mut() else { return self.say(told) };
    match &f.kept {
      // the file is up and the name is taken: ask for another
      Some(k) => u.rename(k.clone()),
      None => p.up = None,
    }
    self.up_note(f.why.clone());
  }
}
