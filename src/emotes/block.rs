// locally blocked emotes. identity matches the ext/server: a block is filed under a hash — the
// provider's own emote id for 7tv/bttv/ffz/twitch urls, else base64 of the url — and hides the
// emote wherever its NAME or hash shows up. logged out it lives only in the `blocked` store file
use std::collections::HashSet;

use crate::net::auth::b64url;
use crate::store;

const FILE: &str = "blocked";
// server column is varchar(255); the base64 shape is capped at 253 chars + padding
pub const MAX_HASH: usize = 255;
const MAX_BLOCKED: usize = 5000;

pub fn valid_hash(h: &str) -> bool {
  !h.is_empty() && h.len() <= MAX_HASH && h.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/_-=".contains(&b))
}

// the id after `/<marker>/` in a cdn url: `cdn.7tv.app/emote/<id>/1x.webp`
fn id_after<'a>(url: &'a str, host: &str, marker: &str) -> Option<&'a str> {
  let rest = url.strip_prefix("https://")?;
  let (h, path) = rest.split_once('/')?;
  if h != host && !h.ends_with(&format!(".{host}")) {
    return None;
  }
  let id = path.strip_prefix(marker)?.split(['/', '?', '#']).next()?;
  valid_hash(id).then_some(id)
}

fn provider_id(url: &str) -> Option<&str> {
  id_after(url, "7tv.app", "emote/")
    .or_else(|| id_after(url, "betterttv.net", "emote/"))
    .or_else(|| id_after(url, "frankerfacez.com", "emote/"))
    .or_else(|| id_after(url, "jtvnw.net", "emoticons/v2/"))
}

// what to file a block under. None = the url is too long to be a valid hash (local-only block)
pub fn emote_hash(url: &str) -> Option<String> {
  if let Some(id) = provider_id(url) {
    return Some(id.to_owned());
  }
  // a truncated id could collide with another emote's, so too long means local-only
  let b = b64url(url.as_bytes());
  (b.len() <= 253 && valid_hash(&b)).then_some(b)
}

#[derive(Default)]
pub struct Blocked {
  // locally blocked names (the render gate, like the ext's blockedEmoteNames)
  names: HashSet<Box<str>>,
  // hashes: ours plus the server's list (other devices)
  hashes: HashSet<Box<str>>,
}

impl Blocked {
  pub fn load() -> Self {
    let mut b = Self::default();
    for l in store::read(FILE).unwrap_or_default().lines().take(MAX_BLOCKED) {
      let (name, hash) = l.split_once('\t').unwrap_or((l, ""));
      if crate::heatsync::valid_name(name) {
        b.names.insert(name.into());
      }
      if valid_hash(hash) {
        b.hashes.insert(hash.into());
      }
    }
    b
  }

  pub fn contains(&self, name: &str, url: &str) -> bool {
    if self.names.contains(name) {
      return true;
    }
    if self.hashes.is_empty() {
      return false;
    }
    // provider urls (the common case) match without allocating
    match provider_id(url) {
      Some(id) => self.hashes.contains(id),
      None => emote_hash(url).is_some_and(|h| self.hashes.contains(&*h)),
    }
  }

  // true when this changed anything
  pub fn block(&mut self, name: &str, url: &str) -> bool {
    if self.names.len() >= MAX_BLOCKED || !crate::heatsync::valid_name(name) {
      return false;
    }
    let a = self.names.insert(name.into());
    let b = emote_hash(url).is_some_and(|h| self.hashes.insert(h.into()));
    a || b
  }

  pub fn unblock(&mut self, name: &str, url: &str) -> bool {
    let a = self.names.remove(name);
    let b = emote_hash(url).is_some_and(|h| self.hashes.remove(&*h));
    a || b
  }

  // the server's list replaces what an earlier fetch said; local names stay
  pub fn set_server(&mut self, hashes: Vec<Box<str>>) {
    self.hashes.extend(hashes.into_iter().filter(|h| valid_hash(h)).take(MAX_BLOCKED));
  }

  pub fn save(&self) {
    let mut s = String::new();
    for n in &self.names {
      s.push_str(n);
      s.push('\n');
    }
    if let Err(e) = store::write(FILE, &s) {
      eprintln!("save blocked: {e}");
    }
  }
}

// what a right-click does, given what the emote is to you. precedence (mellen's call): an emote in
// your own heatsync inventory is removed from it — never blocked; anything else toggles its block
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
  Remove,
  Block,
  Unblock,
}

pub fn right_click(owned: bool, blocked: bool) -> Click {
  match (owned, blocked) {
    (true, _) => Click::Remove,
    (false, true) => Click::Unblock,
    (false, false) => Click::Block,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn right_click_precedence() {
    assert_eq!(right_click(true, false), Click::Remove);
    // owned wins even when blocked: removal, not unblock
    assert_eq!(right_click(true, true), Click::Remove);
    assert_eq!(right_click(false, false), Click::Block);
    assert_eq!(right_click(false, true), Click::Unblock);
  }

  #[test]
  fn hashes_follow_the_provider_id() {
    assert_eq!(emote_hash("https://cdn.7tv.app/emote/01ABCDEF/1x.webp").as_deref(), Some("01ABCDEF"));
    assert_eq!(emote_hash("https://cdn.betterttv.net/emote/5f1b/1x.webp").as_deref(), Some("5f1b"));
    assert_eq!(emote_hash("https://cdn.frankerfacez.com/emote/381875/1").as_deref(), Some("381875"));
    assert_eq!(emote_hash("https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0").as_deref(), Some("25"));
    // anything else: base64 of the url, always a legal hash, bounded
    let h = emote_hash("https://cdn.heatsync.org/uploads/a.webp").unwrap();
    assert!(valid_hash(&h));
    assert!(emote_hash(&format!("https://cdn.heatsync.org/{}", "a".repeat(400))).is_none());
    // a lookalike host never borrows a provider id
    assert!(id_after("https://evil7tv.app/emote/x/1x", "7tv.app", "emote/").is_none());
  }

  #[test]
  fn block_by_name_or_hash() {
    let mut b = Blocked::default();
    let url = "https://cdn.7tv.app/emote/01ABC/1x.webp";
    assert!(!b.contains("KEKW", url));
    assert!(b.block("KEKW", url));
    assert!(!b.block("KEKW", url));
    // same name from another provider, and same emote under another name, are both hidden
    assert!(b.contains("KEKW", "https://cdn.betterttv.net/emote/zz/1x.webp"));
    assert!(b.contains("Other", url));
    assert!(b.unblock("KEKW", url));
    assert!(!b.contains("KEKW", url) && !b.contains("Other", url));
  }

  #[test]
  fn server_list_is_validated() {
    let mut b = Blocked::default();
    b.set_server(vec!["01ABC".into(), "bad hash!".into(), "".into()]);
    assert!(b.contains("x", "https://cdn.7tv.app/emote/01ABC/1x.webp"));
    assert_eq!(b.hashes.len(), 1);
  }
}
