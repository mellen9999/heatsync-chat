// heatsync's anonymous websocket: kick + youtube chat relay, stream/pin/moment events.
// hardening per docs/contract.md: backoff reset only after a stable session, idle watchdog,
// server:shutdown spread without a backoff charge, kick `_seq` resume, yt re-subscribe on reconnect
use crate::i18n::t;
use crate::model::ident;
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_tungstenite::tungstenite::Message as Ws;

mod more;

use super::http::ws_connect;
use super::{NetEvent, Out, backoff, retrying, session};
use super::{pin, pushed, relay, senders};
use crate::cosmetics::id::PaintId;
use crate::heatsync::valid_name;
use crate::model::livegate::Ask;
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
  // your channel tabs, for the account (the list model::tabsync built)
  TabSync(Vec<Value>),
  // ask the account for its tab list again (answered with `multichat:config`, nothing when none is saved)
  TabPull,
  // the feed thread on screen (its root post id), whose live reactions and votes come here; None = none.
  // one room at a time: naming another leaves the last (the ext's feed_thread)
  Thread(Option<Box<str>>),
  // a feed room (`new`, or `ulog:<platform>:<login>`) joined (true) or left (false): the posts and archive lines
  // that arrive live while a view that wants them is on screen. kept across reconnects
  Feed(Box<str>, bool),
  // the emotes a line you just sent names that you picked on purpose (Tab, the picker), so the room draws
  // them from your word while they join your inventory (`emotes:used`). rule 7: a send never waits on that
  Used(Platform, Box<str>, Vec<Vouch>),
  // a line you just sent, told once (not per room) as heatsync.org does: `[H]` on its echo for your other
  // devices, and your picks stored on the line (`chat:origin_tag`). (room, text, send id, picks)
  Origin(Box<str>, Box<str>, Box<str>, Vec<Vouch>),
}

// one picked emote: name, image, provider label
pub struct Vouch {
  pub name: Box<str>,
  pub url: Box<str>,
  pub provider: &'static str,
}

// the server's own bounds on `emotes:used` (ws-handlers.ts handleEmoteUsed)
const MAX_VOUCH: usize = 50;
const MAX_VOUCH_URL: usize = 500;

fn vouch_frame(p: Platform, ch: &str, v: &[Vouch]) -> Option<Value> {
  let name = |n: &str| (1..=64).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
  let emotes: Vec<Value> = v
    .iter()
    .filter(|e| name(&e.name) && e.url.starts_with("https://") && e.url.len() <= MAX_VOUCH_URL)
    .take(MAX_VOUCH)
    .map(|e| json!({"emoteName": &*e.name, "emoteData": {"url": &*e.url, "provider": e.provider}}))
    .collect();
  (!emotes.is_empty() && ident::relay_key(ch))
    .then(|| json!({"type": "emotes:used", "platform": plat(p), "channel": ch, "emotes": emotes}))
}

fn plat(p: Platform) -> &'static str {
  p.name()
}

#[derive(Default)]
struct State {
  rooms: BTreeSet<(&'static str, Box<str>)>,
  yt: BTreeSet<Box<str>>,
  // the thread room joined (lowercase post id), re-joined on every new socket
  thread: Option<Box<str>>,
  // the other feed rooms joined (more.rs `feed_key`), re-joined on every new socket
  feeds: BTreeSet<Box<str>>,
  // last `_seq` seen per kick room, so the server can replay what we missed
  kick_seq: HashMap<Box<str>, u64>,
  yt_status: HashMap<Box<str>, Box<str>>,
  gate: Gate,
  token: Option<String>,
  // a socket has run before, so the next one is a comeback
  resumed: bool,
  // signed in on some socket before / on this one: a later socket's sign-in re-reads the account
  authed: bool,
  socket_authed: bool,
  // youtube feeds this socket re-subscribed after a drop: their replayed backlog fills the gap
  // (lowercase). on a first socket the replay is dropped, history comes from /api/recent
  catching: BTreeSet<Box<str>>,
  // frame types nothing reads, with how many came (more.rs): a missing arm is visible
  unknown: HashMap<Box<str>, u32>,
}

enum End {
  // ui gone: stop for good
  Quit,
  Reconnect,
  // server restarting: come back after a random slice of this many ms
  Shutdown(u64),
  // the server closed us with 4001: the token we signed in with is no good
  BadToken,
}

// the close code heatsync gives a socket whose token it turned down (docs/contract.md)
const CLOSE_BAD_TOKEN: u16 = 4001;

// what a close frame means for the socket: 4001 = the token is bad, anything else just reconnects
fn closed(code: Option<u16>) -> End {
  if code == Some(CLOSE_BAD_TOKEN) { End::BadToken } else { End::Reconnect }
}

pub async fn run(
  mut rx: mpsc::UnboundedReceiver<Cmd>,
  pins: mpsc::UnboundedSender<pin::Ask>,
  sess: mpsc::UnboundedSender<session::Cmd>,
  out: Out,
) {
  let mut st = State::default();
  st.gate.start(crate::util::time::now_ms());
  let mut attempts = 0u32;
  // the user was told heatsync can't be reached, so they're told when it's back
  let mut told = false;
  loop {
    let mut wait = None;
    match timeout(HANDSHAKE_LIMIT, ws_connect(URL, MAX_FRAME)).await {
      Ok(Ok((ws, _))) => {
        let started = Instant::now();
        if std::mem::take(&mut told) {
          out.status(t("net1.hs.back"));
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
          // the token is no good: read anonymously until the login check has ruled on it (it signs
          // us in again only if the token stands), and wait out a backoff so a stubborn one can't loop hot
          End::BadToken => {
            st.token = None;
            let _ = sess.send(session::Cmd::HsRejected);
            attempts += 1;
            wait = Some(backoff(attempts));
          }
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
        retrying(&out, t("net1.hs.unreachable"), attempts - 1, wait);
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
    Cmd::TabSync(channels) => vec![json!({"type": "multichat:sync", "channels": channels})],
    Cmd::TabPull => vec![json!({"type": "multichat:pull"})],
    // only a room we are in (the server refuses the rest)
    Cmd::Used(p, ch, v) if st.rooms.contains(&(plat(p), ch.clone())) => vouch_frame(p, &ch, &v).into_iter().collect(),
    // the server only reads it from a signed-in socket
    Cmd::Origin(ch, text, sid, v) if st.token.is_some() => {
      super::origin::frame(&ch, &text, &sid, &v).into_iter().collect()
    }
    Cmd::Thread(id) => {
      let id: Option<Box<str>> = id
        .filter(|i| (1..=8).contains(&i.len()) && i.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map(|i| i.to_ascii_lowercase().into());
      if id == st.thread {
        return Vec::new();
      }
      let mut out: Vec<Value> = st.thread.iter().map(|i| feed_room("feed:leave", i)).collect();
      out.extend(id.iter().map(|i| feed_room("feed:join", i)));
      st.thread = id;
      out
    }
    Cmd::Feed(key, on) => more::feed_cmd(st, key, on),
    // `youtube:subscribe` joins no room: chat:origin comes to `youtube/<handle>`, joined like any other room
    Cmd::YtSub(h) if ident::relay_key(&h) => {
      let mut out: Vec<Value> = st.yt.insert(h.clone()).then(|| yt_sub(&h)).into_iter().collect();
      let key = crate::model::hsorigin::room(&h);
      if st.rooms.len() < MAX_ROOMS && st.rooms.insert(("youtube", key.clone())) {
        out.extend(join("youtube", &key));
      }
      out
    }
    Cmd::YtUnsub(h) => {
      st.yt.remove(&h);
      st.yt_status.remove(&h);
      // without a videoId the server ref-counts and reaps on drop
      let key = crate::model::hsorigin::room(&h);
      if st.rooms.remove(&("youtube", key.clone())) {
        return vec![json!({"type": "channel:leave", "platform": "youtube", "channel": &*key})];
      }
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

// the server's room for a thread is `feed:thread:<id>`; a socket joins it with feed `thread:<id>`
fn feed_room(ty: &str, id: &str) -> Value {
  json!({"type": ty, "feed": format!("thread:{id}")})
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
  hello.extend(st.thread.iter().map(|i| feed_room("feed:join", i)));
  hello.extend(st.feeds.iter().map(|k| json!({"type": "feed:join", "feed": &**k})));
  st.catching = resubscribed(st);
  st.socket_authed = false;
  for f in hello {
    if tx.send(Ws::text(f.to_string())).await.is_err() {
      return End::Reconnect;
    }
  }
  // a comeback: pushes missed in the drop are re-read (the app seeds every open room again)
  if st.resumed {
    out.send(NetEvent::HsUp);
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
          Some(Ok(Ws::Close(f))) => return closed(f.map(|f| u16::from(f.code))),
          None | Some(Err(_)) => return End::Reconnect,
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
          out.status(t("net1.hs.quiet"));
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
    "kick-sub-event" | "kick-kicks-event" => {
      let key: Box<str> =
        v.get("channel").and_then(Value::as_str).filter(|c| ident::relay_key(c))?.to_lowercase().into();
      if !st.rooms.contains(&("kick", key.clone())) {
        return None;
      }
      let m = relay::kick_event(v, ty == "kick-kicks-event")?;
      send(NetEvent::Relay { platform: Platform::Kick, key, msgs: vec![m], history: false })
    }
    "kick-moderation" => {
      let key: Box<str> = v.get("channel").and_then(Value::as_str).filter(|c| ident::relay_key(c))?.into();
      if v.get("action").and_then(Value::as_str) == Some("delete") {
        let id: Box<str> = v.get("messageId").and_then(Value::as_str).filter(|i| ident::relay_key(i))?.into();
        return send(NetEvent::RelayDelete { platform: Platform::Kick, key, id });
      }
      let login: Box<str> =
        v.get("username").and_then(Value::as_str).filter(|u| ident::relay_key(u))?.to_lowercase().into();
      let secs = (v.get("action").and_then(Value::as_str) == Some("timeout"))
        .then(|| v.get("durationSec").and_then(Value::as_u64).filter(|&s| s > 0).map(|s| s.min(u32::MAX as u64) as u32))
        .flatten();
      send(NetEvent::RelayClear { platform: Platform::Kick, key, login, secs })
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
    "youtube:delete" => {
      let key: Box<str> = v.get("channelId").and_then(Value::as_str).filter(|c| ident::relay_key(c))?.into();
      super::ytdel::parse(v).and_then(|gone| send(NetEvent::YtGone { key, gone }))
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
    // a bulk change to YOUR inventory from the site (apply a set, import, undo/redo): refetch it, exactly
    // like an add (the ext's scheduleInventoryRefresh). no payload is read; only your own sockets get it
    "emotes:refresh" => send(NetEvent::Inventory { removed: None }),
    // the server's heat push for chatters whose score moved (about every minute): kept for those on screen
    "user:heat_batch_update" => {
      let got = crate::heatsync::heat::parse_push(v);
      (!got.is_empty()).then(|| send(NetEvent::Heat(got))).flatten()
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
      let tabbed = st.rooms.contains(&(plat(platform), channel.clone()));
      if !ident::relay_key(&channel) || !(followed || tabbed) {
        return None;
      }
      let text = |k: &str| stream_events::clean(v.get(k).and_then(Value::as_str).unwrap_or(""));
      let kind = match ty {
        "stream:online" => Kind::Online,
        "stream:offline" => Kind::Offline,
        _ => Kind::Update,
      };
      // the followed list hears every sign-on, switch and sign-off of a channel you follow, ahead of the
      // chat lines' own gates (which may hold or drop the line)
      if followed
        && let Some(c) = crate::model::following::Change::from_frame(kind, v)
        && let Some(end) = send(NetEvent::FollowChange(c))
      {
        return Some(end);
      }
      // a followed channel you have a tab for speaks through its room's own frames (ext: the follow path
      // skips config.channels), so its go-live is one line, not two
      if followed && tabbed {
        return None;
      }
      let color = v.get("color").and_then(Value::as_str).and_then(crate::model::color::parse_hex);
      let user_id = stream_events::user_id(platform, v.get("userId"));
      let ev = Ev {
        kind,
        platform,
        channel,
        game: text("game"),
        prev_game: text("prevGame"),
        color,
        user_id,
        fresh: followed,
      };
      st.gate.event(crate::util::time::now_ms(), ev).and_then(|l| send(NetEvent::Stream(l)))
    }
    // the server's last 24h of followed channels' stream news, once after sign-in
    "follow:history" => {
      let rows = stream_events::history(crate::util::time::now_ms(), v);
      (!rows.is_empty()).then(|| send(NetEvent::StreamHistory(rows))).flatten()
    }
    // the room's own news: a raid out, a gift bomb, a new sub (in the room's tab only)
    "stream:raid" | "stream:sub-gift" | "stream:sub" => {
      let platform = room_platform(v.get("platform").and_then(Value::as_str)?)?;
      let channel: Box<str> = v.get("channel").and_then(Value::as_str)?.to_ascii_lowercase().into();
      if !ident::relay_key(&channel) || !st.rooms.contains(&(plat(platform), channel.clone())) {
        return None;
      }
      let raid = ty == "stream:raid";
      let what = if ty == "stream:sub" { stream_events::sub_text(v)? } else { stream_events::news_text(v, raid)? };
      let tone = if raid { stream_events::Tone::Raid } else { stream_events::Tone::Gift };
      st.gate.news(crate::util::time::now_ms(), platform, channel, tone, &what).and_then(|l| send(NetEvent::Stream(l)))
    }
    "stream:redeem" => {
      let platform = room_platform(v.get("platform").and_then(Value::as_str)?)?;
      let channel: Box<str> = v.get("channel").and_then(Value::as_str)?.to_ascii_lowercase().into();
      if !ident::relay_key(&channel) || !st.rooms.contains(&(plat(platform), channel.clone())) {
        return None;
      }
      st.gate.redeem(crate::util::time::now_ms(), platform, channel, v).and_then(|l| send(NetEvent::Stream(l)))
    }
    // pushed only to your own sockets: a line you sent from heatsync.org
    // anyone's line that went through heatsync, in a room we are in
    "chat:origin" => super::origins::frame(v).and_then(send),
    "chat:origin_broadcast" => crate::model::origin::text(v).and_then(|t| send(NetEvent::Origin(t))),
    // once after sign-in: who you follow is live, from the server's cache (nothing when it is cold)
    "follow:live:snapshot" => {
      let rows = crate::model::following::parse(v.get("streams"));
      (!rows.is_empty()).then(|| send(NetEvent::FollowSnapshot(rows))).flatten()
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
    // the thread open: someone reacted, or took it back (the room is joined only for that thread)
    "reaction:added" | "reaction:removed" if st.thread.is_some() => {
      let f = crate::model::reaction::parse_frame(v, ty == "reaction:added")?;
      send(NetEvent::Reaction(f))
    }
    "vote:updated" if st.thread.is_some() => post_heat(v, "message_id", "heat").and_then(send),
    // the site's heat beat: a feed post's number moved (to every client, up to 100 posts a tick). absolute
    "heat:updated" => post_heat(v, "messageId", "newHeat").and_then(send),
    "new-message" => {
      let p = crate::model::post::parse(v.get("data")?)?;
      send(NetEvent::FeedLive(p))
    }
    // a heatsync name colour changed (to every client): their posts and dms wear it
    "profile:color" => pushed::profile_color(v).and_then(|c| send(NetEvent::ProfileColor(c))),
    // someone else's action on a feed post, seen by every client: gone, rewritten, renumbered, moderated
    "message:hidden" => pushed::hidden(v).and_then(|id| send(NetEvent::PostHidden(id))),
    "message-deleted" => pushed::deleted(v).and_then(|id| send(NetEvent::PostHidden(id))),
    "message-edited" => pushed::edited(v).and_then(|e| send(NetEvent::PostEdited(e))),
    "message-updated" => pushed::updated(v).and_then(|m| send(NetEvent::PostMeta(m))),
    "message:approved" | "message:flagged" => {
      pushed::moderated(v, ty == "message:flagged").and_then(|(id, m)| send(NetEvent::PostModerated(id, m)))
    }
    // pushed only to your own sockets, from another of your devices: the payload names the change, the
    // lists are read again (rate-limited in the app)
    "follows:changed" => send(NetEvent::Refetch(Ask::Follows)),
    "emote:blocked" | "emote:unblocked" => send(NetEvent::Refetch(Ask::EmoteBlocks)),
    "user:blocked" | "user:unblocked" => send(NetEvent::Refetch(Ask::Blocks)),
    // your mention rules were edited on another client; the server matched a line to one of them
    "mention-rules:changed" => send(NetEvent::Refetch(Ask::MentionRules)),
    "mention:rule-match" => crate::model::mrules::hit(v).and_then(|h| send(NetEvent::RuleHit(h))),
    "notifications:read" => pushed::unread(v).and_then(|n| send(NetEvent::Unread(n))),
    "bookmarks:changed" => pushed::bookmark(v).and_then(|(id, on)| send(NetEvent::Bookmark(id, on))),
    // your account settings changed: plus, and the five flagged-content switches. the rest of the keys are the
    // site's own
    "user_settings:update" => {
      let prefs = pushed::content_prefs(v);
      if !prefs.is_empty()
        && let Some(end) = send(NetEvent::ContentPrefs(prefs))
      {
        return Some(end);
      }
      pushed::plus_moved(v).then(|| send(NetEvent::PlusMoved)).flatten()
    }
    "mute:added" | "mute:removed" | "mute:expired" | "mute:cleared" => {
      super::mutes::frame(v, ty).and_then(|m| send(NetEvent::MuteSync(m)))
    }
    "seen:update" => {
      use crate::model::unseen::{Surface, at};
      // `inbox` (the site's dm badge time) is none of ours: it parses to nothing
      let surface = Surface::parse(v.get("surface").and_then(Value::as_str)?)?;
      send(NetEvent::Seen { surface, at: at(v.get("at"))? })
    }
    // a heatsync notification for you: a mention, a reply, a fixed report (the list is this session's)
    "notification:new" => {
      let it = crate::model::inbox::parse(v, crate::util::time::now_ms())?;
      send(NetEvent::Notif(it))
    }
    "authenticated" => {
      // who we are, for telling our own reaction frames from other people's
      let who = v.get("userId").and_then(Value::as_str).filter(|u| {
        (1..=64).contains(&u.len()) && u.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
      });
      if let Some(end) = who.and_then(|u| send(NetEvent::HsUser(u.into()))) {
        return Some(end);
      }
      // a link or unlink while we were away sent its account:changed to nobody: read the account again
      let again = st.authed && !st.socket_authed;
      (st.authed, st.socket_authed) = (true, true);
      send(NetEvent::HsAuth(true)).or_else(|| again.then(|| send(NetEvent::AccountChanged)).flatten())
    }
    // a platform was linked or unlinked on heatsync.org (the frame carries nothing else)
    "account:changed" => send(NetEvent::AccountChanged),
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
    // your tab list as the account holds it (after sign-in, on a pull, and on every save by any of your clients)
    "multichat:config" => crate::model::tabsync::frame(v).and_then(|f| send(NetEvent::TabSync(f))),
    "whisper:new" => {
      let w = crate::model::whisper::parse(v, 0, None)?;
      send(NetEvent::Whispers { items: vec![w], live: true, epoch: None })
    }
    // a heatsync dm to you (`dm:sent` is our own send echoing to our other clients: not shown)
    "dm:new" => {
      let w = crate::model::whisper::parse_dm(v, None)?;
      send(NetEvent::Whispers { items: vec![w], live: true, epoch: None })
    }
    // a dm taken down by either of its two people, on any client: its row leaves this list too
    "dm:message-deleted" => {
      crate::model::whisper::dm_ext(v.get("messageId")).and_then(|key| send(NetEvent::DmGone(key)))
    }
    _ => more::more(ty, v, st, out),
  }
}

// a post's heat as a frame says it: the post (a short alphanumeric id, lowercased) and its number, absolute,
// finite, not negative, bounded. a frame that does not say both is nothing
fn post_heat(v: &Value, id: &str, heat: &str) -> Option<NetEvent> {
  let id = v.get(id).and_then(Value::as_str).filter(|i| crate::model::post::id_ok(i))?;
  let heat = v.get(heat).and_then(Value::as_f64).filter(|h| h.is_finite() && *h >= 0.0)?;
  Some(NetEvent::PostHeat { id: id.to_ascii_lowercase().into(), heat: heat.min(1e9).round() as u32 })
}

// one frame through the same reader the socket uses, for the app's headless tests
#[cfg(test)]
pub(crate) fn feed_frame(v: &Value, out: &Out) {
  dispatch(v, &mut State::default(), out);
}

#[cfg(test)]
mod tests;
