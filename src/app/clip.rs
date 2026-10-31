// the system clipboard (arboard: wayland data-control, x11, windows). made on first use, never at
// boot. paste = ctrl-v or shift-insert; vi yanks and ctrl-c on a selected chat line copy out; a
// picture on the clipboard (and no text) uploads like a dropped file. every failure is a status line
use arboard::{Clipboard, Error};
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::message::Message;
use crate::model::vi::Mode;
use crate::net::Cmd;

// bigger than this many pixels is not a chat picture (and would be a huge in-memory copy)
const MAX_PIXELS: usize = 40_000_000;

// pasted text as one line: newlines and other control chars become spaces
pub fn flat(s: &str) -> String {
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

// what ctrl-c on a selected chat line copies
pub fn line_text(m: &Message) -> String {
  format!("{}: {}", m.nick, m.body)
}

pub fn is_paste(key: &Key<&str>, ctrl: bool, shift: bool) -> bool {
  match key {
    Key::Character("v" | "V") => ctrl,
    Key::Named(NamedKey::Insert) => shift && !ctrl,
    _ => false,
  }
}

enum Got {
  Text(String),
  Pixels(u32, u32, Vec<u8>),
  Nothing,
  Failed,
}

impl App {
  // the clipboard, made on first use
  fn clipboard(&mut self) -> Option<&mut Clipboard> {
    if self.clip.is_none() {
      self.clip = Clipboard::new().ok();
    }
    self.clip.as_mut()
  }

  pub(super) fn copy(&mut self, text: &str) {
    if text.is_empty() {
      return;
    }
    let ok = self.clipboard().is_some_and(|c| c.set_text(text).is_ok());
    if !ok {
      self.say("couldn't copy: no clipboard available");
    }
  }

  // ctrl-c on a selected chat line
  pub(super) fn copy_selected(&mut self) {
    let Some(m) = self.tabs.get(self.active).and_then(|t| t.view.selected()) else { return };
    let s = line_text(m);
    self.status.clear();
    self.copy(&s);
    if self.status.is_empty() {
      self.say("copied");
    }
  }

  fn read_clipboard(&mut self) -> Got {
    let Some(c) = self.clipboard() else { return Got::Failed };
    match c.get_text() {
      Ok(t) if !t.trim().is_empty() => return Got::Text(t),
      Ok(_) | Err(Error::ContentNotAvailable) => {}
      Err(_) => return Got::Failed,
    }
    match c.get_image() {
      Ok(i) if i.width * i.height <= MAX_PIXELS => Got::Pixels(i.width as u32, i.height as u32, i.bytes.into_owned()),
      Ok(_) => Got::Failed,
      Err(Error::ContentNotAvailable) => Got::Nothing,
      Err(_) => Got::Failed,
    }
  }

  // ctrl-v / shift-insert. true when the key was a paste and is handled (even if it pasted nothing)
  pub(super) fn paste_key(&mut self, key: &Key<&str>) -> bool {
    if !is_paste(key, self.mods.control_key(), self.mods.shift_key()) {
      return false;
    }
    let to_picker =
      self.prompt.is_none() && self.picker.as_ref().is_some_and(|p| p.focus == crate::model::picker::Focus::Search);
    let to_line = self.prompt.is_some()
      || self.form.is_some()
      || (self.picker.is_none() && self.menu.is_none() && self.mode == Mode::Insert);
    if !to_picker && !to_line {
      return self.picker.is_some() || self.menu.is_some() || self.mode == Mode::Insert;
    }
    self.status.clear();
    match self.read_clipboard() {
      Got::Text(t) => self.paste_text(&t, to_picker),
      Got::Pixels(w, h, px) if !to_picker => self.paste_pixels(w, h, px),
      Got::Pixels(..) | Got::Nothing => self.say("nothing to paste"),
      Got::Failed => self.say("couldn't paste: no clipboard available"),
    }
    true
  }

  fn paste_text(&mut self, t: &str, to_picker: bool) {
    if to_picker {
      if let Some(p) = &mut self.picker {
        p.type_str(&flat(t));
      }
      self.picker_reveal();
      self.picker_changed();
      return;
    }
    if let Some(f) = &mut self.form {
      // one line per field
      f.fields[f.focus].insert(t);
      f.err = None;
      return self.wake();
    }
    match self.prompt.as_mut() {
      Some(l) => l.insert(t),
      None => {
        self.line.insert(t);
        self.complete = None;
      }
    }
    self.input_dirty = true;
    self.wake();
  }

  fn paste_pixels(&mut self, w: u32, h: u32, px: Vec<u8>) {
    if !self.upload_ok() {
      return;
    }
    self.uploading = true;
    self.say("uploading pasted picture…");
    self.net.send(Cmd::Pixels(w, h, px));
    self.input_dirty = true;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn newlines_flatten() {
    assert_eq!(flat("a\nb\r\nc\td"), "a b  c d");
    assert_eq!(flat("plain"), "plain");
  }

  #[test]
  fn paste_keys() {
    assert!(is_paste(&Key::Character("v"), true, false));
    assert!(is_paste(&Key::Character("V"), true, true));
    assert!(!is_paste(&Key::Character("v"), false, false));
    assert!(is_paste(&Key::Named(NamedKey::Insert), false, true));
    assert!(!is_paste(&Key::Named(NamedKey::Insert), false, false));
  }

  #[test]
  fn selected_line_is_nick_colon_text() {
    let mut m = Message::system(0, crate::model::message::Platform::Twitch, "x");
    m.nick = "bob".into();
    m.body = "hi there".into();
    assert_eq!(line_text(&m), "bob: hi there");
  }
}
