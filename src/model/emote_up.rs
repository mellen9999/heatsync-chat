// the picker's "upload your own emote" form: no drawing. where it is (a dialog is open, a path is being typed,
// a name is being typed, the upload is out) and the one line of text the picker's top row shows while it is
use crate::i18n::t;
use std::path::PathBuf;

use crate::net::upload::emote::Kept;

// the text a path or a name can grow to: a name is ≤100 (`heatsync::MAX_NAME`), a path is cut here
const MAX_TEXT: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
  // the file dialog is open
  Choosing,
  // no dialog on this machine: type where the picture is
  Path,
  Name,
  // the upload (or the add) is out
  Busy,
}

pub struct EmoteUp {
  pub stage: Stage,
  pub text: String,
  // the picked file, once there is one
  pub file: Option<PathBuf>,
  // the upload the server holds while the name is being fixed
  pub kept: Option<Kept>,
}

impl EmoteUp {
  pub fn choosing() -> Self {
    Self { stage: Stage::Choosing, text: String::new(), file: None, kept: None }
  }

  // what the top row says before the text
  pub fn prompt(&self) -> &'static str {
    match self.stage {
      Stage::Choosing => t("chat1.emote_up.choosing"),
      Stage::Path => t("chat1.emote_up.path"),
      Stage::Name if self.kept.is_some() => t("chat1.emote_up.rename"),
      Stage::Name => t("chat1.emote_up.name"),
      Stage::Busy => t("chat1.emote_up.busy"),
    }
  }

  // does the text field take keys now
  pub fn typing(&self) -> bool {
    matches!(self.stage, Stage::Path | Stage::Name)
  }

  pub fn type_str(&mut self, s: &str) {
    if self.typing() {
      let room = MAX_TEXT.saturating_sub(self.text.chars().count());
      self.text.extend(s.chars().filter(|c| !c.is_control()).take(room));
    }
  }

  pub fn backspace(&mut self) {
    self.text.pop();
  }

  pub fn delete_word(&mut self) {
    let t = self.text.trim_end_matches(['/', '\\', ' ']).len();
    self.text.truncate(self.text[..t].rfind(['/', '\\', ' ']).map_or(0, |i| i + 1));
  }

  // the typed path was good: its name is next, the file's own word to start with
  pub fn named(&mut self, file: PathBuf) {
    self.text = crate::net::upload::emote::default_name(&file);
    self.file = Some(file);
    self.stage = Stage::Name;
  }

  // the name is taken: the upload stays up there, another name is one try
  pub fn rename(&mut self, kept: Kept) {
    self.kept = Some(kept);
    self.stage = Stage::Name;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn text_is_taken_only_where_it_is_typed() {
    let mut u = EmoteUp::choosing();
    u.type_str("abc");
    assert!(u.text.is_empty(), "a dialog is open: no typing");
    u.stage = Stage::Path;
    u.type_str("/a\nb");
    assert_eq!(u.text, "/ab", "control bytes are dropped");
    u.stage = Stage::Busy;
    u.type_str("zzz");
    assert_eq!(u.text, "/ab");
  }

  #[test]
  fn text_is_bounded() {
    let mut u = EmoteUp::choosing();
    u.stage = Stage::Path;
    u.type_str(&"a".repeat(MAX_TEXT + 50));
    assert_eq!(u.text.chars().count(), MAX_TEXT);
  }

  #[test]
  fn a_picked_file_offers_its_word_as_the_name() {
    let mut u = EmoteUp::choosing();
    u.named(PathBuf::from("/pics/Pog Champ-2x.png"));
    assert_eq!((u.stage, u.text.as_str()), (Stage::Name, "PogChamp"));
    assert_eq!(u.prompt(), "name your emote: ");
    u.rename(Kept { url: "https://heatsync.org/uploads/a.webp".into(), phash: None, w: 1, h: 1, flagged: vec![] });
    assert_eq!(u.prompt(), "pick another name: ");
  }

  #[test]
  fn ctrl_w_takes_a_path_part() {
    let mut u = EmoteUp::choosing();
    u.stage = Stage::Path;
    u.type_str("/home/me/pics/cat.png");
    u.delete_word();
    assert_eq!(u.text, "/home/me/pics/");
    u.delete_word();
    assert_eq!(u.text, "/home/me/");
  }
}
