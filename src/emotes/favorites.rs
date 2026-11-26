// the emotes you pinned. the list is the account's: heatsync.org's synced setting `emoteFavorites`, the one
// the site and the extension keep (GET / PUT /api/settings), so a pin made anywhere is here and a pin made
// here is everywhere. whole records, in the order you pinned them. a favorite is the picture, not the
// name: the same picture at another size or format is the same favorite, another emote of its name is not.
// nothing is kept on this machine: the list comes with the login and leaves with it
use serde_json::{Value, json};

use super::block::{Scope, cdn_id, emote_hash, picture_of};
use super::draft::host;

// how many, and how big the whole list may be in bytes: the setting's own rule (the site's registry)
pub const MAX: usize = 32;
const MAX_BYTES: usize = 9000;
const MAX_NAME: usize = 100;
const MAX_URL: usize = 400;
// emotes a platform draws itself: usable only where that platform hands them to you
const NATIVE: [&str; 3] = ["twitch", "kick", "youtube"];

#[derive(Clone, Debug, PartialEq)]
pub struct Fav {
  pub name: Box<str>,
  pub url: Box<str>,
  pub zero_width: bool,
  // our own upload's hash: the same upload met under another host is the same picture
  hash: Option<Box<str>>,
  // a platform's own emote: offered only where it is held
  pub native: bool,
  // this app can show it: a name the box takes, a picture from a host it loads from
  pub shown: bool,
  // the record as the account holds it. written back whole, so what another client keeps in it stays
  raw: Value,
}

impl Fav {
  // one record of the account's list. never trusted: the site, another device or an old build wrote it
  fn parse(v: &Value) -> Option<Fav> {
    let s = |k: &str| v.get(k).and_then(Value::as_str);
    let (name, url) = (s("name")?, s("url")?);
    if name.is_empty() || name.encode_utf16().count() > MAX_NAME || url.len() > MAX_URL || !url.starts_with("https://")
    {
      return None;
    }
    let native = s("provider").is_some_and(|p| NATIVE.contains(&p)) || NATIVE.contains(&host(url));
    Some(Fav {
      name: name.into(),
      url: url.into(),
      zero_width: v.get("zeroWidth").and_then(Value::as_bool) == Some(true),
      hash: s("hash").filter(|h| !h.is_empty() && h.len() <= 128).map(Into::into),
      native,
      shown: crate::heatsync::valid_name(name) && crate::images::url::allowed(url),
      raw: v.clone(),
    })
  }

  // the server's hash of the upload this pin is, if it is one
  pub fn hash(&self) -> Option<&str> {
    self.hash.as_deref()
  }

  // is the picture at `url` this favorite's: the same image, or the same upload of ours
  pub fn same(&self, url: &str) -> bool {
    picture_of(&self.url) == picture_of(url)
      || self.hash.as_deref().is_some_and(|h| h.len() == 64 && h == emote_hash(url, Scope::Hs))
  }

  // what is kept of a picture pinned here: enough for any client to draw it, pick it and tell it from
  // another emote of its name
  fn pinned(name: &str, url: &str, zero_width: bool) -> Option<Fav> {
    let provider = match host(url) {
      "heatsync" => "hs",
      p => p,
    };
    // the site draws webp: a 7tv picture is kept at the one address every client can show
    let id = cdn_id(url).filter(|_| matches!(provider, "7tv" | "bttv" | "ffz"));
    let url = match id.filter(|_| provider == "7tv") {
      Some(id) => format!("https://cdn.7tv.app/emote/{id}/2x.webp"),
      None => url.to_owned(),
    };
    let mut rec = json!({ "name": name, "url": url, "provider": provider });
    if let Some(id) = id {
      rec["id"] = id.into();
    }
    if provider == "hs" {
      rec["hash"] = emote_hash(&url, Scope::Hs).into();
    }
    if zero_width {
      rec["zeroWidth"] = true.into();
    }
    Fav::parse(&rec)
  }
}

// the account's `emoteFavorites` as records, in its order. None = it is not a list at all
pub fn parse(v: &Value) -> Option<Vec<Fav>> {
  Some(v.as_array()?.iter().filter_map(Fav::parse).take(MAX).collect())
}

fn json_of(list: &[Fav]) -> Value {
  Value::Array(list.iter().map(|f| f.raw.clone()).collect())
}

// what a pin or an unpin did. `Added` and `Removed` carry the whole list, for the account
#[derive(Debug, PartialEq)]
pub enum Toggle {
  Added(Value),
  Removed(Value),
  // the list is as long, or as big, as it may be
  Full,
  // nothing to pin: not a name and an https picture
  Nope,
}

#[derive(Default)]
pub struct Favorites {
  list: Vec<Fav>,
  // pins made here the account has not answered for yet: its older list must not undo them
  unsent: u32,
}

impl Favorites {
  pub fn list(&self) -> &[Fav] {
    &self.list
  }

  fn at(&self, name: &str, url: &str) -> Option<usize> {
    self.list.iter().position(|f| *f.name == *name && f.same(url))
  }

  // is this picture (the word and the image) pinned
  pub fn has(&self, name: &str, url: &str) -> bool {
    self.at(name, url).is_some()
  }

  // the account's list: at login, or changed on another of your clients. false = nothing changed here
  pub fn adopt(&mut self, list: Vec<Fav>) -> bool {
    if self.unsent > 0 || self.list == list {
      return false;
    }
    self.list = list;
    true
  }

  // logged out, or another account: its list is not the next one's
  pub fn clear(&mut self) -> bool {
    self.unsent = 0;
    !std::mem::take(&mut self.list).is_empty()
  }

  // pin or unpin a picture. a new favorite goes last: the order is the order you pinned in
  pub fn toggle(&mut self, name: &str, url: &str, zero_width: bool) -> Toggle {
    let at = self.at(name, url);
    let mut next = self.list.clone();
    match at {
      Some(i) => drop(next.remove(i)),
      None => match Fav::pinned(name, url, zero_width) {
        Some(f) => next.push(f),
        None => return Toggle::Nope,
      },
    }
    let json = json_of(&next);
    if at.is_none() && (next.len() > MAX || json.to_string().len() > MAX_BYTES) {
      return Toggle::Full;
    }
    self.list = next;
    self.unsent += 1;
    if at.is_some() { Toggle::Removed(json) } else { Toggle::Added(json) }
  }

  // the account answered one of our writes (took it, or refused it and its own list follows)
  pub fn answered(&mut self) {
    self.unsent = self.unsent.saturating_sub(1);
  }

  // a favorite's own overlay flag, for a pick of one nothing here holds
  pub fn zero_width(&self, name: &str, url: &str) -> Option<bool> {
    self.at(name, url).map(|i| self.list[i].zero_width)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const A: &str = "https://cdn.7tv.app/emote/01ARZ3NDEKTSV4RRFFQ69G5FAV/2x.webp";

  fn account() -> Value {
    json!([
      { "name": "Pog", "url": A, "provider": "7tv", "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "animated": true, "later": 1 },
      { "name": "Kappa", "url": "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0", "provider": "twitch" },
      { "name": "mine", "url": "https://cdn.heatsync.org/uploads/a.webp", "provider": "hs", "zeroWidth": true },
    ])
  }

  #[test]
  fn the_accounts_list_is_read_whole_and_hostile_records_are_dropped() {
    let l = parse(&account()).unwrap();
    assert_eq!(l.iter().map(|f| &*f.name).collect::<Vec<_>>(), ["Pog", "Kappa", "mine"]);
    assert_eq!(l.iter().map(|f| f.native).collect::<Vec<_>>(), [false, true, false]);
    assert!(l[2].zero_width && !l[0].zero_width);
    let bad = json!([
      null,
      7,
      { "name": "", "url": A },
      { "name": "x", "url": "http://cdn.7tv.app/emote/a/1x.webp" },
      { "name": "x".repeat(101), "url": A },
      { "name": "x", "url": format!("https://cdn.7tv.app/{}", "a".repeat(400)) },
      { "name": "ok", "url": A },
    ]);
    assert_eq!(parse(&bad).unwrap().iter().map(|f| &*f.name).collect::<Vec<_>>(), ["ok"]);
    assert!(parse(&json!({ "name": "x" })).is_none(), "not a list at all");
    let many: Vec<Value> = (0..40).map(|i| json!({ "name": format!("e{i}"), "url": A })).collect();
    assert_eq!(parse(&Value::Array(many)).unwrap().len(), MAX);
  }

  // drawn nowhere here, kept all the same: a write must not drop what another client pinned
  #[test]
  fn a_record_this_app_cannot_show_stays_in_the_list() {
    let v = json!([{ "name": "far", "url": "https://example.org/a.png" }, { "name": "two words", "url": A }]);
    let l = parse(&v).unwrap();
    assert_eq!(l.iter().map(|f| f.shown).collect::<Vec<_>>(), [false, false]);
    let mut f = Favorites::default();
    assert!(f.adopt(l));
    let Toggle::Added(sent) = f.toggle("Pog", A, false) else { panic!("pinned") };
    assert_eq!(sent.as_array().unwrap().len(), 3);
    assert_eq!(sent[0], v[0]);
    assert_eq!(sent[1], v[1]);
  }

  #[test]
  fn a_favorite_is_its_picture_at_any_size_or_format() {
    let mut f = Favorites::default();
    f.adopt(parse(&account()).unwrap());
    assert!(f.has("Pog", "https://cdn.7tv.app/emote/01ARZ3NDEKTSV4RRFFQ69G5FAV/1x.avif"));
    assert!(f.has("Pog", "https://cdn.7tv.app/emote/01ARZ3NDEKTSV4RRFFQ69G5FAV/4x_static.webp"));
    assert!(!f.has("Pog", "https://cdn.7tv.app/emote/01ARZ3NDEKTSV4RRFFQ69G5ZZZ/2x.webp"), "another Pog");
    assert!(!f.has("pog", A), "another name on the same picture");
    assert!(f.has("mine", "https://heatsync.org/uploads/a.webp"), "our upload under another host");
  }

  #[test]
  fn a_pin_goes_last_and_an_unpin_keeps_every_other_record_as_the_account_wrote_it() {
    let mut f = Favorites::default();
    f.adopt(parse(&account()).unwrap());
    let ffz = "https://cdn.frankerfacez.com/emote/381875/1";
    let Toggle::Added(sent) = f.toggle("ZW", ffz, true) else { panic!("pinned") };
    let rows = sent.as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[3], json!({ "name": "ZW", "url": ffz, "provider": "ffz", "id": "381875", "zeroWidth": true }));
    assert_eq!(rows[0]["later"], 1, "a field this app does not know stays");
    assert_eq!(f.zero_width("ZW", ffz), Some(true));
    let Toggle::Removed(sent) =
      f.toggle("Kappa", "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0", false)
    else {
      panic!("unpinned")
    };
    let names: Vec<&str> = sent.as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Pog", "mine", "ZW"]);
  }

  #[test]
  fn a_7tv_pin_is_kept_at_the_address_every_client_draws_and_an_upload_with_its_hash() {
    let mut f = Favorites::default();
    let Toggle::Added(sent) = f.toggle("Pog", "https://cdn.7tv.app/emote/01ARZ3NDEKTSV4RRFFQ69G5FAV/1x.avif", false)
    else {
      panic!("pinned")
    };
    assert_eq!(sent[0]["url"], A);
    assert_eq!(sent[0]["id"], "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    let up = "https://cdn.heatsync.org/uploads/a.webp";
    let Toggle::Added(sent) = f.toggle("mine", up, false) else { panic!("pinned") };
    assert_eq!(sent[1]["provider"], "hs");
    assert_eq!(sent[1]["hash"].as_str().unwrap(), emote_hash(up, Scope::Hs));
    assert!(sent[1].get("id").is_none());
  }

  #[test]
  fn the_list_is_full_at_the_settings_own_bounds_and_an_unpin_still_works() {
    let mut f = Favorites::default();
    let url = |i: usize| format!("https://cdn.betterttv.net/emote/{i:024x}/1x.webp");
    for i in 0..MAX {
      assert!(matches!(f.toggle(&format!("e{i}"), &url(i), false), Toggle::Added(_)), "{i}");
    }
    assert_eq!(f.toggle("over", &url(99), false), Toggle::Full);
    assert_eq!(f.list().len(), MAX);
    assert!(matches!(f.toggle("e0", &url(0), false), Toggle::Removed(_)));
    assert_eq!(f.toggle("x", "http://cdn.7tv.app/emote/a/1x.webp", false), Toggle::Nope);
    assert_eq!(f.toggle("", A, false), Toggle::Nope);
  }

  // a login check, or another device's pin, answering while our own write is on its way is older than it
  #[test]
  fn the_accounts_list_never_undoes_a_pin_it_has_not_answered() {
    let mut f = Favorites::default();
    assert!(matches!(f.toggle("Pog", A, false), Toggle::Added(_)));
    assert!(!f.adopt(Vec::new()), "the answer to an older ask");
    assert!(f.has("Pog", A));
    f.answered();
    assert!(!f.adopt(f.list().to_vec()), "the same list is no change");
    assert!(f.adopt(Vec::new()), "now the account's list is the list");
    assert!(!f.has("Pog", A));
  }
}
