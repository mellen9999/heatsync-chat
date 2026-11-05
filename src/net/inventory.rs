// your heatsync inventory, managed (the ext's picker "mine" tab menu, its removed and sets tabs, undo/redo):
// list, rename, move, remove, undo, redo, restore, apply a saved set, share, import a channel. each op is
// one request; the answer is checked and handed to the screen. a change that lands also tells the sender
// path (`NetEvent::Inventory`), so your own lines draw the new set through the same refetch as ever
use reqwest::Method;
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out};
use crate::model::inventory::{self, Gone, Saved, Slot};

// 5000 rows of a long name and a url each, with room
const MAX_LIST: usize = 8 << 20;
const MAX_REPLY: usize = 256 << 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
  List,
  Removed,
  Sets,
  // (slot, its name now, the new name)
  Rename(u32, Box<str>, Box<str>),
  Move(u32, u32),
  // (slot, its name)
  Remove(u32, Box<str>),
  Undo,
  Redo,
  Restore(u64),
  Apply(u64),
  Share,
  Unshare,
  Import(Box<str>),
}

#[derive(Debug, PartialEq)]
pub enum Got {
  List(Vec<Slot>),
  Removed(Vec<Gone>),
  Sets(Vec<Saved>),
  Renamed(u32, Box<str>),
  Moved(u32, u32),
  Dropped(u32, Box<str>),
  // undo, redo, restore, apply, import: the inventory changed in ways only a fresh read shows. the line
  // says what happened
  Changed(String),
  Shared(String),
  Unshared,
}

// one request, apart from sending it
pub struct Req {
  pub method: Method,
  pub path: String,
  pub body: Option<Value>,
}

fn req(method: Method, path: impl Into<String>, body: Option<Value>) -> Req {
  Req { method, path: path.into(), body }
}

pub fn request(op: &Op) -> Req {
  match op {
    Op::List => req(Method::GET, "/api/user/emotes", None),
    Op::Removed => req(Method::GET, "/api/user/emotes/removed", None),
    Op::Sets => req(Method::GET, "/api/user/sets", None),
    Op::Rename(slot, _, name) => {
      req(Method::PUT, format!("/api/user/emotes/{slot}/rename"), Some(json!({ "newName": name })))
    }
    Op::Move(from, to) => req(Method::PUT, format!("/api/user/emotes/{from}/move"), Some(json!({ "toSlot": to }))),
    Op::Remove(slot, _) => req(Method::DELETE, format!("/api/user/emotes/{slot}"), None),
    Op::Undo => req(Method::POST, "/api/user/emotes/undo", None),
    Op::Redo => req(Method::POST, "/api/user/emotes/redo", None),
    Op::Restore(id) => req(Method::POST, format!("/api/user/emotes/removed/{id}/restore"), None),
    Op::Apply(id) => req(Method::POST, format!("/api/user/sets/{id}/apply"), None),
    Op::Share => req(Method::POST, "/api/user/emotes/share", Some(json!({}))),
    Op::Unshare => req(Method::DELETE, "/api/user/emotes/share", None),
    Op::Import(channel) => req(Method::POST, "/api/user/emotes/import-channel", Some(json!({ "channel": channel }))),
  }
}

// the plain words for a settled op, from the server's own numbers where it sent them
fn changed(op: &Op, v: &Value) -> String {
  let n = |k: &str| v.get(k).and_then(Value::as_u64);
  match op {
    Op::Undo => "undone".into(),
    Op::Redo => "redone".into(),
    Op::Restore(_) => "restored".into(),
    Op::Apply(_) => match n("applied") {
      Some(c) => format!("set applied: {c} emotes (your old ones are in removed)"),
      None => "set applied (your old ones are in removed)".into(),
    },
    Op::Import(c) => format!("imported {} from {c}", n("imported").map_or_else(|| "emotes".into(), |i| i.to_string())),
    _ => String::new(),
  }
}

// a share link: the server's own, or built from its short id; only ever heatsync's
fn link(v: &Value) -> Option<String> {
  let id = v.get("shortId").and_then(Value::as_str).filter(|s| crate::model::ident::slug(s, 32))?;
  Some(format!("https://heatsync.org/set/{id}"))
}

fn answer(op: Op, v: &Value) -> Result<Got, String> {
  Ok(match op {
    Op::List => Got::List(inventory::slots(v)),
    Op::Removed => Got::Removed(inventory::gone(v)),
    Op::Sets => Got::Sets(inventory::saved(v)),
    Op::Rename(slot, _, name) => Got::Renamed(slot, v.get("name").and_then(Value::as_str).map_or(name, Into::into)),
    Op::Move(from, to) => Got::Moved(from, to),
    Op::Remove(slot, name) => Got::Dropped(slot, name),
    Op::Share => Got::Shared(link(v).ok_or("heatsync sent back no link")?),
    Op::Unshare => Got::Unshared,
    ref o => Got::Changed(changed(o, v)),
  })
}

// the name a change makes obsolete, for the sender path's tombstone
fn gone_name(op: &Op) -> Option<Box<str>> {
  match op {
    Op::Rename(_, old, _) => Some(old.clone()),
    Op::Remove(_, name) => Some(name.clone()),
    _ => None,
  }
}

// does this op change the inventory (so your own lines should refetch it)
fn writes(op: &Op) -> bool {
  !matches!(op, Op::List | Op::Removed | Op::Sets | Op::Share | Op::Unshare)
}

pub async fn run(http: reqwest::Client, token: Option<String>, op: Op, out: Out) {
  let res = match token {
    None => Err(super::http::plain_status(401)),
    Some(t) => ask(&http, &t, op.clone()).await,
  };
  if res.is_ok() && writes(&op) {
    out.send(NetEvent::Inventory { removed: gone_name(&op) });
  }
  out.send(NetEvent::Inv { op, res });
}

async fn ask(http: &reqwest::Client, token: &str, op: Op) -> Result<Got, String> {
  let q = request(&op);
  let mut b = http.request(q.method, format!("{ORIGIN}{}", q.path)).bearer_auth(token);
  if let Some(body) = q.body {
    b = b.header("content-type", "application/json").body(body.to_string());
  }
  let r = b.send().await.map_err(|e| super::http::offline("heatsync", e))?;
  let code = r.status().as_u16();
  let max = if matches!(op, Op::List) { MAX_LIST } else { MAX_REPLY };
  let body = super::http::read(r, max).await?;
  let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
  if !super::emote_ops::ok(code) {
    return Err(super::emote_ops::why(code, &v));
  }
  answer(op, &v)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn each_op_is_the_ext_s_request() {
    let r = request(&Op::Rename(12, "old".into(), "PogU".into()));
    assert_eq!(
      (r.method, r.path.as_str(), r.body),
      (Method::PUT, "/api/user/emotes/12/rename", Some(json!({"newName": "PogU"})))
    );
    let r = request(&Op::Move(3, 40));
    assert_eq!(
      (r.method, r.path.as_str(), r.body),
      (Method::PUT, "/api/user/emotes/3/move", Some(json!({"toSlot": 40})))
    );
    let r = request(&Op::Remove(7, "x".into()));
    assert_eq!((r.method, r.path.as_str(), r.body), (Method::DELETE, "/api/user/emotes/7", None));
    let r = request(&Op::Restore(77));
    assert_eq!((r.method, r.path.as_str()), (Method::POST, "/api/user/emotes/removed/77/restore"));
    let r = request(&Op::Apply(9));
    assert_eq!((r.method, r.path.as_str()), (Method::POST, "/api/user/sets/9/apply"));
    let r = request(&Op::Import("forsen".into()));
    assert_eq!(r.body, Some(json!({"channel": "forsen"})));
    assert_eq!(request(&Op::Undo).path, "/api/user/emotes/undo");
    assert_eq!(request(&Op::Redo).path, "/api/user/emotes/redo");
    let (s, u) = (request(&Op::Share), request(&Op::Unshare));
    assert_eq!((s.method, u.method, u.path.as_str()), (Method::POST, Method::DELETE, "/api/user/emotes/share"));
    assert_eq!(request(&Op::Sets).path, "/api/user/sets");
    assert_eq!(request(&Op::Removed).path, "/api/user/emotes/removed");
    assert_eq!(request(&Op::List).path, "/api/user/emotes");
  }

  #[test]
  fn answers_are_read_and_a_link_is_only_ever_heatsync_s() {
    assert_eq!(
      answer(Op::Share, &json!({"shortId": "AbCd1234", "url": "https://evil.example/x"})),
      Ok(Got::Shared("https://heatsync.org/set/AbCd1234".into()))
    );
    assert!(answer(Op::Share, &json!({"shortId": "a/b"})).is_err());
    assert!(answer(Op::Share, &json!({"url": "https://heatsync.org/set/x"})).is_err());
    // the server's sanitised name wins over the typed one
    assert_eq!(answer(Op::Rename(1, "a".into(), "B ".into()), &json!({"name": "B"})), Ok(Got::Renamed(1, "B".into())));
    assert_eq!(answer(Op::Rename(1, "a".into(), "B".into()), &json!(null)), Ok(Got::Renamed(1, "B".into())));
    assert_eq!(answer(Op::Remove(2, "x".into()), &json!(null)), Ok(Got::Dropped(2, "x".into())));
    assert_eq!(
      answer(Op::Apply(1), &json!({"applied": 12})),
      Ok(Got::Changed("set applied: 12 emotes (your old ones are in removed)".into()))
    );
    assert_eq!(
      answer(Op::Import("forsen".into()), &json!({"imported": 96})),
      Ok(Got::Changed("imported 96 from forsen".into()))
    );
  }

  #[test]
  fn only_a_write_wakes_the_sender_path_and_names_what_left() {
    for op in [Op::List, Op::Removed, Op::Sets, Op::Share, Op::Unshare] {
      assert!(!writes(&op), "{op:?}");
    }
    for op in [Op::Undo, Op::Redo, Op::Move(1, 2), Op::Restore(1), Op::Apply(1), Op::Import("a".into())] {
      assert!(writes(&op) && gone_name(&op).is_none(), "{op:?}");
    }
    assert_eq!(gone_name(&Op::Rename(1, "old".into(), "new".into())).as_deref(), Some("old"));
    assert_eq!(gone_name(&Op::Remove(1, "x".into())).as_deref(), Some("x"));
  }
}
