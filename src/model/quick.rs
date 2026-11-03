// the ext's small composer commands: /shrug, /tableflip, /unflip add their face to what you say;
// /set <setting> <value> reaches any settings row by its id, command or label
use super::schema::{DEFS, Def};

// `/shrug hi` → `hi ¯\_(ツ)_/¯`; None for anything else
pub fn face(text: &str) -> Option<String> {
  let rest = text.trim_start().strip_prefix('/')?;
  let (word, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
  let face = match word.to_ascii_lowercase().as_str() {
    "shrug" => r"¯\_(ツ)_/¯",
    "tableflip" => "(╯°□°)╯︵ ┻━┻",
    "unflip" => "┬─┬ノ( ゜-゜ノ)",
    _ => return None,
  };
  let arg = arg.trim();
  Some(if arg.is_empty() { face.to_owned() } else { format!("{arg} {face}") })
}

// a /set key: the row's id (`tab_modlog`), its command (`glow`, `/glow`) or its label (`mod log tab`,
// `mod_log_tab`)
pub fn setting(key: &str) -> Option<&'static Def> {
  let k = key.trim().to_ascii_lowercase().replace(['-', ' '], "_");
  let k = k.trim_start_matches('/');
  DEFS.iter().find(|d| {
    d.id == k || d.cmd.is_some_and(|c| c.trim_start_matches('/') == k) || d.label.replace([' ', '-'], "_") == k
  })
}

// on / off as people type it (ext /set's words)
pub fn switch(v: &str) -> Option<bool> {
  match v.trim().to_ascii_lowercase().as_str() {
    "on" | "true" | "1" | "yes" | "enable" | "enabled" | "show" => Some(true),
    "off" | "false" | "0" | "no" | "disable" | "disabled" | "hide" => Some(false),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn faces_and_settings_read_like_the_ext() {
    assert_eq!(face("/shrug").as_deref(), Some(r"¯\_(ツ)_/¯"));
    assert_eq!(face("/tableflip  rage ").as_deref(), Some("rage (╯°□°)╯︵ ┻━┻"));
    assert!(face("/shrugs").is_none() && face("shrug").is_none());
    assert_eq!(setting("tab_modlog").map(|d| d.id), Some("tab_modlog"));
    assert_eq!(setting("Mod-Log-Tab").map(|d| d.id), Some("tab_modlog"));
    assert_eq!(setting("/glow").map(|d| d.id), Some("first_glow"));
    assert!(setting("nope").is_none());
    assert_eq!((switch("ON"), switch("hide"), switch("maybe")), (Some(true), Some(false), None));
  }
}
