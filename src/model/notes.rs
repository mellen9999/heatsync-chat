// private notes on people (ext user-notes.js): `platform:login` (lowercase) -> up to 2000 chars, kept in
// one small file. `/note <user|kick:user|youtube:user> [text]` sets one, no text deletes it. a note
// also remembers the heatsync profile id the card gave and whether the server has this version, so a
// signed-in card can sync it (server note wins, a local-only one uploads once)
use std::collections::BTreeMap;

use crate::model::message::Platform;
use crate::model::modcmd::name;

pub const MAX_CHARS: usize = 2000;
// notes kept; the file stays a few MB at the worst
const MAX_NOTES: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
  pub text: String,
  // the server holds exactly this text
  pub synced: bool,
  // the heatsync user id this person's card named (never a kick_ / yt_ shadow id)
  pub pid: Option<Box<str>>,
}

#[derive(Default, Debug)]
pub struct Notes {
  by: BTreeMap<String, Note>,
}

pub fn key(p: Platform, login: &str) -> String {
  format!("{}:{}", p.name(), login.trim().to_ascii_lowercase())
}

// one line of one note: no control chars (a pasted newline would break the file), cut to the cap
pub fn clean(s: &str) -> String {
  let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
  // counted the way the server counts (js string length, utf-16 units), so an emoji-heavy note is never refused
  let mut units = 0;
  flat
    .trim()
    .chars()
    .take_while(|c| {
      units += c.len_utf16();
      units <= MAX_CHARS
    })
    .collect()
}

// a heatsync id the server keeps notes for: not the kick_ / yt_ ids made up for people without an account
pub fn server_id(id: &str) -> bool {
  !id.is_empty() && id.len() <= 64 && !id.starts_with("kick_") && !id.starts_with("yt_")
}

impl Notes {
  pub fn get(&self, p: Platform, login: &str) -> Option<&Note> {
    self.by.get(&key(p, login))
  }

  pub fn text(&self, p: Platform, login: &str) -> &str {
    self.get(p, login).map_or("", |n| &n.text)
  }

  // false = refused (the file is full of other people's notes)
  pub fn set(&mut self, p: Platform, login: &str, text: &str, pid: Option<Box<str>>) -> bool {
    let (k, text) = (key(p, login), clean(text));
    if text.is_empty() {
      self.by.remove(&k);
      return true;
    }
    if self.by.len() >= MAX_NOTES && !self.by.contains_key(&k) {
      return false;
    }
    let pid = pid.or_else(|| self.by.get(&k).and_then(|n| n.pid.clone()));
    self.by.insert(k, Note { text, synced: false, pid });
    true
  }

  // the server said this is the note (or acked our upload)
  pub fn adopt(&mut self, p: Platform, login: &str, text: &str, pid: &str) {
    let text = clean(text);
    if !text.is_empty() && (self.by.len() < MAX_NOTES || self.by.contains_key(&key(p, login))) {
      self.by.insert(key(p, login), Note { text, synced: true, pid: server_id(pid).then(|| pid.into()) });
    }
  }

  // an upload of `text` landed: synced, unless the note was edited meanwhile
  pub fn acked(&mut self, p: Platform, login: &str, text: &str) -> bool {
    match self.by.get_mut(&key(p, login)) {
      Some(n) if n.text == text && !n.synced => {
        n.synced = true;
        true
      }
      _ => false,
    }
  }

  pub fn delete(&mut self, p: Platform, login: &str) -> Option<Note> {
    self.by.remove(&key(p, login))
  }

  #[cfg(test)]
  pub fn len(&self) -> usize {
    self.by.len()
  }

  // `key \t synced(0|1) \t pid \t text`, backslash and newline escaped by `clean` having removed them
  pub fn render(&self) -> String {
    let mut s = String::new();
    for (k, n) in &self.by {
      s.push_str(&format!("{k}\t{}\t{}\t{}\n", u8::from(n.synced), n.pid.as_deref().unwrap_or(""), n.text));
    }
    s
  }

  // tolerant: a bad line is skipped, never an error
  pub fn parse(s: &str) -> Self {
    let mut by = BTreeMap::new();
    for line in s.lines() {
      let mut f = line.splitn(4, '\t');
      let (Some(k), Some(sync), Some(pid), Some(text)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
      let Some((plat, login)) = k.split_once(':') else { continue };
      let ok = matches!(plat, "twitch" | "kick" | "youtube")
        && !login.is_empty()
        && login.len() <= 64
        && login.chars().all(|c| !c.is_control() && !c.is_uppercase());
      let text = clean(text);
      if !ok || text.is_empty() || by.len() >= MAX_NOTES {
        continue;
      }
      let pid = (server_id(pid)).then(|| pid.into());
      by.insert(k.to_owned(), Note { text, synced: sync == "1" && pid.is_some(), pid });
    }
    Notes { by }
  }

  pub fn load() -> Self {
    crate::store::read("notes").map(|s| Self::parse(&s)).unwrap_or_default()
  }

  pub fn save(&self) {
    if let Err(e) = crate::store::write("notes", &self.render()) {
      // never the text: only that it failed
      eprintln!("save notes: {e}");
    }
  }
}

#[derive(PartialEq, Eq, Debug)]
pub enum Cmd {
  // empty text = delete
  Set(Platform, String, String),
  Usage(&'static str),
}

const USAGE: &str = "usage: /note <user> [text] (kick:user, youtube:user). no text deletes it";

// `bob`, `kick:bob`, `twitch:bob`, `youtube:bob` / `yt:bob`; bare = twitch
pub fn who(w: &str) -> Option<(Platform, String)> {
  let (p, l) = match w.split_once(':') {
    Some(("kick", l)) => (Platform::Kick, l),
    Some(("youtube" | "yt", l)) => (Platform::Youtube, l),
    Some(("twitch", l)) => (Platform::Twitch, l),
    Some(_) => return None,
    None => (Platform::Twitch, w),
  };
  name(l).map(|l| (p, l))
}

// what the box writes for the menu row: bob, or kick:bob
pub fn tag(p: Platform, login: &str) -> String {
  match p {
    Platform::Twitch => login.to_owned(),
    Platform::Kick => format!("kick:{login}"),
    Platform::Youtube => format!("youtube:{login}"),
  }
}

pub fn command(text: &str) -> Option<Cmd> {
  let t = text.trim();
  let (cmd, rest) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
  if !cmd.eq_ignore_ascii_case("/note") {
    return None;
  }
  let (first, tail) = rest.trim().split_once(char::is_whitespace).unwrap_or((rest.trim(), ""));
  Some(match who(first) {
    Some((p, l)) => Cmd::Set(p, l, tail.trim().to_owned()),
    None => Cmd::Usage(USAGE),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_cap_counts_like_the_server() {
    assert_eq!(clean(&"😀".repeat(MAX_CHARS)).chars().count(), MAX_CHARS / 2);
    assert_eq!(clean(&"a".repeat(MAX_CHARS + 5)).len(), MAX_CHARS);
  }

  #[test]
  fn keys_are_platform_and_lowercase() {
    assert_eq!(key(Platform::Twitch, " Bob "), "twitch:bob");
    let mut n = Notes::default();
    n.set(Platform::Twitch, "Bob", "hi", None);
    assert_eq!(n.text(Platform::Twitch, "bOB"), "hi");
    assert_eq!(n.text(Platform::Kick, "bob"), "", "a kick bob is someone else");
  }

  #[test]
  fn text_is_flat_and_capped() {
    let long = "x".repeat(MAX_CHARS + 50);
    assert_eq!(clean(&long).chars().count(), MAX_CHARS);
    assert_eq!(clean("a\nb\tc "), "a b c");
  }

  #[test]
  fn empty_text_deletes() {
    let mut n = Notes::default();
    n.set(Platform::Kick, "bob", "hi", None);
    assert_eq!(n.len(), 1);
    n.set(Platform::Kick, "bob", "  ", None);
    assert_eq!(n.len(), 0);
    n.set(Platform::Kick, "bob", "hi", None);
    assert_eq!(n.delete(Platform::Kick, "BOB").unwrap().text, "hi");
    assert!(n.delete(Platform::Kick, "bob").is_none());
  }

  #[test]
  fn round_trips_and_survives_garbage() {
    let mut n = Notes::default();
    n.set(Platform::Twitch, "bob", "likes cats", None);
    n.adopt(Platform::Kick, "al", "from server", "u123");
    let back = Notes::parse(&n.render());
    assert_eq!(back.get(Platform::Twitch, "bob"), n.get(Platform::Twitch, "bob"));
    assert_eq!(back.get(Platform::Kick, "al").unwrap().pid.as_deref(), Some("u123"));
    assert!(back.get(Platform::Kick, "al").unwrap().synced);
    let junk = "\n\tx\nnoplat\t1\t\tt\nfoo:bar\t0\t\tt\ntwitch:Up\t0\t\tt\ntwitch:ok\t0\t\t\ntwitch:fine\t1\t\tkept\n\u{0}\u{1}\n";
    let g = Notes::parse(junk);
    assert_eq!(g.len(), 1);
    // "synced" without a profile id is not trusted
    assert!(!g.get(Platform::Twitch, "fine").unwrap().synced);
  }

  #[test]
  fn a_full_file_refuses_new_people_but_edits_old_ones() {
    let mut n = Notes::default();
    for i in 0..MAX_NOTES {
      assert!(n.set(Platform::Twitch, &format!("u{i}"), "x", None));
    }
    assert!(!n.set(Platform::Twitch, "newbie", "x", None));
    assert!(n.set(Platform::Twitch, "u1", "y", None));
  }

  #[test]
  fn an_ack_counts_only_for_the_text_sent() {
    let mut n = Notes::default();
    n.set(Platform::Twitch, "bob", "a", Some("u1".into()));
    n.set(Platform::Twitch, "bob", "b", None);
    assert_eq!(n.get(Platform::Twitch, "bob").unwrap().pid.as_deref(), Some("u1"), "id is kept across edits");
    assert!(!n.acked(Platform::Twitch, "bob", "a"));
    assert!(n.acked(Platform::Twitch, "bob", "b"));
  }

  #[test]
  fn shadow_ids_never_sync() {
    assert!(!server_id("kick_5") && !server_id("yt_UC1") && !server_id(""));
    assert!(server_id("a1b2"));
    let mut n = Notes::default();
    n.adopt(Platform::Kick, "al", "t", "kick_5");
    assert_eq!(n.get(Platform::Kick, "al").unwrap().pid, None);
  }

  #[test]
  fn the_slash_command() {
    let c = |t| command(t).unwrap();
    assert_eq!(c("/note Bob likes  cats"), Cmd::Set(Platform::Twitch, "bob".into(), "likes  cats".into()));
    assert_eq!(c("/note kick:bob hi"), Cmd::Set(Platform::Kick, "bob".into(), "hi".into()));
    assert_eq!(c("/note youtube:bob"), Cmd::Set(Platform::Youtube, "bob".into(), String::new()));
    assert_eq!(c("/note bob"), Cmd::Set(Platform::Twitch, "bob".into(), String::new()));
    assert!(matches!(c("/note"), Cmd::Usage(_)));
    assert!(matches!(c("/note nope:bob x"), Cmd::Usage(_)));
    assert!(command("/notes bob").is_none() && command("hello").is_none());
    assert_eq!(tag(Platform::Kick, "bob"), "kick:bob");
  }
}
