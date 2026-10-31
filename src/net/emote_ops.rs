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
}

// body capped while streaming, so a hostile server can't make us buffer more than `max`
async fn read(mut r: reqwest::Response, max: usize) -> Result<(u16, Value), String> {
  let code = r.status().as_u16();
  let mut buf = Vec::new();
  while let Some(c) = r.chunk().await.map_err(|e| e.to_string())? {
    if buf.len() + c.len() > max {
      return Err("response too large".into());
    }
    buf.extend_from_slice(&c);
  }
  Ok((code, serde_json::from_slice(&buf).unwrap_or(Value::Null)))
}

fn why(code: u16, v: &Value) -> String {
  v.get("error").and_then(Value::as_str).map_or_else(|| format!("http {code}"), |e| e.chars().take(100).collect())
}

fn ok(code: u16) -> bool {
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
      let r = http.delete(format!("{ORIGIN}/api/user/emotes/blocked/{hash}")).bearer_auth(token).send().await;
      // 404 = it was never blocked on the server: nothing to undo
      if let Err(e) = result(r).await
        && e != "Block not found"
      {
        out.status(format!("unblocked here, but sync failed: {e}"));
      }
    }
    EmoteCmd::Add { name, url, hash, zero_width } => {
      if !valid_name(&name) || !allowed(&url) {
        return out.status("can't add that emote");
      }
      let body = json!({
        "emoteUrl": &*url, "emoteName": &*name, "customName": &*name,
        "source": "extension", "sourceId": hash, "zeroWidth": zero_width,
      });
      let r = http
        .post(format!("{ORIGIN}/api/user/emotes"))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await;
      match result(r).await {
        Ok(_) => {
          out.status(format!("added {name} to your emotes"));
          out.send(NetEvent::Inventory { removed: None });
        }
        Err(e) => out.status(format!("couldn't add {name}: {e}")),
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

async fn result(r: Result<reqwest::Response, reqwest::Error>) -> Result<Value, String> {
  let (code, v) = read(r.map_err(|e| e.to_string())?, MAX_LIST).await?;
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
