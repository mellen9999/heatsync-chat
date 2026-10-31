// user prefs, persisted as `settings` (key=value lines). vi + auto_hide default off: a plain chat box
use crate::store;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
  // vim keys in the message box + chat-driving mode
  pub vi: bool,
  // the input bar takes no room while empty and unused
  pub auto_hide: bool,
  // orange edge on someone's first line of the session. default on, like the ext
  pub first_glow: bool,
}

impl Default for Settings {
  fn default() -> Self {
    Settings { vi: false, auto_hide: false, first_glow: true }
  }
}

impl Settings {
  pub fn parse(s: &str) -> Self {
    let mut out = Self::default();
    for l in s.lines() {
      let Some((k, v)) = l.split_once('=') else { continue };
      let on = v.trim() == "true";
      match k.trim() {
        "vi" => out.vi = on,
        "auto_hide" => out.auto_hide = on,
        "first_glow" => out.first_glow = on,
        _ => {}
      }
    }
    out
  }

  pub fn render(&self) -> String {
    format!("vi={}\nauto_hide={}\nfirst_glow={}\n", self.vi, self.auto_hide, self.first_glow)
  }

  pub fn load() -> Self {
    store::read("settings").map(|s| Self::parse(&s)).unwrap_or_default()
  }

  pub fn save(&self) {
    if let Err(e) = store::write("settings", &self.render()) {
      eprintln!("save settings: {e}");
    }
  }
}

// slash commands handled here, never sent to a platform
#[derive(PartialEq, Eq, Debug)]
pub enum Local {
  Vi,
  AutoHide,
  Glow,
}

pub const LOCAL: [&str; 3] = ["/vi", "/autohide", "/glow"];

pub fn local_cmd(text: &str) -> Option<Local> {
  match text.trim().to_ascii_lowercase().as_str() {
    "/vi" => Some(Local::Vi),
    "/autohide" => Some(Local::AutoHide),
    "/glow" => Some(Local::Glow),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn defaults() {
    assert_eq!(Settings::default(), Settings { vi: false, auto_hide: false, first_glow: true });
    assert_eq!(Settings::parse(""), Settings::default());
    assert_eq!(Settings::parse("garbage\nvi\n=true\nvi=maybe"), Settings::default());
  }

  #[test]
  fn roundtrip() {
    for (vi, auto_hide, first_glow) in
      [(false, false, true), (true, false, false), (false, true, true), (true, true, false)]
    {
      let s = Settings { vi, auto_hide, first_glow };
      assert_eq!(Settings::parse(&s.render()), s);
    }
    // unknown keys and spacing are tolerated
    assert_eq!(Settings::parse("future=1\n vi = true \n"), Settings { vi: true, ..Settings::default() });
  }

  #[test]
  fn slash_commands() {
    assert_eq!(local_cmd("/vi"), Some(Local::Vi));
    assert_eq!(local_cmd("  /AutoHide "), Some(Local::AutoHide));
    assert_eq!(local_cmd("/vi now"), None);
    assert_eq!(local_cmd("vi"), None);
    assert_eq!(local_cmd("/me waves"), None);
  }
}
