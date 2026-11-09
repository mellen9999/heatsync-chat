// picture/video upload: ctrl-o or the + button opens a file dialog (zenity/kdialog) or, with none
// installed, a path prompt in the input bar; a dropped file uploads at once. the link lands in the box
use crate::i18n::{f, t};
use std::path::PathBuf;

use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::line::Line;
use crate::model::message::{Message, Platform};
use crate::net::Cmd;
use crate::net::upload::{Upload, expand};
use crate::ui::input_bar;
use crate::util::time::now_ms;

fn home() -> Option<String> {
  crate::util::dirs::home().and_then(|h| h.into_os_string().into_string().ok())
}

impl App {
  pub(super) fn over_upload(&self) -> bool {
    let (x, y) = self.pointer;
    self.input_h > 0 && input_bar::upload_button(self.input_area(), self.res.text.line_px as i32).contains(x, y)
  }

  // a dim line in the chat on screen: what just happened, or why it didn't
  pub(super) fn say(&mut self, s: impl Into<String>) {
    let s = s.into();
    let Some(t) = self.tabs.get_mut(self.active).filter(|_| !s.is_empty()) else { return };
    t.view.push(Message::system(now_ms(), Platform::Twitch, s));
    self.wake();
  }

  // can an upload start now? says why not
  pub(super) fn upload_ok(&mut self) -> bool {
    if self.me.is_none() {
      self.say(t("app.upload.login"));
      return false;
    }
    if self.uploading {
      self.say(t("app.upload.one_at_a_time"));
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
    self.say(t("app.upload.choose"));
    self.net.send(Cmd::Pick);
    self.input_dirty = true;
  }

  pub(super) fn upload_file(&mut self, path: PathBuf) {
    self.close_prompt();
    if !self.upload_ok() {
      return;
    }
    self.uploading = true;
    self.say(t("app.upload.uploading"));
    self.net.send(Cmd::Upload(path));
    self.input_dirty = true;
  }

  // a folder listing is back: it counts only if it is for the newest tab press and for what is still typed
  fn on_completed(&mut self, seq: u64, input: &str, text: String, many: &[String]) {
    if seq != self.path_seq {
      return;
    }
    let Some(l) = self.prompt.as_mut().filter(|l| &*l.text == input) else { return };
    l.set_text(text);
    l.end();
    if !many.is_empty() {
      let shown: Vec<&str> = many.iter().take(8).map(String::as_str).collect();
      let more = if many.len() > 8 { format!(" +{}", many.len() - 8) } else { String::new() };
      self.say(format!("{}{more}", shown.join("  ")));
    }
  }

  pub(super) fn on_upload(&mut self, u: Upload) {
    self.input_dirty = true;
    if self.emote_up_event(&u) {
      return;
    }
    match u {
      Upload::NoChooser => {
        self.uploading = false;
        self.prompt = Some(Line::default());
        self.say(t("app.upload.no_dialog"));
      }
      Upload::Cancelled => {
        self.uploading = false;
        self.say("");
      }
      Upload::Completed { seq, input, text, many } => self.on_completed(seq, &input, text, &many),
      Upload::Failed(e) => {
        self.uploading = false;
        self.say(f("app.upload.failed", &[("why", &e.to_string())]));
      }
      Upload::EmotePicked(_) | Upload::EmoteDone(_) | Upload::EmoteFailed(_) => {}
      Upload::Url(url) => {
        self.uploading = false;
        self.line.end();
        self.line.insert_word(&url);
        if self.mode != crate::model::vi::Mode::Insert {
          self.enter_insert();
        }
        self.say(t("app.upload.done"));
      }
    }
  }

  pub(super) fn close_prompt(&mut self) {
    if self.prompt.take().is_some() {
      self.find.on = false;
      self.ime.clear();
      self.input_dirty = true;
      self.wake();
    }
  }

  pub(super) fn prompt_key(&mut self, key: Key<&str>, text: Option<&str>) {
    if self.find.on {
      return self.find_key(key, text);
    }
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
        // the folder is read off the ui thread; the answer lands in `on_completed`
        self.path_seq += 1;
        let ask = Cmd::Complete { seq: self.path_seq, input: l.text.to_string(), home: home() };
        self.net.send(ask);
        return;
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
