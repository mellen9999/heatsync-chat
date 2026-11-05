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
  // your channel tabs, for the account (the list model::tabsync built)
  TabSync(Vec<Value>),
  // ask the account for its tab list again (answered with `multichat:config`, nothing when none is saved)
  TabPull,
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
  // signed in on some socket before / on this one: a later socket's sign-in re-reads the account
  authed: bool,
  socket_authed: bool,
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
    Cmd::TabSync(channels) => vec![json!({"type": "multichat:sync", "channels": channels})],
    Cmd::TabPull => vec![json!({"type": "multichat:pull"})],
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
  st.socket_authed = false;
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
      // the followed list hears every sign-on, switch and sign-off of a channel you follow, ahead of the
      // chat lines' own gates (which may hold or drop the line)
      if followed
        && let Some(c) = crate::model::following::Change::from_frame(kind, v)
        && let Some(end) = send(NetEvent::FollowChange(c))
      {
        return Some(end);
      }
      let color = v.get("color").and_then(Value::as_str).and_then(crate::model::color::parse_hex);
      let ev = Ev { kind, platform, channel, game: text("game"), prev_game: text("prevGame"), name, color };
      st.gate.event(crate::util::time::now_ms(), ev).and_then(|l| send(NetEvent::Stream(l)))
    }
    // the room's own news: a raid out, a gift bomb (in the room's tab only)
    "stream:raid" | "stream:sub-gift" => {
      let platform = room_platform(v.get("platform").and_then(Value::as_str)?)?;
      let channel: Box<str> = v.get("channel").and_then(Value::as_str)?.to_ascii_lowercase().into();
      if !ident::relay_key(&channel) || !st.rooms.contains(&(plat(platform), channel.clone())) {
        return None;
      }
      let raid = ty == "stream:raid";
      let what = stream_events::news_text(v, raid)?;
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
    "new-message" => {
      let p = crate::model::post::parse(v.get("data")?)?;
      send(NetEvent::FeedLive(p))
    }
    "mute:added" | "mute:removed" | "mute:expired" | "mute:cleared" => {
      super::mutes::frame(v, ty).and_then(|m| send(NetEvent::MuteSync(m)))
    }
    "seen:update" => {
      use crate::model::unseen::{Surface, at};
      // `inbox` is the site's dm badge time, not the notifications': another client reading its dms must not
      // clear the notifs tab (that comes back as the account's count, re-read on every sign-in)
      let surface = Surface::parse(v.get("surface").and_then(Value::as_str)?).filter(|s| *s != Surface::Inbox)?;
      send(NetEvent::Seen { surface, at: at(v.get("at"))? })
    }
    // a heatsync notification for you: a mention, a reply, a fixed report (the list is this session's)
    "notification:new" => {
      let it = crate::model::inbox::parse(v, crate::util::time::now_ms())?;
      send(NetEvent::Notif(it))
    }
    "authenticated" => {
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
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_notification_frame_is_checked_and_an_inbox_look_is_heard() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    // the envelope's type is the event name; the kind is the notification's own
    let v = json!({"type": "notification:new", "kind": "reply", "message_id": "ab12", "content": "yo", "parent_is_op": false});
    dispatch(&v, &mut st, &out);
    let Ok(NetEvent::Notif(it)) = rx.try_recv() else { panic!("no notification") };
    assert_eq!((it.post.as_deref(), &*it.text), (Some("ab12"), "replied to your reply: yo"));
    // nothing usable, or hostile: nothing reaches the app
    for bad in [json!({"type": "notification:new"}), json!({"type": "notification:new", "kind": 7, "content": 7})] {
      dispatch(&bad, &mut st, &out);
    }
    assert!(rx.try_recv().is_err());
    dispatch(&json!({"type": "seen:update", "surface": "inbox", "at": 1_700_000_000_000i64}), &mut st, &out);
    assert!(rx.try_recv().is_err(), "the site's dm inbox time is not the notifications'");
  }

  #[test]
  fn a_followed_channel_going_live_is_a_stream_line() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    st.gate.start(0);
    let v = json!({"type": "follow:stream:online", "platform": "twitch", "channel": "Ennortix", "game": "Chess", "title": "x"});
    assert!(dispatch(&v, &mut st, &out).is_none());
    // the followed list hears it first, then the chat gets its line
    let Ok(NetEvent::FollowChange(c)) = rx.try_recv() else { panic!("no list change") };
    assert_eq!((&*c.key, &*c.game, &*c.name), ("ennortix", "Chess", ""));
    let Ok(NetEvent::Stream(l)) = rx.try_recv() else { panic!("no line") };
    assert_eq!((&*l.channel, &*l.text), ("ennortix", "[ennortix] ◆ went live — Chess"));
    // the room reporting the same go-live is the same line: not shown twice
    apply(&mut st, Cmd::Join(Platform::Twitch, "ennortix".into()));
    let room = json!({"type": "stream:online", "platform": "twitch", "channel": "ennortix", "game": "Chess"});
    dispatch(&room, &mut st, &out);
    assert!(rx.try_recv().is_err());
  }

  // the plain `stream:*` of a room is no follow: the list ignores it. the snapshot is bounded and checked
  #[test]
  fn the_followed_list_hears_follow_frames_and_the_snapshot_only() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    st.gate.start(0);
    apply(&mut st, Cmd::Join(Platform::Twitch, "ennortix".into()));
    let room = json!({"type": "stream:online", "platform": "twitch", "channel": "ennortix", "game": "Chess"});
    dispatch(&room, &mut st, &out);
    assert!(!rx.try_iter().any(|e| matches!(e, NetEvent::FollowChange(_))), "a room's own news is not a follow");
    for bad in [json!({"platform": "twitch", "channel": "a b"}), json!({"platform": "youtube", "channel": "x"})] {
      let mut v = bad;
      v["type"] = json!("follow:stream:offline");
      dispatch(&v, &mut st, &out);
    }
    assert!(rx.try_iter().next().is_none(), "junk makes nothing");
    let many: Vec<_> =
      (0..2000).map(|i| json!({"platform": "twitch", "username": format!("u{i}"), "viewerCount": i})).collect();
    dispatch(&json!({"type": "follow:live:snapshot", "streams": many}), &mut st, &out);
    let Ok(NetEvent::FollowSnapshot(rows)) = rx.try_recv() else { panic!("no snapshot") };
    assert_eq!(rows.len(), crate::model::following::MAX_ROWS);
    for junk in
      [json!({"type": "follow:live:snapshot"}), json!({"type": "follow:live:snapshot", "streams": [1, "x", null]})]
    {
      dispatch(&junk, &mut st, &out);
    }
    assert!(rx.try_recv().is_err(), "no rows, no event");
  }

  // kick-moderation: a timeout carries its length, a ban none; a junk name is no removal
  #[test]
  fn a_kick_removal_says_how_long() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    let f = |action: &str, user: &str| json!({"type": "kick-moderation", "channel": "xqc", "action": action, "username": user, "durationSec": 300});
    let got = |rx: &std::sync::mpsc::Receiver<NetEvent>| match rx.try_recv() {
      Ok(NetEvent::RelayClear { login, secs, .. }) => Some((login.to_string(), secs)),
      _ => None,
    };
    dispatch(&f("timeout", "Bob"), &mut st, &out);
    assert_eq!(got(&rx), Some(("bob".into(), Some(300))));
    dispatch(&f("ban", "bob"), &mut st, &out);
    assert_eq!(got(&rx), Some(("bob".into(), None)));
    dispatch(&f("ban", "a b\n"), &mut st, &out);
    assert_eq!(got(&rx), None);
  }

  #[test]
  fn the_heat_push_becomes_scores_by_login_and_a_bad_one_is_dropped() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    let f = json!({"type": "user:heat_batch_update", "updates": [
      {"username": "Bob", "userHeat": 120, "op": 1, "re": 2}, {"username": "bad name", "userHeat": 5}, {"userHeat": 9}, 7]});
    dispatch(&f, &mut st, &out);
    let Ok(NetEvent::Heat(got)) = rx.try_recv() else { panic!("no heat") };
    assert_eq!(got, vec![("bob".into(), 120)]);
    dispatch(&json!({"type": "user:heat_batch_update", "updates": "x"}), &mut st, &out);
    dispatch(&json!({"type": "user:heat_batch_update"}), &mut st, &out);
    assert!(rx.try_recv().is_err(), "nothing readable: nothing sent");
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
  fn youtube_delete_and_emotes_refresh_are_bounded_and_the_site_only_frames_are_ignored() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    let del = json!({"type": "youtube:delete", "channelId": "@chan", "videoId": "v",
      "messageIds": ["m1", "bad id", 5], "authorChannelIds": []});
    dispatch(&del, &mut st, &out);
    let Ok(NetEvent::YtGone { key, gone }) = rx.try_recv() else { panic!("no delete") };
    assert_eq!((&*key, gone.ids.len(), gone.authors.len()), ("@chan", 1, 0));
    // no room, a hostile room, or nothing usable to remove: nothing
    for bad in [
      json!({"type": "youtube:delete", "messageIds": ["m1"]}),
      json!({"type": "youtube:delete", "channelId": "a b", "messageIds": ["m1"]}),
      json!({"type": "youtube:delete", "channelId": "@chan", "messageIds": "m1"}),
    ] {
      dispatch(&bad, &mut st, &out);
    }
    assert!(rx.try_recv().is_err());
    // emotes:refresh is a bare nudge; whatever rides along is never read
    dispatch(&json!({"type": "emotes:refresh", "junk": "x".repeat(10_000)}), &mut st, &out);
    assert!(matches!(rx.try_recv(), Ok(NetEvent::Inventory { removed: None })));
    // settings:delete / heat:updated are the site's (setting keys, feed-post heat): nothing here uses them
    dispatch(&json!({"type": "settings:delete", "key": "theme", "ts": 1}), &mut st, &out);
    dispatch(&json!({"type": "heat:updated", "messageId": "ab12", "newHeat": 9, "newMaxHeat": 9}), &mut st, &out);
    assert!(rx.try_recv().is_err());
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

  // account:changed, and a later socket's sign-in (one sent while we were away is lost), re-read the account
  #[test]
  fn the_account_is_read_again_on_account_changed_and_on_a_later_sign_in() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut st = State::default();
    let auth = json!({"type": "authenticated"});
    let got = |rx: &std::sync::mpsc::Receiver<NetEvent>| {
      rx.try_iter().filter(|e| matches!(e, NetEvent::AccountChanged)).count()
    };
    dispatch(&auth, &mut st, &out);
    assert_eq!(got(&rx), 0, "the first sign-in: login just read it");
    dispatch(&json!({"type": "account:changed"}), &mut st, &out);
    assert_eq!(got(&rx), 1);
    dispatch(&auth, &mut st, &out);
    assert_eq!(got(&rx), 0, "the same socket signing in again is not a comeback");
    st.socket_authed = false;
    dispatch(&auth, &mut st, &out);
    assert_eq!(got(&rx), 1, "a new socket's sign-in");
  }
}
