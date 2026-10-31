// picture/video upload: ctrl-o or the ↑ button opens a file dialog (zenity/kdialog) or, with none
// installed, a path prompt in the input bar; a dropped file uploads at once. the link lands in the box
use std::path::PathBuf;

use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::line::Line;
use crate::model::pathcomp;
use crate::net::Cmd;
use crate::net::upload::{Upload, expand};
use crate::ui::input_bar;

fn home() -> Option<String> {
  std::env::var("HOME").ok().filter(|h| h.starts_with('/'))
}

impl App {
  pub(super) fn over_upload(&self) -> bool {
    let (x, y) = self.pointer;
    self.input_h > 0 && input_bar::upload_button(self.input_area(), self.res.text.line_px as i32).contains(x, y)
  }

  pub(super) fn say(&mut self, s: impl Into<String>) {
    self.status = s.into();
    self.bar_dirty = true;
    self.wake();
  }

  // can an upload start now? says why not
  pub(super) fn upload_ok(&mut self) -> bool {
    if self.me.is_none() {
      self.say("log in to upload (ctrl-l)");
      return false;
    }
    if self.uploading {
      self.say("one upload at a time");
      return false;
    }
    true
  }

  pub(super) fn pick_file(&mut self) {
    if self.prompt.is_some() {
      return self.close_prompt();
    }
    if !self.upload_ok() {
      return;
    }
    self.uploading = true;
    self.say("choose a picture or video…");
    self.net.send(Cmd::Pick);
    self.input_dirty = true;
  }

  pub(super) fn upload_file(&mut self, path: PathBuf) {
    self.close_prompt();
    if !self.upload_ok() {
      return;
    }
    self.uploading = true;
    self.say("uploading…");
    self.net.send(Cmd::Upload(path));
    self.input_dirty = true;
  }

  pub(super) fn on_upload(&mut self, u: Upload) {
    self.input_dirty = true;
    match u {
      Upload::NoChooser => {
        self.uploading = false;
        self.prompt = Some(Line::default());
        self.say("no file dialog found: type a path");
      }
      Upload::Cancelled => {
        self.uploading = false;
        self.say("");
      }
      Upload::Failed(e) => {
        self.uploading = false;
        self.say(format!("upload failed: {e}"));
      }
      Upload::Url(url) => {
        self.uploading = false;
        self.line.end();
        self.line.insert_word(&url);
        if self.mode != crate::model::vi::Mode::Insert {
          self.enter_insert();
        }
        self.say("upload done");
      }
    }
  }

  fn close_prompt(&mut self) {
    if self.prompt.take().is_some() {
      self.ime.clear();
      self.input_dirty = true;
      self.wake();
    }
  }

  pub(super) fn prompt_key(&mut self, key: Key<&str>, text: Option<&str>) {
    let ctrl = self.mods.control_key();
    let Some(l) = self.prompt.as_mut() else { return };
    match key {
      Key::Named(NamedKey::Escape) => return self.close_prompt(),
      Key::Character("o") if ctrl => return self.close_prompt(),
      Key::Named(NamedKey::Enter) => {
        let path = expand(&l.text, home().as_deref());
        if l.text.trim().is_empty() {
          return self.close_prompt();
        }
        return self.upload_file(path);
      }
      Key::Named(NamedKey::Tab) => {
        let (t, many) = pathcomp::complete(&l.text, home().as_deref());
        l.set_text(t);
        l.end();
        if !many.is_empty() {
          let shown: Vec<&str> = many.iter().take(8).map(String::as_str).collect();
          let more = if many.len() > 8 { format!(" +{}", many.len() - 8) } else { String::new() };
          let s = format!("{}{more}", shown.join("  "));
          self.say(s);
        }
      }
      Key::Named(NamedKey::Backspace) if ctrl => l.delete_word_back(),
      Key::Named(NamedKey::Backspace) => l.backspace(),
      Key::Named(NamedKey::Delete) => l.delete(),
      Key::Named(NamedKey::ArrowLeft) => l.left(),
      Key::Named(NamedKey::ArrowRight) => l.right(),
      Key::Named(NamedKey::Home) => l.home(),
      Key::Named(NamedKey::End) => l.end(),
      Key::Character("a") if ctrl => l.home(),
      Key::Character("e") if ctrl => l.end(),
      Key::Character("w") if ctrl => l.delete_word_back(),
      Key::Character("u") if ctrl => l.clear_to_start(),
      Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !self.mods.alt_key() => {
        if let Some(t) = text.filter(|_| self.ime.accepts_key_text()) {
          l.insert(t);
        }
      }
      _ => return,
    }
    self.input_dirty = true;
    self.wake();
  }
}
