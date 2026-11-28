// the logged-in user's emote writes: block list, add to / remove from their heatsync inventory.
// every response is hostile: bounded read, validated fields. what each answer says is `emotes::say`: the
// http status picks the line, the same line every heatsync app shows
use crate::emotes::say::{Op, Say, says};
use crate::i18n::{f, t};
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out};
use crate::emotes::block::valid_hash;
use crate::heatsync::valid_name;
use crate::images::url::allowed;

const MAX_LIST: usize = 8 << 20;
const MAX_ROWS: usize = 5000;
// the server's own slot bound
const MAX_SLOT: u64 = 50_000;

pub enum EmoteCmd {
  // a block or unblock made here (logged in): the account is told, and says so in one line when it is done.
  // `name` is what the line calls it, the hashes are what the account files it under
  Block { name: Box<str>, hash: String },
  Unblock { name: Box<str>, hashes: Vec<String> },
  Add { name: Box<str>, url: Box<str>, hash: Option<String>, zero_width: bool },
  Remove { name: Box<str> },
  // your twitch sub emotes, which heatsync reads off your twitch account
  Subs,
  // your blocked emotes changed on another device: read the server's list again
  Blocked,
  // your favorite emotes, the whole list as the account will hold it (a pin or an unpin made here)
  Favorites(Value),
  // another emote you own under this name makes way for this one (remove, then add: the server holds
  // one emote per name)
  Swap { name: Box<str>, url: Box<str>, zero_width: bool },
  // the picker's own-emote upload (`session` runs these on their own tasks, never this queue): ask a file
  // dialog for a picture, upload a file under a name, or add an upload the server already holds under another
  PickFile,
  Upload { path: std::path::PathBuf, name: String },
  AddKept { kept: super::upload::emote::Kept, name: String },
}

// a request that did not go through: the http status (0 = no answer at all: offline, timed out, cut off), and
// plain words for the callers whose message is not one of the emote lines
#[derive(Debug)]
pub struct Fail {
  pub status: u16,
  pub why: String,
}

fn nothing(why: impl std::fmt::Display) -> Fail {
  Fail { status: 0, why: super::http::offline("heatsync", why) }
}

// body capped while streaming, so a hostile server can't make us buffer more than `max`
async fn read(mut r: reqwest::Response, max: usize) -> Result<(u16, Value), Fail> {
  let code = r.status().as_u16();
  super::http::lapsed(code);
  let mut buf = Vec::new();
  while let Some(c) = r.chunk().await.map_err(nothing)? {
    if buf.len() + c.len() > max {
      return Err(Fail { status: 0, why: t("net2.emote.too_much").into() });
    }
    buf.extend_from_slice(&c);
  }
  Ok((code, serde_json::from_slice(&buf).unwrap_or(Value::Null)))
}

pub(super) fn why(code: u16, v: &Value) -> String {
  match super::http::server_words(v.get("error").and_then(Value::as_str).unwrap_or("")) {
    w if w.is_empty() => super::http::plain_status(code),
    w => w,
  }
}

pub(super) fn ok(code: u16) -> bool {
  (200..300).contains(&code)
}

// the server's blocked hashes (its `blocked_emotes[].hash`), replacing nothing local
pub async fn load_blocked(http: reqwest::Client, token: String, out: Out) {
  let r = http.get(format!("{ORIGIN}/api/user/emotes/blocked")).bearer_auth(&token).send().await;
  let Ok(r) = r else { return };
  let Ok((code, v)) = read(r, MAX_LIST).await else { return };
  if !ok(code) {
    return;
  }
  let hashes = v
    .get("blocked_emotes")
    .and_then(Value::as_array)
    .map(|a| {
      a.iter()
        .take(MAX_ROWS)
        .filter_map(|x| x.get("hash")?.as_str())
        .filter(|h| valid_hash(h))
        .map(Into::into)
        .collect::<Vec<Box<str>>>()
    })
    .unwrap_or_default();
  out.send(NetEvent::BlockedHashes(hashes));
}

// the emotes you pinned: the account's `emoteFavorites` setting, the list the site and the extension keep.
// an account that never pinned has no list, which is an empty one. quiet on failure: the next login asks again
pub async fn load_favorites(http: reqwest::Client, token: String, out: Out) {
  let r = http.get(format!("{ORIGIN}/api/settings")).bearer_auth(&token).send().await;
  let Ok(r) = r else { return };
  let Ok((code, v)) = read(r, MAX_LIST).await else { return };
  let Some(settings) = v.get("settings").filter(|s| ok(code) && s.is_object()) else { return };
  let list = settings.get("emoteFavorites").and_then(crate::emotes::favorites::parse).unwrap_or_default();
  out.send(NetEvent::Favorites(list));
}

// the account takes the whole list. it took it only when it says when (`ts`): a server that does not know
// the setting answers ok and stores nothing
async fn save_favorites(http: &reqwest::Client, token: &str, list: &Value) -> Result<(), String> {
  let r = http
    .put(format!("{ORIGIN}/api/settings"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(json!({ "patches": { "emoteFavorites": list } }).to_string())
    .send()
    .await;
  let v = result(r).await.map_err(|e| e.why)?;
  if v.get("ts").is_some_and(Value::is_number) { Ok(()) } else { Err(super::http::plain_status(501)) }
}

pub async fn run(http: &reqwest::Client, token: &str, cmd: EmoteCmd, out: &Out) {
  match cmd {
    EmoteCmd::Favorites(list) => {
      let saved = save_favorites(http, token, &list).await;
      out.send(NetEvent::FavoritesSaved);
      // refused: say why, and the account's own list is the list again
      if let Err(e) = saved {
        out.status(f("net2.emote.fav_sync", &[("why", &e)]));
        load_favorites(http.clone(), token.to_owned(), out.clone()).await;
      }
    }
    EmoteCmd::Block { name, hash } => {
      let r = match valid_hash(&hash) {
        true => {
          let r = http
            .post(format!("{ORIGIN}/api/user/emotes/block"))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(json!({ "emote_hash": hash }).to_string())
            .send()
            .await;
          result(r).await
        }
        false => Err(Fail { status: 0, why: String::new() }),
      };
      let (say, line) = told(Op::Block, &name, &r);
      out.status(line);
      if !say.is_ok() {
        out.send(NetEvent::EmoteUndone { block: true, name, hashes: vec![hash] });
      }
    }
    EmoteCmd::Unblock { name, hashes } => {
      let mut refused = Vec::new();
      for hash in hashes {
        let r = match valid_hash(&hash) {
          true => {
            let r = http
              .delete(format!("{ORIGIN}/api/user/emotes/blocked/{}", crate::util::path_seg(&hash)))
              .bearer_auth(token)
              .send()
              .await;
            result(r).await
          }
          false => Err(Fail { status: 0, why: String::new() }),
        };
        // nothing blocked there (404) is an unblock that worked
        if !told(Op::Unblock, &name, &r).0.is_ok() {
          refused.push(hash);
        }
      }
      // one line for the action, however many hashes it was filed under
      let said = if refused.is_empty() { Say::UnblockOk } else { Say::UnblockFailed };
      out.status(said.line(&name, None));
      if !refused.is_empty() {
        out.send(NetEvent::EmoteUndone { block: false, name, hashes: refused });
      }
    }
    EmoteCmd::Add { name, url, hash, zero_width } => {
      if !valid_name(&name) || !allowed(&url) {
        return out.status(Say::AddFailed.line(&name, None));
      }
      let r = add(http, token, &name, &url, hash.as_deref(), zero_width, false).await;
      let (say, line) = told(Op::Add, &name, &r);
      out.status(line);
      if say.is_ok() {
        out.send(NetEvent::Inventory { removed: None });
      }
    }
    EmoteCmd::Blocked => {
      tokio::spawn(load_blocked(http.clone(), token.to_owned(), out.clone()));
    }
    EmoteCmd::Subs => {
      let r = http.get(format!("{ORIGIN}/api/user/emotes")).bearer_auth(token).send().await;
      // quiet on failure: tab and the picker just go without them until the next login
      if let Ok(v) = result(r).await {
        out.send(NetEvent::SubEmotes(subs(&v)));
      }
    }
    EmoteCmd::Swap { name, url, zero_width } => {
      if !valid_name(&name) || !allowed(&url) {
        return out.status(Say::SwapFailed.line(&name, None));
      }
      // one request: the server points your slot at the new emote, so a failure leaves the old one
      let r = add(http, token, &name, &url, None, zero_width, true).await;
      let (say, line) = told(Op::Swap, &name, &r);
      out.status(line);
      if say.is_ok() {
        out.send(NetEvent::Inventory { removed: Some(name) });
      }
    }
    EmoteCmd::PickFile | EmoteCmd::Upload { .. } | EmoteCmd::AddKept { .. } => {}
    EmoteCmd::Remove { name } => {
      let r = remove(http, token, &name).await.map(|()| Value::Null);
      let (say, line) = told(Op::Remove, &name, &r);
      out.status(line);
      if say.is_ok() {
        out.send(NetEvent::Inventory { removed: Some(name) });
      }
    }
  }
}

// the line an answer to `op` says, and which line that is. a slot the server filed an add under is in the answer;
// every number off the wire is bounded
fn told(op: Op, name: &str, answer: &Result<Value, Fail>) -> (Say, String) {
  let status = answer.as_ref().err().map_or(200, |e| e.status);
  let say = says(op, status);
  let slot = answer.as_ref().ok().and_then(|v| v.get("slot")?.as_u64()).filter(|&s| s <= MAX_SLOT);
  (say, say.line(name, slot))
}

// `subscriptionEmotes` of GET /api/user/emotes: hostile, so bounded, named sanely, and only urls on
// the image host allowlist
fn subs(v: &Value) -> crate::emotes::Set {
  let rows = v.get("subscriptionEmotes").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
  let mut out = crate::emotes::Set::new();
  for r in rows.iter().take(crate::emotes::MAX_SET) {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
    let (name, url) = (s("custom_name"), s("url"));
    if !valid_name(name) || !url.starts_with("https://") || !allowed(url) {
      continue;
    }
    let px = |k: &str| r.get(k).and_then(Value::as_u64).filter(|&n| n <= 512).unwrap_or(0) as u16;
    out.entry(name.into()).or_insert(crate::emotes::Emote {
      url: url.into(),
      w: px("width"),
      h: px("height"),
      zero_width: false,
    });
  }
  out
}

async fn add(
  http: &reqwest::Client,
  token: &str,
  name: &str,
  url: &str,
  hash: Option<&str>,
  zero_width: bool,
  replace: bool,
) -> Result<Value, Fail> {
  let r = http
    .post(format!("{ORIGIN}/api/user/emotes"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(add_body(name, url, hash, zero_width, replace).to_string())
    .send()
    .await;
  result(r).await
}

// `replace`: a different emote you own under this name is swapped for this one in the same slot
fn add_body(name: &str, url: &str, hash: Option<&str>, zero_width: bool, replace: bool) -> Value {
  let mut b = json!({
    "emoteUrl": url, "emoteName": name, "customName": name,
    "source": "extension", "sourceId": hash, "zeroWidth": zero_width,
  });
  if replace {
    b["replace"] = Value::Bool(true);
  }
  b
}

async fn result(r: Result<reqwest::Response, reqwest::Error>) -> Result<Value, Fail> {
  let (code, v) = read(r.map_err(nothing)?, MAX_LIST).await?;
  if ok(code) { Ok(v) } else { Err(Fail { status: code, why: why(code, &v) }) }
}

// the slot is how the server names an inventory row: look it up by name, then delete it. a name that is not
// in the list is already gone: a remove that worked
async fn remove(http: &reqwest::Client, token: &str, name: &str) -> Result<(), Fail> {
  let r = http.get(format!("{ORIGIN}/api/user/emotes")).bearer_auth(token).send().await;
  let v = result(r).await?;
  let slot = v.get("emotes").and_then(Value::as_array).and_then(|rows| slot_of(rows, name));
  let Some(slot) = slot else { return Ok(()) };
  let r = http.delete(format!("{ORIGIN}/api/user/emotes/{slot}")).bearer_auth(token).send().await;
  result(r).await.map(drop)
}

fn slot_of(rows: &[Value], name: &str) -> Option<u64> {
  rows
    .iter()
    .take(MAX_ROWS)
    .find(|r| {
      let n = |k: &str| r.get(k).and_then(Value::as_str);
      n("custom_name").or_else(|| n("name")) == Some(name)
    })
    .and_then(|r| r.get("slot_number")?.as_u64())
    .filter(|&s| s <= MAX_SLOT)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn en() {
    crate::i18n::set(crate::i18n::lang::EN);
  }

  fn refused(status: u16) -> Result<Value, Fail> {
    Err(Fail { status, why: "the server's own words".into() })
  }

  // an add says the slot the server filed it in; a refusal says why in the contract's words
  #[test]
  fn an_add_says_its_slot_and_a_refusal_says_why() {
    en();
    assert_eq!(told(Op::Add, "MeN", &Ok(json!({"slot": 12}))).1, "added MeN to slot 12");
    assert_eq!(told(Op::Add, "MeN", &refused(401)).1, "log in to add emotes");
    assert_eq!(told(Op::Add, "MeN", &refused(403)).1, "emote slots full — remove some first");
    assert_eq!(told(Op::Add, "MeN", &refused(409)).1, "already have an emote named MeN");
    assert_eq!(told(Op::Add, "MeN", &refused(503)).1, "moderation busy, try again");
    assert_eq!(told(Op::Add, "MeN", &refused(500)).1, "couldn't add MeN");
    assert_eq!(told(Op::Add, "MeN", &refused(0)).1, "couldn't add MeN");
    assert!(told(Op::Add, "MeN", &Ok(json!({"slot": 12}))).0.is_ok());
    assert!(!told(Op::Add, "MeN", &refused(409)).0.is_ok());
  }

  // the slot is the server's word, bounded: missing, absurd or the wrong kind says "added" and no number
  #[test]
  fn a_slot_off_the_wire_is_never_trusted_into_the_line() {
    en();
    for v in [json!({}), json!({"slot": "12"}), json!({"slot": 999_999_999}), json!({"slot": -1}), json!(null)] {
      assert_eq!(told(Op::Add, "MeN", &Ok(v)).1, "added MeN");
    }
  }

  #[test]
  fn a_swap_a_remove_and_the_blocks_say_the_contracts_lines() {
    en();
    let ok = || -> Result<Value, Fail> { Ok(json!({"slot": 3})) };
    assert_eq!(told(Op::Swap, "IDK", &ok()).1, "IDK swapped to your pick");
    assert_eq!(told(Op::Swap, "IDK", &refused(409)).1, "IDK is your upload — kept it");
    assert_eq!(told(Op::Swap, "IDK", &refused(500)).1, "couldn't swap IDK — kept yours");
    assert_eq!(told(Op::Remove, "IDK", &Ok(Value::Null)).1, "removed IDK");
    assert_eq!(told(Op::Remove, "IDK", &refused(404)).1, "removed IDK", "already gone is a remove that worked");
    assert_eq!(told(Op::Remove, "IDK", &refused(401)).1, "log in to manage inventory");
    assert_eq!(told(Op::Remove, "IDK", &refused(500)).1, "remove failed: IDK");
    assert_eq!(told(Op::Block, "IDK", &ok()).1, "blocked IDK");
    assert_eq!(told(Op::Block, "IDK", &refused(500)).1, "couldn't block IDK — try again");
    assert_eq!(told(Op::Unblock, "IDK", &ok()).1, "unblocked IDK");
    assert_eq!(told(Op::Unblock, "IDK", &refused(404)).1, "unblocked IDK", "nothing blocked there");
    assert_eq!(told(Op::Unblock, "IDK", &refused(0)).1, "couldn't unblock IDK — try again");
  }

  #[test]
  fn a_swap_is_one_add_that_replaces() {
    let u = "https://cdn.7tv.app/emote/x/1x.webp";
    assert_eq!(add_body("IDK", u, None, false, true)["replace"], true);
    assert!(add_body("IDK", u, None, false, false).get("replace").is_none());
  }

  #[test]
  fn sub_emotes_are_read_bounded_and_on_the_allowlist() {
    let row = |n: &str, u: &str| serde_json::json!({"custom_name": n, "url": u, "width": 28, "height": 28});
    let ok = "https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_x/default/dark/1.0";
    let v = serde_json::json!({"subscriptionEmotes": [
      row("delloreat", ok), row("evil", "https://evil.example/x.png"), row("plain", "http://static-cdn.jtvnw.net/x"),
      row("has space", ok), row("", ok), {"custom_name": 5, "url": ok}
    ]});
    let s = subs(&v);
    assert_eq!(s.keys().map(|k| &**k).collect::<Vec<_>>(), ["delloreat"]);
    assert_eq!(s["delloreat"].w, 28);
    for junk in [serde_json::json!(null), serde_json::json!({"subscriptionEmotes": "x"}), serde_json::json!([])] {
      assert!(subs(&junk).is_empty());
    }
  }

  #[test]
  fn finds_the_slot_by_name() {
    let rows = vec![
      json!({"custom_name": "a", "slot_number": 3}),
      json!({"name": "b", "slot_number": 7}),
      json!({"custom_name": "evil", "slot_number": 999999}),
      json!({"custom_name": "noslot"}),
    ];
    assert_eq!(slot_of(&rows, "a"), Some(3));
    assert_eq!(slot_of(&rows, "b"), Some(7));
    assert_eq!(slot_of(&rows, "evil"), None);
    assert_eq!(slot_of(&rows, "noslot"), None);
    assert_eq!(slot_of(&rows, "zzz"), None);
  }
}
