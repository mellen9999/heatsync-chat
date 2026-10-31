// heatsync's anonymous websocket: kick + youtube chat relay, stream/pin/moment events.
// hardening per docs/contract.md: backoff reset only after a stable session, idle watchdog,
// server:shutdown spread without a backoff charge, kick `_seq` resume, yt re-subscribe on reconnect
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as Ws;

use super::{NetEvent, Out, backoff};
use super::{pin, relay, senders};
use crate::heatsync::valid_name;
use crate::model::message::Platform;
use crate::model::stream_events::{self, Ev, Gate, Kind};

const URL: &str = "wss://heatsync.org/ws?b=1";
const HEARTBEAT: Duration = Duration::from_secs(25);
const IDLE_LIMIT: Duration = Duration::from_secs(90);
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(30);
const STABLE: Duration = Duration::from_secs(30);
const MAX_BATCH: usize = 256;
const MAX_ROOMS: usize = 50;

pub enum Cmd {
  Join(Platform, Box<str>),
  Leave(Platform, Box<str>),
  YtSub(Box<str>),
  YtUnsub(Box<str>),
  // sign the socket in with the heatsync session (None = signed out)
  Auth(Option<String>),
}

fn plat(p: Platform) -> &'static str {
  match p {
    Platform::Twitch => "twitch",
    Platform::Kick => "kick",
    Platform::Youtube => "youtube",
  }
}

pub fn valid_key(k: &str) -> bool {
  (1..=100).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.@".contains(&b))
}

#[derive(Default)]
struct State {
  rooms: BTreeSet<(&'static str, Box<str>)>,
  yt: BTreeSet<Box<str>>,
  // last `_seq` seen per kick room, so the server can replay what we missed
  kick_seq: HashMap<Box<str>, u64>,
  yt_status: HashMap<Box<str>, Box<str>>,
  gate: Gate,
  token: Option<String>,
}

enum End {
  // ui gone: stop for good
  Quit,
  Reconnect,
  // server restarting: come back after a random slice of this many ms
  Shutdown(u64),
}

pub async fn run(mut rx: mpsc::UnboundedReceiver<Cmd>, pins: mpsc::UnboundedSender<pin::Ask>, out: Out) {
  let mut st = State::default();
  st.gate.start(crate::util::time::now_ms());
  let mut attempts = 0u32;
  loop {
    let mut wait = None;
    match timeout(HANDSHAKE_LIMIT, connect_async(URL)).await {
      Ok(Ok((ws, _))) => {
        let started = Instant::now();
        match session(ws, &mut rx, &mut st, &pins, &out).await {
          End::Quit => return,
          End::Shutdown(spread) => {
            let ms = crate::util::time::now_ms().unsigned_abs() % spread.clamp(1000, 60_000);
            wait = Some(Duration::from_millis(ms));
          }
          End::Reconnect => {}
        }
        if started.elapsed() >= STABLE {
          attempts = 0;
        }
      }
      Ok(Err(e)) => out.status(format!("heatsync connect failed: {e}")),
      Err(_) => out.status("heatsync handshake timed out"),
    }
    let wait = wait.unwrap_or_else(|| {
      attempts += 1;
      backoff(attempts)
    });
    let t = sleep(wait);
    tokio::pin!(t);
    loop {
      tokio::select! {
        _ = &mut t => break,
        c = rx.recv() => match c {
          Some(c) => { apply(&mut st, c); }
          None => return,
        },
      }
    }
  }
}

// update desired state; returns the frame to send if connected
fn apply(st: &mut State, c: Cmd) -> Option<Value> {
  match c {
    Cmd::Join(p, ch) if valid_key(&ch) && st.rooms.len() < MAX_ROOMS => st
      .rooms
      .insert((plat(p), ch.clone()))
      .then(|| json!({"type": "channel:join", "platform": plat(p), "channel": &*ch})),
    Cmd::Leave(p, ch) => {
      if p == Platform::Kick {
        st.kick_seq.remove(&ch);
      }
      st.rooms
        .remove(&(plat(p), ch.clone()))
        .then(|| json!({"type": "channel:leave", "platform": plat(p), "channel": &*ch}))
    }
    Cmd::Auth(t) => {
      st.token = t;
      None
    }
    Cmd::YtSub(h) if valid_key(&h) => st.yt.insert(h.clone()).then(|| yt_sub(&h)),
    Cmd::YtUnsub(h) => {
      st.yt.remove(&h);
      st.yt_status.remove(&h);
      // without a videoId the server ref-counts and reaps on drop
      None
    }
    _ => None,
  }
}

fn authenticate(token: &String) -> Value {
  json!({"type": "authenticate", "token": token})
}

fn yt_sub(handle: &str) -> Value {
  json!({"type": "youtube:subscribe", "url": handle, "channelId": handle.to_ascii_lowercase()})
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn session(
  ws: Socket,
  rx: &mut mpsc::UnboundedReceiver<Cmd>,
  st: &mut State,
  pins: &mpsc::UnboundedSender<pin::Ask>,
  out: &Out,
) -> End {
  let (mut tx, mut read) = ws.split();
  let mut hello: Vec<Value> = st.token.iter().map(authenticate).collect();
  hello.extend(st.rooms.iter().map(|(p, c)| json!({"type": "channel:join", "platform": p, "channel": &**c})));
  let acks: Vec<Value> =
    st.kick_seq.iter().take(200).map(|(c, s)| json!({"channel": format!("kick/{c}"), "lastSeq": s})).collect();
  if !acks.is_empty() {
    hello.push(json!({"type": "ack:channels", "channels": acks}));
  }
  hello.extend(st.yt.iter().map(|h| yt_sub(h)));
  for f in hello {
    if tx.send(Ws::text(f.to_string())).await.is_err() {
      return End::Reconnect;
    }
  }
  // a pin may have changed while we were away (or before the first connect)
  for (p, c) in &st.rooms {
    if let Some(plat) = room_platform(p) {
      let _ = pins.send((plat, c.clone()));
    }
  }
  let mut last_rx = Instant::now();
  let mut beat = interval(HEARTBEAT);
  beat.tick().await;
  loop {
    tokio::select! {
      frame = read.next() => {
        let text = match frame {
          Some(Ok(Ws::Text(t))) => t,
          Some(Ok(Ws::Close(_))) | None | Some(Err(_)) => return End::Reconnect,
          Some(Ok(_)) => { last_rx = Instant::now(); continue }
        };
        last_rx = Instant::now();
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        if v.get("type").and_then(Value::as_str) == Some("batch") {
          let list = v.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
          for f in list.iter().take(MAX_BATCH) {
            match dispatch(f, st, out) {
              Some(End::Reconnect) => {}
              Some(e) => return e,
              None => {}
            }
          }
        } else if let Some(e) = dispatch(&v, st, out) {
          return e;
        }
      }
      c = rx.recv() => {
        let Some(c) = c else { return End::Quit };
        if let Cmd::Auth(t) = c {
          let (changed, was) = (st.token != t, st.token.is_some());
          st.token = t;
          match &st.token {
            Some(t) if changed => {
              if tx.send(Ws::text(authenticate(t).to_string())).await.is_err() {
                return End::Reconnect;
              }
            }
            // a signed-out socket must not keep receiving someone's whispers: start a fresh one
            None if was => return End::Reconnect,
            _ => {}
          }
          continue;
        }
        let joined = if let Cmd::Join(p, ch) = &c { Some((*p, ch.clone())) } else { None };
        if let Some(f) = apply(st, c) {
          if let Some(a) = joined {
            let _ = pins.send(a);
          }
          if tx.send(Ws::text(f.to_string())).await.is_err() {
            return End::Reconnect;
          }
        }
      }
      _ = hold(st.gate.next_due()) => {
        for l in st.gate.due(crate::util::time::now_ms()) {
          if !out.send(NetEvent::Stream(l)) {
            return End::Quit;
          }
        }
      }
      _ = beat.tick() => {
        if last_rx.elapsed() > IDLE_LIMIT {
          out.status("heatsync silent, reconnecting");
          return End::Reconnect;
        }
        if tx.send(Ws::text(r#"{"type":"presence:heartbeat"}"#)).await.is_err() {
          return End::Reconnect;
        }
      }
    }
  }
}

// sleeps until a held game switch is due; forever when none is
async fn hold(due: Option<i64>) {
  match due {
    Some(at) => sleep(Duration::from_millis((at - crate::util::time::now_ms()).max(0) as u64)).await,
    None => std::future::pending().await,
  }
}

fn room_platform(p: &str) -> Option<Platform> {
  match p {
    "twitch" => Some(Platform::Twitch),
    "kick" => Some(Platform::Kick),
    _ => None,
  }
}

fn kick_room(v: &Value) -> Option<Box<str>> {
  v.get("_ch").and_then(Value::as_str)?.strip_prefix("kick/").filter(|c| valid_key(c)).map(Into::into)
}

// twitch ids out of a `cosmetic:changed` frame; kick_/yt_ ids belong to other platforms and are skipped
fn cosmetic_ids(v: &Value) -> Vec<u64> {
  let list = v.get("ids").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  list.iter().take(MAX_BATCH).filter_map(|i| i.as_str()?.parse().ok()).collect()
}

// Some(Quit|Shutdown) ends the session; everything else keeps reading
fn dispatch(v: &Value, st: &mut State, out: &Out) -> Option<End> {
  let ty = v.get("type").and_then(Value::as_str)?;
  let send = |ev: NetEvent| (!out.send(ev)).then_some(End::Quit);
  match ty {
    "server:shutdown" => {
      let spread = v.get("reconnectSpreadMs").and_then(Value::as_u64).unwrap_or(60_000);
      Some(End::Shutdown(spread))
    }
    "kick-chat-message" => {
      let data = v.get("data")?;
      let key = kick_room(v)
        .or_else(|| data.get("channel").and_then(Value::as_str).filter(|c| valid_key(c)).map(Into::into))?;
      // only rooms we are in: a stray frame must not pin a slot in the (capped) resume map
      if let (Some(seq), true) = (v.get("_seq").and_then(Value::as_u64), st.rooms.contains(&("kick", key.clone()))) {
        st.kick_seq.insert(key.clone(), seq);
      }
      let m = relay::kick(data)?;
      send(NetEvent::Relay { platform: Platform::Kick, key, msgs: vec![m], history: false })
    }
    "kick-chat-backfill" => {
      let key: Box<str> = v.get("channel").and_then(Value::as_str).filter(|c| valid_key(c))?.into();
      let msgs = relay::batch(v.get("messages"), relay::kick);
      send(NetEvent::Relay { platform: Platform::Kick, key, msgs, history: true })
    }
    "kick-moderation" => {
      let key: Box<str> = v.get("channel").and_then(Value::as_str).filter(|c| valid_key(c))?.into();
      let login: Box<str> = v.get("username").and_then(Value::as_str)?.to_lowercase().into();
      send(NetEvent::RelayClear { platform: Platform::Kick, key, login })
    }
    "youtube:chat" => {
      // a fresh subscribe replays the whole backlog; history comes from /api/recent instead
      if v.get("replay").and_then(Value::as_bool) == Some(true) {
        return None;
      }
      let key: Box<str> = v.get("channelId").and_then(Value::as_str).filter(|c| valid_key(c))?.into();
      let msgs = relay::batch(v.get("messages"), relay::youtube);
      send(NetEvent::Relay { platform: Platform::Youtube, key, msgs, history: false })
    }
    "youtube:status" => {
      let key: Box<str> = v.get("channelId").and_then(Value::as_str).filter(|c| valid_key(c))?.into();
      let status: Box<str> = v.get("status").and_then(Value::as_str)?.chars().take(32).collect::<String>().into();
      if st.yt_status.get(&key) == Some(&status) {
        return None;
      }
      if st.yt_status.len() >= MAX_ROOMS && !st.yt_status.contains_key(&key) {
        return None;
      }
      st.yt_status.insert(key.clone(), status.clone());
      send(NetEvent::YtStatus { key, status })
    }
    // another user's inventory changed: the viewer push carries `username` + `senderKeys`
    // (the owner's own sync carries `data` instead: not logged in yet, ignore)
    "emote:added" | "emote:removed" if v.get("username").is_some() => {
      let (ids, ver) = senders::stale(v);
      if ids.is_empty() {
        return None;
      }
      let removed = (ty == "emote:removed")
        .then(|| v.get("emoteName").and_then(Value::as_str).filter(|n| valid_name(n)).map(Into::into))
        .flatten();
      send(NetEvent::SendersStale { ids, ver, removed })
    }
    // a chatter's heatsync paint changed (ids only; the spec stays behind the entitlement gate): ask again
    "cosmetic:changed" => {
      let ids = cosmetic_ids(v);
      (!ids.is_empty()).then(|| send(NetEvent::CosmeticsChanged(ids))).flatten()
    }
    "stream:online" | "stream:offline" | "stream:update" => {
      let platform = room_platform(v.get("platform").and_then(Value::as_str)?)?;
      let channel: Box<str> = v.get("channel").and_then(Value::as_str)?.to_ascii_lowercase().into();
      if !valid_key(&channel) {
        return None;
      }
      let text = |k: &str| stream_events::clean(v.get(k).and_then(Value::as_str).unwrap_or(""));
      let kind = match ty {
        "stream:online" => Kind::Online,
        "stream:offline" => Kind::Offline,
        _ => Kind::Update,
      };
      let ev = Ev { kind, platform, channel, game: text("game"), prev_game: text("prevGame") };
      st.gate.event(crate::util::time::now_ms(), ev).and_then(|l| send(NetEvent::Stream(l)))
    }
    "moment:spike" => {
      let platform = room_platform(v.get("platform").and_then(Value::as_str).unwrap_or("twitch"))?;
      let key: Box<str> = v.get("channel").and_then(Value::as_str)?.to_ascii_lowercase().into();
      let num = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
      if !valid_key(&key) {
        return None;
      }
      let text = stream_events::moment_text(&key, num("rate"), num("baseline"))?.into();
      send(NetEvent::Moment { platform, key, text })
    }
    "pin:set" | "pin:clear" => pin::frame(v, ty == "pin:set").and_then(send),
    "new-message" => {
      let p = crate::model::post::parse(v.get("data")?)?;
      send(NetEvent::FeedLive(p))
    }
    "mute:added" | "mute:removed" | "mute:expired" | "mute:cleared" => {
      super::mutes::frame(v, ty).and_then(|m| send(NetEvent::MuteSync(m)))
    }
    "whisper:new" => {
      let w = crate::model::whisper::parse(v, 0, None)?;
      send(NetEvent::Whispers { items: vec![w], live: true, epoch: None })
    }
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn apply_validates_and_dedupes() {
    let mut st = State::default();
    assert!(apply(&mut st, Cmd::Join(Platform::Kick, "xqc".into())).is_some());
    assert!(apply(&mut st, Cmd::Join(Platform::Kick, "xqc".into())).is_none());
    assert!(apply(&mut st, Cmd::Join(Platform::Kick, "bad name".into())).is_none());
    assert!(apply(&mut st, Cmd::YtSub("@handle".into())).is_some());
    let f = apply(&mut st, Cmd::Leave(Platform::Kick, "xqc".into())).unwrap();
    assert_eq!(f["type"], "channel:leave");
  }

  #[test]
  fn cosmetic_changed_keeps_only_twitch_ids() {
    let v = json!({"type": "cosmetic:changed", "ids": ["37005319", "kick_5", "yt_UC1", 7, "x"]});
    assert_eq!(cosmetic_ids(&v), vec![37005319]);
    assert!(cosmetic_ids(&json!({"type": "cosmetic:changed"})).is_empty());
  }

  #[test]
  fn leaving_a_kick_room_forgets_its_resume_seq() {
    let mut st = State::default();
    apply(&mut st, Cmd::Join(Platform::Kick, "xqc".into()));
    st.kick_seq.insert("xqc".into(), 7);
    apply(&mut st, Cmd::Leave(Platform::Kick, "xqc".into()));
    assert!(st.kick_seq.is_empty());
  }
}
