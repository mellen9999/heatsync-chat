// the system clipboard (util/clip.rs: wl-copy / wl-paste, or xclip), run off the ui thread by the net
// thread. paste = ctrl-v or shift-insert; vi yanks and ctrl-c on a selected chat line copy out; a
// picture on the clipboard (and no text) uploads like a dropped file. every failure is a status line
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::message::Message;
use crate::model::vi::Mode;
use crate::net::Cmd;
use crate::util::clip::Paste;

// pasted text as one line: newlines and other control chars become spaces
pub fn flat(s: &str) -> String {
  s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

// what ctrl-c on a selected chat line copies
pub fn line_text(m: &Message) -> String {
  format!("{}: {}", m.nick, m.body)
}

// what copy → input puts in the box: the words, one line, and a space to keep typing after (ext mcQuoteToInput)
pub fn quote_text(m: &Message) -> String {
  format!("{} ", flat(m.body.trim()))
}

pub fn is_paste(key: &Key<&str>, ctrl: bool, shift: bool) -> bool {
  match key {
    Key::Character("v" | "V") => ctrl,
    Key::Named(NamedKey::Insert) => shift && !ctrl,
    _ => false,
  }
}

impl App {
  // text to the clipboard; `done` is said once it is there (empty = nothing said). a failure says why
  pub(super) fn copy(&mut self, text: &str, done: impl Into<String>) {
    if !text.is_empty() {
      self.net.send(Cmd::Copy { text: text.to_owned(), done: done.into() });
    }
  }

  // ctrl-c on a selected chat line
  pub(super) fn copy_selected(&mut self) {
    let Some(m) = self.tabs.get(self.active).and_then(|t| t.view.selected()) else { return };
    let s = line_text(m);
    self.copy(&s, "message copied");
  }

  // copy name: the selected line's sender
  pub(super) fn copy_name(&mut self) {
    let Some(n) = self.tabs.get(self.active).and_then(|t| t.view.selected()).map(|m| m.nick.to_string()) else {
      return;
    };
    self.copy(&n, "name copied");
  }

  // copy → input: the selected line's words into the message box
  pub(super) fn quote_selected(&mut self) {
    let Some(q) = self.tabs.get(self.active).and_then(|t| t.view.selected()).map(quote_text) else { return };
    if !q.trim().is_empty() {
      self.type_into_line(&q);
    }
  }

  // where a paste would land right now: Some(true) the picker's search box, Some(false) the form, prompt
  // or message line, None nowhere
  fn paste_dest(&self) -> Option<bool> {
    let to_picker =
      self.prompt.is_none() && self.picker.as_ref().is_some_and(|p| p.focus == crate::model::picker::Focus::Search);
    let to_line = self.prompt.is_some()
      || self.form.is_some()
      || (self.picker.is_none() && self.menu().is_none() && self.mode == Mode::Insert);
    (to_picker || to_line).then_some(to_picker)
  }

  // ctrl-v / shift-insert. true when the key was a paste and is handled (even if it pastes nothing).
  // the clipboard is read off the ui thread: `on_pasted` gets it
  pub(super) fn paste_key(&mut self, key: &Key<&str>) -> bool {
    if !is_paste(key, self.mods.control_key(), self.mods.shift_key()) {
      return false;
    }
    match self.paste_dest() {
      Some(to_picker) => self.net.send(Cmd::Paste { to_picker }),
      None => return self.picker.is_some() || self.menu().is_some() || self.mode == Mode::Insert,
    }
    true
  }

  // what the clipboard held. dropped if the box it was for has gone since (esc, another screen)
  pub(super) fn on_pasted(&mut self, to_picker: bool, got: Result<Paste, String>) {
    if self.paste_dest() != Some(to_picker) {
      return;
    }
    match got {
      Ok(Paste::Text(t)) => self.paste_text(&t, to_picker),
      Ok(Paste::Image(bytes)) if !to_picker => self.paste_picture(bytes),
      Ok(Paste::Image(_) | Paste::Nothing) => self.say("nothing to paste"),
      Err(e) => self.say(e),
    }
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
      Some(l) => {
        l.insert(t);
        if self.find.on {
          self.find_live();
        }
      }
      None => {
        self.line.checkpoint(false);
        self.line.insert(t);
        self.complete = None;
      }
    }
    self.input_dirty = true;
    self.wake();
  }

  fn paste_picture(&mut self, bytes: Vec<u8>) {
    if !self.upload_ok() {
      return;
    }
    self.uploading = true;
    self.say("uploading pasted picture…");
    self.net.send(Cmd::Picture(bytes));
    self.input_dirty = true;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::net::Net;
  use winit::keyboard::ModifiersState;

  type Rx = tokio::sync::mpsc::UnboundedReceiver<Cmd>;

  fn room() -> (App, Rx) {
    let (net, rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.mode = Mode::Insert;
    app.mods = ModifiersState::CONTROL;
    (app, rx)
  }

  fn said(app: &App) -> String {
    app.tabs[app.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
  }

  fn sent(rx: &mut Rx) -> Vec<Cmd> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter(|c| matches!(c, Cmd::Copy { .. } | Cmd::Paste { .. } | Cmd::Picture(_)))
      .collect()
  }

  #[test]
  fn a_copy_goes_to_the_net_thread_with_its_done_line() {
    let (mut app, mut rx) = room();
    app.copy("hello", "name copied");
    app.copy("", "nothing to copy");
    assert!(matches!(sent(&mut rx).as_slice(), [Cmd::Copy { text, done }] if text == "hello" && done == "name copied"));
  }

  #[test]
  fn ctrl_v_asks_for_the_clipboard_and_the_answer_lands_in_the_line() {
    let (mut app, mut rx) = room();
    assert!(app.paste_key(&Key::Character("v")));
    assert!(matches!(sent(&mut rx).as_slice(), [Cmd::Paste { to_picker: false }]));
    assert_eq!(app.line.text, "", "nothing is typed until the clipboard answers");
    app.on_pasted(false, Ok(Paste::Text("hi there".into())));
    assert_eq!(app.line.text, "hi there");
  }

  #[test]
  fn an_answer_for_a_box_that_has_gone_is_dropped() {
    let (mut app, mut rx) = room();
    assert!(app.paste_key(&Key::Character("v")));
    let _ = sent(&mut rx);
    // esc in between: normal mode takes no paste
    app.mode = Mode::Normal;
    app.on_pasted(false, Ok(Paste::Text("late".into())));
    assert_eq!(app.line.text, "");
    // and a paste meant for the line is not the picker's
    app.mode = Mode::Insert;
    app.on_pasted(true, Ok(Paste::Text("wrong box".into())));
    assert_eq!(app.line.text, "");
  }

  #[test]
  fn a_pasted_picture_uploads_only_when_logged_in() {
    let (mut app, mut rx) = room();
    app.on_pasted(false, Ok(Paste::Image(vec![1, 2, 3])));
    assert_eq!(said(&app), "log in to upload (ctrl-l)");
    assert!(sent(&mut rx).is_empty() && !app.uploading);
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
    app.on_pasted(false, Ok(Paste::Image(vec![1, 2, 3])));
    assert!(matches!(sent(&mut rx).as_slice(), [Cmd::Picture(b)] if b == &[1, 2, 3]));
    assert!(app.uploading);
  }

  #[test]
  fn an_empty_or_failed_clipboard_says_so() {
    let (mut app, _rx) = room();
    app.on_pasted(false, Ok(Paste::Nothing));
    assert_eq!(said(&app), "nothing to paste");
    app.on_pasted(false, Err("pasting needs wl-clipboard: paru -S wl-clipboard".into()));
    assert_eq!(said(&app), "pasting needs wl-clipboard: paru -S wl-clipboard");
    assert_eq!(app.line.text, "");
  }

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

  #[test]
  fn quoting_a_line_is_its_words_flat_with_a_space_after() {
    let mut m = Message::system(0, crate::model::message::Platform::Twitch, "");
    m.body = " hi\nthere  ".into();
    assert_eq!(quote_text(&m), "hi there ");
  }
}
