// locally blocked emotes. identity matches the ext/server: a block is filed under a hash — the
// provider's own emote id for 7tv/bttv/ffz/twitch urls, else base64 of the url — and hides the
// emote wherever its NAME or hash shows up. logged out it lives only in the `blocked` store file
use std::collections::HashSet;

use crate::store;

const FILE: &str = "blocked";
// server column is varchar(255); the base64 shape is capped at 253 chars + padding
pub const MAX_HASH: usize = 255;
const MAX_BLOCKED: usize = 5000;

pub fn valid_hash(h: &str) -> bool {
  !h.is_empty() && h.len() <= MAX_HASH && h.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/_-=".contains(&b))
}

// where a rendered emote came from. the ext files a block under whatever hash its cache holds
// for that emote, and that depends on the source — so parity needs the source too
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
  // a heatsync inventory (yours, a sender's, a relayed hsEmote): the server's sha256
  Hs,
  // a channel's 7tv/bttv/ffz/twitch set: the provider's own id (ffz as `ffz-<id>`)
  Channel,
  // global sets come through heatsync's /api/emotes: sha256 — except twitch's own, by id
  Global,
  // twitch's own emote in a line: its id
  Native,
}

// the id after `<marker>` in a cdn url: `cdn.7tv.app/emote/<id>/1x.webp`
fn id_after<'a>(url: &'a str, host: &str, marker: &str) -> Option<&'a str> {
  let rest = url.strip_prefix("https://")?;
  let (h, path) = rest.split_once('/')?;
  if h != host && !h.ends_with(&format!(".{host}")) {
    return None;
  }
  let id = path.strip_prefix(marker)?.split(['/', '?', '#']).next()?;
  valid_hash(id).then_some(id)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
  Plain,
  Ffz,
  Twitch,
}

fn provider_id(url: &str) -> Option<(Kind, &str)> {
  let p = |host, marker| id_after(url, host, marker);
  p("7tv.app", "emote/")
    .or_else(|| p("betterttv.net", "emote/"))
    .or_else(|| p("kick.com", "emotes/"))
    .map(|id| (Kind::Plain, id))
    .or_else(|| p("frankerfacez.com", "emote/").map(|id| (Kind::Ffz, id)))
    .or_else(|| p("jtvnw.net", "emoticons/v2/").map(|id| (Kind::Twitch, id)))
}

// the server's url normalisation before it hashes (shared/emote-url.ts + utils/server-utils.ts):
// a 7tv avif is its webp, and first-party origins drop to a path
fn normalized(url: &str) -> String {
  let mut u = url.to_owned();
  for pre in ["https://cdn.7tv.app/emote/", "http://cdn.7tv.app/emote/", "//cdn.7tv.app/emote/"] {
    if let Some(rest) = u.strip_prefix(pre)
      && let Some((id, file)) = rest.split_once('/')
      && !id.is_empty()
    {
      let (name, tail) = file.find(['?', '#']).map_or((file, ""), |i| file.split_at(i));
      let stem = name.strip_suffix(".avif");
      if let Some(stem) = stem
        && let Some(size) = stem.strip_suffix("_static").or(Some(stem))
        && matches!(size, "1x" | "2x" | "3x" | "4x")
      {
        u = format!("{pre}{id}/{stem}.webp{tail}");
      }
      break;
    }
  }
  for origin in [
    "https://cdn.heatsync.org",
    "https://heatsync.org",
    "http://heatsync.org",
    "https://www.heatsync.org",
    "http://www.heatsync.org",
    "http://localhost:3001",
    "http://localhost:5173",
  ] {
    if u == origin {
      return "/".into();
    }
    if let Some(rest) = u.strip_prefix(origin)
      && rest.starts_with('/')
    {
      return rest.to_owned();
    }
  }
  u
}

fn sha_hex<'a>(url: &str, out: &'a mut [u8; 64]) -> &'a str {
  const HEX: &[u8; 16] = b"0123456789abcdef";
  let d = ring::digest::digest(&ring::digest::SHA256, normalized(url).as_bytes());
  for (i, b) in d.as_ref().iter().enumerate() {
    out[2 * i] = HEX[(b >> 4) as usize];
    out[2 * i + 1] = HEX[(b & 15) as usize];
  }
  std::str::from_utf8(out).unwrap_or("")
}

// the hash the ext files a block under for an emote from `scope`
pub fn emote_hash(url: &str, scope: Scope) -> String {
  let sha = || sha_hex(url, &mut [0; 64]).to_owned();
  match (provider_id(url), scope) {
    (Some((Kind::Twitch, id)), _) => id.to_owned(),
    (Some((Kind::Plain, id)), Scope::Channel | Scope::Native) => id.to_owned(),
    (Some((Kind::Ffz, id)), Scope::Channel | Scope::Native) => format!("ffz-{id}"),
    _ => sha(),
  }
}

// every form a block of this emote might be filed under (any ext path), checked without allocating
fn any_form(url: &str, mut f: impl FnMut(&str) -> bool) -> bool {
  if let Some((k, id)) = provider_id(url) {
    if f(id) {
      return true;
    }
    if k == Kind::Ffz && f(&format!("ffz-{id}")) {
      return true;
    }
    if k == Kind::Twitch {
      return false;
    }
  }
  f(sha_hex(url, &mut [0; 64]))
}

#[derive(Default)]
pub struct Blocked {
  // locally blocked names (the render gate, like the ext's blockedEmoteNames)
  names: HashSet<Box<str>>,
  // hashes of our own blocks
  hashes: HashSet<Box<str>>,
  // the logged-in account's server list (other devices): replaced on every fetch, dropped on logout
  server: HashSet<Box<str>>,
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
    if self.hashes.is_empty() && self.server.is_empty() {
      return false;
    }
    any_form(url, |h| self.hashes.contains(h) || self.server.contains(h))
  }

  // true when this changed anything. `hash` is what to file it under (see `emote_hash`)
  pub fn block(&mut self, name: &str, url: &str, scope: Scope) -> bool {
    if self.names.len() >= MAX_BLOCKED || !crate::heatsync::valid_name(name) {
      return false;
    }
    let a = self.names.insert(name.into());
    let b = self.hashes.insert(emote_hash(url, scope).into());
    a || b
  }

  // every hash this emote was filed under, now forgotten (for the server to forget too)
  pub fn unblock(&mut self, name: &str, url: &str, scope: Scope) -> Vec<String> {
    let mut gone = Vec::new();
    self.names.remove(name);
    any_form(url, |h| {
      let a = self.hashes.remove(h);
      let b = self.server.remove(h);
      if a || b {
        gone.push(h.to_owned());
      }
      false
    });
    if gone.is_empty() {
      gone.push(emote_hash(url, scope));
    }
    gone
  }

  // the server's list replaces what an earlier fetch said; local names stay
  pub fn set_server(&mut self, hashes: Vec<Box<str>>) {
    self.server = hashes.into_iter().filter(|h| valid_hash(h)).take(MAX_BLOCKED).collect();
  }

  // logged out or switched account: the old account's list must not keep hiding emotes
  pub fn clear_server(&mut self) {
    self.server.clear();
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

  // expected values computed the ext/server way: sha256 (python hashlib) of the normalised url
  const SEVENTV: &str = "https://cdn.7tv.app/emote/01ABC/1x.webp";
  const SEVENTV_SHA: &str = "b77bff3d9068dbd6ac26734d183ecb9480e63250de106a79ab1c317d0360dc65";

  #[test]
  fn hash_parity_per_source() {
    // channel sets: the provider id (ffz prefixed), twitch by id wherever it shows up
    assert_eq!(emote_hash(SEVENTV, Scope::Channel), "01ABC");
    assert_eq!(emote_hash("https://cdn.betterttv.net/emote/5f1b/1x.webp", Scope::Channel), "5f1b");
    assert_eq!(emote_hash("https://cdn.frankerfacez.com/emote/381875/1", Scope::Channel), "ffz-381875");
    assert_eq!(emote_hash("https://files.kick.com/emotes/777/fullsize", Scope::Channel), "777");
    let tw = "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0";
    for sc in [Scope::Hs, Scope::Channel, Scope::Global, Scope::Native] {
      assert_eq!(emote_hash(tw, sc), "25");
    }
    assert_eq!(emote_hash(tw, Scope::Native), "25");
    // inventories and /api/emotes globals: the server's sha256, even for a provider url
    assert_eq!(emote_hash(SEVENTV, Scope::Hs), SEVENTV_SHA);
    assert_eq!(emote_hash(SEVENTV, Scope::Global), SEVENTV_SHA);
    assert_eq!(
      emote_hash("https://cdn.betterttv.net/emote/5f1b/1x.webp", Scope::Hs),
      "710451d50450b9b8da58fb1f07c244b16ef0b0c4c66c24ba632fb1dd77725fb1"
    );
    assert_eq!(
      emote_hash("https://cdn.frankerfacez.com/emote/381875/1", Scope::Global),
      "83c8be4f483c6ac93edbac7f132bdec8685e0a4c670c79b7d8d18cd0ffa97b2e"
    );
  }

  #[test]
  fn sha_normalises_like_the_server() {
    // first-party origins hash as their path
    let want = "4cd6546ec9fb7e1ac16b8d09c7ebfd662df89c94ec09e723dff5c5abc516d91f";
    for u in ["https://cdn.heatsync.org/uploads/a.webp", "https://heatsync.org/uploads/a.webp", "/uploads/a.webp"] {
      assert_eq!(emote_hash(u, Scope::Hs), want, "{u}");
    }
    // a lookalike host is not first-party
    assert_ne!(emote_hash("https://cdn.heatsync.org.evil.com/uploads/a.webp", Scope::Hs), want);
    // a 7tv avif is its webp
    assert_eq!(emote_hash("https://cdn.7tv.app/emote/01ABC/1x.avif", Scope::Hs), SEVENTV_SHA);
    assert_eq!(
      normalized("https://cdn.7tv.app/emote/01ABC/2x_static.avif?v=1"),
      "https://cdn.7tv.app/emote/01ABC/2x_static.webp?v=1"
    );
    assert_eq!(normalized("https://cdn.7tv.app/emote/01ABC/1x.gif"), "https://cdn.7tv.app/emote/01ABC/1x.gif");
    assert_eq!(emote_hash("https://x.example/a", Scope::Hs).len(), 64);
  }

  #[test]
  fn a_block_applies_whichever_way_the_ext_filed_it() {
    let mut b = Blocked::default();
    // the ext filed it by id (channel); we see the same emote as an inventory/global url
    b.set_server(vec!["01ABC".into()]);
    assert!(b.contains("x", SEVENTV));
    let mut b = Blocked::default();
    b.set_server(vec![SEVENTV_SHA.into()]);
    assert!(b.contains("x", SEVENTV));
    let mut b = Blocked::default();
    b.set_server(vec!["ffz-381875".into()]);
    assert!(b.contains("x", "https://cdn.frankerfacez.com/emote/381875/1"));
    assert!(!b.contains("x", "https://cdn.frankerfacez.com/emote/1/1"));
  }

  #[test]
  fn server_list_is_replaced_and_dropped_on_logout() {
    let mut b = Blocked::default();
    b.set_server(vec!["01ABC".into()]);
    assert!(b.contains("x", SEVENTV));
    b.set_server(vec!["other".into()]);
    assert!(!b.contains("x", SEVENTV));
    b.set_server(vec!["01ABC".into()]);
    b.clear_server();
    assert!(!b.contains("x", SEVENTV));
    b.block("Mine", "https://cdn.7tv.app/emote/ZZZ/1x.webp", Scope::Channel);
    b.clear_server();
    assert!(b.contains("Mine", ""));
  }

  #[test]
  fn block_by_name_or_hash() {
    let mut b = Blocked::default();
    assert!(!b.contains("KEKW", SEVENTV));
    assert!(b.block("KEKW", SEVENTV, Scope::Channel));
    assert!(!b.block("KEKW", SEVENTV, Scope::Channel));
    // same name from another provider, and same emote under another name, are both hidden
    assert!(b.contains("KEKW", "https://cdn.betterttv.net/emote/zz/1x.webp"));
    assert!(b.contains("Other", SEVENTV));
    // unblocking forgets every form that was held, so the server can forget them too
    b.set_server(vec![SEVENTV_SHA.into()]);
    let gone = b.unblock("KEKW", SEVENTV, Scope::Channel);
    assert_eq!(gone, vec!["01ABC".to_owned(), SEVENTV_SHA.to_owned()]);
    assert!(!b.contains("KEKW", SEVENTV) && !b.contains("Other", SEVENTV));
    // nothing held: still names the scope's hash so a stale server entry can go
    assert_eq!(b.unblock("KEKW", SEVENTV, Scope::Hs), vec![SEVENTV_SHA.to_owned()]);
  }

  #[test]
  fn server_list_is_validated() {
    let mut b = Blocked::default();
    b.set_server(vec!["01ABC".into(), "bad hash!".into(), "".into()]);
    assert!(b.contains("x", SEVENTV));
    assert_eq!(b.server.len(), 1);
  }
}
