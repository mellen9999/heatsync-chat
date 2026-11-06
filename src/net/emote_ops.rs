// the logged-in user's emote writes: block list, add to / remove from their heatsync inventory.
// every response is hostile: bounded read, validated fields
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
  Block { hash: String },
  Unblock { hash: String },
  Add { name: Box<str>, url: Box<str>, hash: Option<String>, zero_width: bool },
  Remove { name: Box<str> },
  // your twitch sub emotes, which heatsync reads off your twitch account
  Subs,
  // your blocked emotes changed on another device: read the server's list again
  Blocked,
  // another emote you own under this name makes way for this one (remove, then add: the server holds
  // one emote per name)
  Swap { name: Box<str>, url: Box<str>, zero_width: bool },
}

// body capped while streaming, so a hostile server can't make us buffer more than `max`
async fn read(mut r: reqwest::Response, max: usize) -> Result<(u16, Value), String> {
  let code = r.status().as_u16();
  let mut buf = Vec::new();
  while let Some(c) = r.chunk().await.map_err(|e| super::http::offline("heatsync", e))? {
    if buf.len() + c.len() > max {
      return Err("heatsync sent back too much".into());
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

pub async fn run(http: &reqwest::Client, token: &str, cmd: EmoteCmd, out: &Out) {
  match cmd {
    EmoteCmd::Block { hash } => {
      if !valid_hash(&hash) {
        return;
      }
      let r = http
        .post(format!("{ORIGIN}/api/user/emotes/block"))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(json!({ "emote_hash": hash }).to_string())
        .send()
        .await;
      if let Err(e) = result(r).await {
        out.status(format!("blocked here, but sync failed: {e}"));
      }
    }
    EmoteCmd::Unblock { hash } => {
      if !valid_hash(&hash) {
        return;
      }
      let r = http
        .delete(format!("{ORIGIN}/api/user/emotes/blocked/{}", crate::util::path_seg(&hash)))
        .bearer_auth(token)
        .send()
        .await;
      // 404 = it was never blocked on the server: nothing to undo
      if let Err(e) = result(r).await
        && e != super::http::server_words("Block not found")
      {
        out.status(format!("unblocked here, but sync failed: {e}"));
      }
    }
    EmoteCmd::Add { name, url, hash, zero_width } => {
      if !valid_name(&name) || !allowed(&url) {
        return out.status("can't add that emote");
      }
      match add(http, token, &name, &url, hash.as_deref(), zero_width, false).await {
        Ok(()) => {
          out.status(format!("added {name} to your emotes"));
          out.send(NetEvent::Inventory { removed: None });
        }
        Err(e) => out.status(format!("couldn't add {name}: {e}")),
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
        return out.status("can't add that emote");
      }
      // one request: the server points your slot at the new emote, so a failure leaves the old one
      match add(http, token, &name, &url, None, zero_width, true).await {
        Ok(()) => {
          out.status(format!("your {name} is now the one you picked"));
          out.send(NetEvent::Inventory { removed: Some(name) });
        }
        Err(e) if e.contains("held by your upload") => {
          out.status(format!("kept your own upload as {name}: an upload never gets swapped out"))
        }
        Err(e) => out.status(format!("couldn't swap {name}, you still have your old one: {e}")),
      }
    }
    EmoteCmd::Remove { name } => match remove(http, token, &name).await {
      Ok(()) => {
        out.status(format!("removed {name} from your emotes"));
        out.send(NetEvent::Inventory { removed: Some(name) });
      }
      Err(e) => out.status(format!("couldn't remove {name}: {e}")),
    },
  }
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
) -> Result<(), String> {
  let r = http
    .post(format!("{ORIGIN}/api/user/emotes"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(add_body(name, url, hash, zero_width, replace).to_string())
    .send()
    .await;
  result(r).await.map(drop)
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

async fn result(r: Result<reqwest::Response, reqwest::Error>) -> Result<Value, String> {
  let (code, v) = read(r.map_err(|e| super::http::offline("heatsync", e))?, MAX_LIST).await?;
  if ok(code) { Ok(v) } else { Err(why(code, &v)) }
}

// the slot is how the server names an inventory row: look it up by name, then delete it
async fn remove(http: &reqwest::Client, token: &str, name: &str) -> Result<(), String> {
  let r = http.get(format!("{ORIGIN}/api/user/emotes")).bearer_auth(token).send().await;
  let v = result(r).await?;
  let slot = v.get("emotes").and_then(Value::as_array).and_then(|rows| slot_of(rows, name));
  let Some(slot) = slot else { return Err("not in your emotes".into()) };
  let r = http.delete(format!("{ORIGIN}/api/user/emotes/{slot}")).bearer_auth(token).send().await;
  match result(r).await {
    Err(e) if e.to_lowercase().contains("not found in your set") => Ok(()),
    other => other.map(drop),
  }
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
