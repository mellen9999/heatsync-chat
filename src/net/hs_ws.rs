// heatsync's anonymous websocket: kick + youtube chat relay, stream/pin/moment events.
// hardening per docs/contract.md: backoff reset only after a stable session, idle watchdog,
// server:shutdown spread without a backoff charge, kick `_seq` resume, yt re-subscribe on reconnect
use crate::model::ident;
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Message as Ws;

use super::http::ws_config;
use super::{NetEvent, Out, backoff, retrying};
use super::{pin, relay, senders};
use crate::cosmetics::id::PaintId;
use crate::heatsync::valid_name;
use crate::model::message::Platform;
use crate::model::stream_events::{self, Ev, Gate, Kind};

const URL: &str = "wss://heatsync.org/ws?b=1";
const HEARTBEAT: Duration = Duration::from_secs(25);
const IDLE_LIMIT: Duration = Duration::from_secs(90);
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(30);
const STABLE: Duration = Duration::from_secs(30);
// a `batch` of 256 chat lines or a kick backfill is the biggest legit frame
const MAX_FRAME: usize = 2 << 20;
const MAX_BATCH: usize = 256;
const MAX_ROOMS: usize = 50;

pub enum Cmd {
  Join(Platform, Box<str>),
  Leave(Platform, Box<str>),
  YtSub(Box<str>),
  YtUnsub(Box<str>),
  // sign the socket in with the heatsync session (None = signed out)
  Auth(Option<String>),
  // settings you changed, for the account (the patch is only keys model::sync wrote)
  UiSync(Value),
}

fn plat(p: Platform) -> &'static str {
  p.name()
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
  // a socket has run before, so the next one is a comeback
  resumed: bool,
  // youtube feeds this socket re-subscribed after a drop: their replayed backlog fills the gap
  // (lowercase). on a first socket the replay is dropped, history comes from /api/recent
  catching: BTreeSet<Box<str>>,
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
  // the user was told heatsync can't be reached, so they're told when it's back
  let mut told = false;
  loop {
    let mut wait = None;
    match timeout(HANDSHAKE_LIMIT, connect_async_with_config(URL, Some(ws_config(MAX_FRAME)), false)).await {
      Ok(Ok((ws, _))) => {
        let started = Instant::now();
        if std::mem::take(&mut told) {
          out.status("heatsync is back");
        }
        let ended = session(ws, &mut rx, &mut st, &pins, &out).await;
        st.resumed = true;
        // nothing is signed in on the next socket until it says so
        out.send(NetEvent::HsAuth(false));
        match ended {
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
      Ok(Err(e)) => eprintln!("heatsync connect: {e}"),
      Err(_) => eprintln!("heatsync connect: handshake timed out"),
    }
    let wait = wait.unwrap_or_else(|| {
      attempts += 1;
      let wait = backoff(attempts);
      // a drop that connects again at once is not worth a line; trouble connecting is
      if attempts > 1 {
        told = true;
        retrying(&out, "can't reach heatsync", attempts - 1, wait);
      }
      wait
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

// update desired state; returns the frames to send if connected
fn apply(st: &mut State, c: Cmd) -> Vec<Value> {
  match c {
    Cmd::Join(p, ch) if ident::relay_key(&ch) && st.rooms.len() < MAX_ROOMS => {
      if st.rooms.insert((plat(p), ch.clone())) { join(plat(p), &ch) } else { Vec::new() }
    }
    Cmd::Leave(p, ch) => {
      if p == Platform::Kick {
        st.kick_seq.remove(&ch);
      }
      if !st.rooms.remove(&(plat(p), ch.clone())) {
        return Vec::new();
      }
      let mut out = vec![json!({"type": "channel:leave", "platform": plat(p), "channel": &*ch})];
      if p == Platform::Twitch {
        out.push(json!({"type": "irc:part", "channel": &*ch}));
      }
      out
    }
    Cmd::Auth(t) => {
      st.token = t;
      Vec::new()
    }
    Cmd::UiSync(patch) => vec![json!({"type": "ui-state:sync", "patch": patch})],
    Cmd::YtSub(h) if ident::relay_key(&h) => st.yt.insert(h.clone()).then(|| yt_sub(&h)).into_iter().collect(),
    Cmd::YtUnsub(h) => {
      st.yt.remove(&h);
      st.yt_status.remove(&h);
      // without a videoId the server ref-counts and reaps on drop
      Vec::new()
    }
    _ => Vec::new(),
  }
}

// a room's join. a twitch room also joins heatsync's own copy of its chat (`irc:join`): its lines carry
// the sender's heatsync refs (`hsEmotes`), stamped at fanout, which our own irc copy lacks
fn join(p: &str, ch: &str) -> Vec<Value> {
  let mut out = vec![json!({"type": "channel:join", "platform": p, "channel": ch})];
  if p == "twitch" && ident::twitch_room(ch) {
    out.push(json!({"type": "irc:join", "channel": ch}));
  }
  out
}

// heatsync's copy of a twitch line, read only for its refs: most carry none, and on a busy channel every
// line comes twice now, so those are dropped before parsing (the server writes `type` first)
fn refless(text: &str) -> bool {
  text.starts_with(r#"{"type":"irc:message""#) && !text.contains(r#""hsEmotes""#)
}

// the youtube feeds a new socket subscribes again after an earlier one ran (lowercase, as frames name them)
fn resubscribed(st: &State) -> BTreeSet<Box<str>> {
  if st.resumed { st.yt.iter().map(|h| h.to_ascii_lowercase().into()).collect() } else { BTreeSet::new() }
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
  hello.extend(st.rooms.iter().flat_map(|(p, c)| join(p, c)));
  let acks: Vec<Value> =
    st.kick_seq.iter().take(200).map(|(c, s)| json!({"channel": format!("kick/{c}"), "lastSeq": s})).collect();
  if !acks.is_empty() {
    hello.push(json!({"type": "ack:channels", "channels": acks}));
  }
  hello.extend(st.yt.iter().map(|h| yt_sub(h)));
  st.catching = resubscribed(st);
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
        if refless(&text) {
          continue;
        }
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
        let frames = apply(st, c);
        if let Some(a) = joined.filter(|_| !frames.is_empty()) {
          let _ = pins.send(a);
        }
        for f in frames {
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
          out.status("heatsync went quiet, reconnecting");
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
  v.get("_ch").and_then(Value::as_str)?.strip_prefix("kick/").filter(|c| ident::relay_key(c)).map(Into::into)
}

// whose cosmetics changed, from a `cosmetic:changed` frame. a bare id is a users.id: a twitch chatter,
// or a heatsync-native account with the same digits, so it refreshes both (never kick/yt, which are prefixed)
pub(crate) fn cosmetic_ids(v: &Value) -> Vec<PaintId> {
  let list = v.get("ids").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  let mut out = Vec::new();
  for s in list.iter().take(MAX_BATCH).filter_map(Value::as_str) {
    let Some(id) = PaintId::from_wire(s) else { continue };
    if matches!(id, PaintId::Twitch(_)) {
      out.push(PaintId::Native(s.into()));
    }
    out.push(id);
  }
  out
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
        .or_else(|| data.get("channel").and_then(Value::as_str).filter(|c| ident::relay_key(c)).map(Into::into))?;
      // only rooms we are in: a stray frame must not pin a slot in the (capped) resume map
      if let (Some(seq), true) = (v.get("_seq").and_then(Value::as_u64), st.rooms.contains(&("kick", key.clone()))) {
        st.kick_seq.insert(key.clone(), seq);
      }
      let m = relay::kick(data)?;
      send(NetEvent::Relay { platform: Platform::Kick, key, msgs: vec![m], history: false })
    }
    "kick-chat-backfill" => {
      let key: Box<str> = v.get("channel").and_then(Value::as_str).filter(|c| ident::relay_key(c))?.into();
      let msgs = relay::batch(v.get("messages"), relay::kick);
      send(NetEvent::Relay { platform: Platform::Kick, key, msgs, history: true })
    }
    "kick-moderation" => {
      let key: Box<str> = v.get("channel").and_then(Value::as_str).filter(|c| ident::relay_key(c))?.into();
      let login: Box<str> = v.get("username").and_then(Value::as_str)?.to_lowercase().into();
      send(NetEvent::RelayClear { platform: Platform::Kick, key, login })
    }
    "youtube:chat" => {
      let key: Box<str> = v.get("channelId").and_then(Value::as_str).filter(|c| ident::relay_key(c))?.into();
      // a fresh subscribe replays the whole backlog; history comes from /api/recent instead. after a
      // drop that backlog is the way to see what was said meanwhile: it fills the gap, by id
      if v.get("replay").and_then(Value::as_bool) == Some(true) {
        if !st.catching.contains(&*key.to_ascii_lowercase()) {
          return None;
        }
        let msgs: Vec<_> =
          relay::batch(v.get("messages"), relay::youtube).into_iter().filter(|m| !m.id.is_empty()).collect();
        return (!msgs.is_empty())
          .then(|| send(NetEvent::CatchUp { platform: Platform::Youtube, key, msgs }))
          .flatten();
      }
      let msgs = relay::batch(v.get("messages"), relay::youtube);
      send(NetEvent::Relay { platform: Platform::Youtube, key, msgs, history: false })
    }
    "youtube:status" => {
      let key: Box<str> = v.get("channelId").and_then(Value::as_str).filter(|c| ident::relay_key(c))?.into();
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
    // a tabbed channel's room, and (`follow:`) a channel you follow but have no tab for: one gate for both.
    // the server also broadcasts plain `stream:*` to every socket for its live grid: only our rooms count
    "stream:online"
    | "stream:offline"
    | "stream:update"
    | "follow:stream:online"
    | "follow:stream:offline"
    | "follow:stream:update" => {
      let followed = ty.starts_with("follow:");
      let ty = ty.strip_prefix("follow:").unwrap_or(ty);
      let platform = room_platform(v.get("platform").and_then(Value::as_str)?)?;
      let channel: Box<str> = v.get("channel").and_then(Value::as_str)?.to_ascii_lowercase().into();
      if !ident::relay_key(&channel) || !(followed || st.rooms.contains(&(plat(platform), channel.clone()))) {
        return None;
      }
      let text = |k: &str| stream_events::clean(v.get(k).and_then(Value::as_str).unwrap_or(""));
      let kind = match ty {
        "stream:online" => Kind::Online,
        "stream:offline" => Kind::Offline,
        _ => Kind::Update,
      };
      // the name as they write it, only when it is their channel key in other letters
      let name = text("displayName");
      let name = if name.eq_ignore_ascii_case(&channel) { name } else { "".into() };
      let color = v.get("color").and_then(Value::as_str).and_then(crate::model::color::parse_hex);
      let ev = Ev { kind, platform, channel, game: text("game"), prev_game: text("prevGame"), name, color };
      st.gate.event(crate::util::time::now_ms(), ev).and_then(|l| send(NetEvent::Stream(l)))
    }
    "follow:colors" => {
      let colors = stream_events::follow_colors(v);
      (!colors.is_empty()).then(|| send(NetEvent::FollowColors(colors))).flatten()
    }
    "moment:spike" => {
      let platform = room_platform(v.get("platform").and_then(Value::as_str).unwrap_or("twitch"))?;
      let key: Box<str> = v.get("channel").and_then(Value::as_str)?.to_ascii_lowercase().into();
      let num = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN);
      if !ident::relay_key(&key) {
        return None;
      }
      let text = stream_events::moment_text(&key, num("rate"), num("baseline"))?.into();
      send(NetEvent::Moment { platform, key, text })
    }
    "pin:set" | "pin:clear" => pin::frame(v, ty == "pin:set").and_then(send),
    // pushed only to the moderator's own sockets
    "automod:hold" => {
      let h = crate::model::held::hold(v, crate::util::time::now_ms())?;
      send(NetEvent::Held(Box::new(h)))
    }
    "automod:update" => {
      let (chan, id, end) = crate::model::held::update(v)?;
      send(NetEvent::HeldEnd { chan, id, end })
    }
    "new-message" => {
      let p = crate::model::post::parse(v.get("data")?)?;
      send(NetEvent::FeedLive(p))
    }
    "mute:added" | "mute:removed" | "mute:expired" | "mute:cleared" => {
      super::mutes::frame(v, ty).and_then(|m| send(NetEvent::MuteSync(m)))
    }
    "seen:update" => {
      use crate::model::unseen::{Surface, at};
      let surface = Surface::parse(v.get("surface").and_then(Value::as_str)?)?;
      send(NetEvent::Seen { surface, at: at(v.get("at"))? })
    }
    "authenticated" => send(NetEvent::HsAuth(true)),
    // heatsync's copy of a twitch line in a room we're in: only its id and refs are kept (our own irc
    // has the line; it is never shown twice)
    "irc:message" => {
      let ch = v.get("channel").and_then(Value::as_str).filter(|c| ident::twitch_room(c))?;
      if !st.rooms.contains(&("twitch", ch.into())) {
        return None;
      }
      let m = v.get("message")?;
      let id = m.get("id").and_then(Value::as_str).filter(|i| (1..=100).contains(&i.len()))?;
      let refs = relay::hs_emotes(m.get("hsEmotes"));
      (!refs.is_empty()).then(|| send(NetEvent::HsRefs { chan: ch.into(), id: id.into(), refs })).flatten()
    }
    // your settings as the account holds them (the full state from a write, or the changed keys from the site)
    "ui-state:update" => {
      let (state, _) = crate::model::sync::pluck(v.get("state")?);
      (!state.is_empty()).then(|| send(NetEvent::UiState(state))).flatten()
    }
    "whisper:new" => {
      let w = crate::model::whisper::parse(v, 0, None)?;
      send(NetEvent::Whispers { items: vec![w], live: true, epoch: None })
    }
    // a heatsync dm to you (`dm:sent` is our own send echoing to our other clients: not shown)
    "dm:new" => {
      let w = crate::model::whisper::parse_dm(v, None)?;
      send(NetEvent::Whispers { items: vec![w], live: true, epoch: None })
    }
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_followed_channel_going_live_is_a_stream_line() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    st.gate.start(0);
    let v = json!({"type": "follow:stream:online", "platform": "twitch", "channel": "Ennortix", "game": "Chess", "title": "x"});
    assert!(dispatch(&v, &mut st, &out).is_none());
    let Ok(NetEvent::Stream(l)) = rx.try_recv() else { panic!("no line") };
    assert_eq!((&*l.channel, &*l.text), ("ennortix", "[ennortix] ◆ went live — Chess"));
    // the room reporting the same go-live is the same line: not shown twice
    apply(&mut st, Cmd::Join(Platform::Twitch, "ennortix".into()));
    let room = json!({"type": "stream:online", "platform": "twitch", "channel": "ennortix", "game": "Chess"});
    dispatch(&room, &mut st, &out);
    assert!(rx.try_recv().is_err());
  }

  #[test]
  fn a_live_grid_broadcast_for_a_channel_without_a_tab_is_dropped() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    st.gate.start(0);
    let grid = json!({"type": "stream:online", "platform": "twitch", "channel": "zbra", "game": "Overwatch"});
    dispatch(&grid, &mut st, &out);
    assert!(rx.try_recv().is_err(), "not followed, not tabbed: no line");
    apply(&mut st, Cmd::Join(Platform::Twitch, "zbra".into()));
    dispatch(&grid, &mut st, &out);
    assert!(matches!(rx.try_recv(), Ok(NetEvent::Stream(_))), "its own tab still hears it");
  }

  #[test]
  fn a_youtube_replay_fills_the_gap_after_a_drop_but_is_dropped_on_a_first_socket() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    apply(&mut st, Cmd::YtSub("@Chan".into()));
    let replay = json!({"type": "youtube:chat", "channelId": "@chan", "replay": true,
      "messages": [{"id": "m1", "user": "al", "text": "hi", "timestamp": 5}, {"user": "bo", "text": "no id"}]});
    // first socket: the backlog is history's job (/api/recent), not ours
    assert!(resubscribed(&st).is_empty());
    dispatch(&replay, &mut st, &out);
    assert!(rx.try_recv().is_err());
    // after a drop the same replay is what was said meanwhile: kept by id, the id-less line is not
    st.resumed = true;
    st.catching = resubscribed(&st);
    assert!(st.catching.contains("@chan"));
    dispatch(&replay, &mut st, &out);
    let Ok(NetEvent::CatchUp { platform, key, msgs }) = rx.try_recv() else { panic!("no catch-up") };
    assert_eq!((platform, &*key, msgs.len()), (Platform::Youtube, "@chan", 1));
    assert_eq!(&*msgs[0].id, "m1");
    // a feed that wasn't re-subscribed (added since) is still dropped, and live batches go as before
    let other = json!({"type": "youtube:chat", "channelId": "@new", "replay": true,
      "messages": [{"id": "x", "user": "al", "text": "hi"}]});
    dispatch(&other, &mut st, &out);
    assert!(rx.try_recv().is_err());
    let live =
      json!({"type": "youtube:chat", "channelId": "@chan", "messages": [{"id": "m2", "user": "al", "text": "yo"}]});
    dispatch(&live, &mut st, &out);
    assert!(matches!(rx.try_recv(), Ok(NetEvent::Relay { history: false, .. })));
  }

  #[test]
  fn apply_validates_and_dedupes() {
    let mut st = State::default();
    assert_eq!(apply(&mut st, Cmd::Join(Platform::Kick, "xqc".into())).len(), 1);
    assert!(apply(&mut st, Cmd::Join(Platform::Kick, "xqc".into())).is_empty());
    assert!(apply(&mut st, Cmd::Join(Platform::Kick, "bad name".into())).is_empty());
    assert_eq!(apply(&mut st, Cmd::YtSub("@handle".into())).len(), 1);
    let f = apply(&mut st, Cmd::Leave(Platform::Kick, "xqc".into()));
    assert_eq!((f.len(), &f[0]["type"]), (1, &json!("channel:leave")));
  }

  #[test]
  fn cosmetic_changed_keeps_each_id_in_its_own_namespace() {
    let v = json!({"type": "cosmetic:changed", "ids": ["37005319", "kick_5", "yt_UC1", 7, "x"]});
    // a bare id refreshes the twitch chatter and a native account of that id; kick_5 only kick 5
    assert_eq!(cosmetic_ids(&v), vec![PaintId::Native("37005319".into()), PaintId::Twitch(37005319), PaintId::Kick(5)]);
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

  // a twitch room joins heatsync's copy of its chat too, and leaves it with the room
  #[test]
  fn a_twitch_room_joins_and_parts_heatsyncs_irc_copy() {
    let mut st = State::default();
    let types = |f: &[Value]| f.iter().map(|f| f["type"].as_str().unwrap_or("").to_owned()).collect::<Vec<_>>();
    let f = apply(&mut st, Cmd::Join(Platform::Twitch, "forsen".into()));
    assert_eq!(types(&f), ["channel:join", "irc:join"]);
    assert_eq!(f[1]["channel"], "forsen");
    assert_eq!(types(&apply(&mut st, Cmd::Leave(Platform::Twitch, "forsen".into()))), ["channel:leave", "irc:part"]);
    assert_eq!(types(&apply(&mut st, Cmd::Join(Platform::Kick, "forsen".into()))), ["channel:join"]);
  }

  // a server copy of a twitch line hands over its refs by id; one without refs is never parsed, one for a
  // room we're not in or with a hostile ref is dropped
  #[test]
  fn an_irc_copy_hands_over_its_refs() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    apply(&mut st, Cmd::Join(Platform::Twitch, "forsen".into()));
    let frame = |ch: &str, refs: Value| {
      json!({"type": "irc:message", "channel": ch, "message": {"type": "privmsg", "id": "abc", "username": "bob",
        "content": "kek hi", "hsEmotes": refs}})
    };
    let good = json!({"kek": {"url": "https://cdn.7tv.app/emote/x/1x.webp", "provider": "7tv", "zeroWidth": true},
      "evil": {"url": "https://evil.example/x.png"}});
    dispatch(&frame("forsen", good.clone()), &mut st, &out);
    let Ok(NetEvent::HsRefs { chan, id, refs }) = rx.try_recv() else { panic!("no refs") };
    assert_eq!((&*chan, &*id), ("forsen", "abc"));
    assert_eq!(&*refs, [("kek".into(), "https://cdn.7tv.app/emote/x/1x.webp".into(), true)]);
    dispatch(&frame("xqc", good), &mut st, &out);
    assert!(rx.try_recv().is_err(), "not our room");
    assert!(refless(r#"{"type":"irc:message","channel":"forsen","message":{"id":"x","content":"hi"}}"#));
    assert!(!refless(&frame("forsen", json!({})).to_string()));
    assert!(!refless(r#"{"type":"kick-chat-message"}"#));
  }
}
