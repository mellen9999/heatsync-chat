// 7tv EventAPI (wss://events.7tv.io/v3): ONE socket, opened only while there is something to
// watch. subscribes `emote_set.update` per joined channel's 7tv set (live emote adds, removals,
// renames) and `user.*` for the broadcasters + us (their paint/badge changed: refetch).
// docs/contract.md bounds apply: every frame is hostile, subscriptions capped
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Message as Ws;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use super::{NetEvent, Out, backoff};
use crate::emotes::live::{self, valid_id};
use crate::emotes::{Provider, providers};

const URL: &str = "wss://events.7tv.io/v3";
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(30);
const STABLE: Duration = Duration::from_secs(30);
const WATCHDOG: Duration = Duration::from_secs(30);
// the hello names a heartbeat (45s); three missed and the socket is a zombie
const DEFAULT_BEAT: Duration = Duration::from_secs(45);
const MAX_FRAME: usize = 512 << 10;
// our own ceiling under whatever the hello allows
const MAX_SUBS: usize = 120;

pub enum Cmd {
  // a channel's 7tv emote set (+ the broadcaster's 7tv user id and twitch id, for cosmetics)
  Watch { ch: Box<str>, set: Box<str>, user: Option<(Box<str>, u64)> },
  Unwatch(Box<str>),
  // our own twitch id: we watch our own cosmetics too
  Me(u64),
  // our own 7tv user id, found from the above
  MyUser(Box<str>, u64),
}

// (set id, broadcaster's 7tv user id + twitch id)
type Watched = (Box<str>, Option<(Box<str>, u64)>);

#[derive(Default)]
struct State {
  // channel → (7tv emote set id, broadcaster's 7tv user id + twitch id)
  chans: HashMap<Box<str>, Watched>,
  me: Option<u64>,
  my_user: Option<(Box<str>, u64)>,
}

type Sub = (&'static str, Box<str>);

impl State {
  fn sets(&self) -> HashSet<&str> {
    self.chans.values().map(|(s, _)| &**s).collect()
  }

  fn channels_of(&self, set: &str) -> Vec<Box<str>> {
    self.chans.iter().filter(|(_, (s, _))| &**s == set).map(|(c, _)| c.clone()).collect()
  }

  fn users(&self) -> HashMap<&str, u64> {
    self.chans.values().filter_map(|(_, u)| u.as_ref()).chain(self.my_user.as_ref()).map(|(s, t)| (&**s, *t)).collect()
  }

  // everything we want a subscription for, bounded
  fn wanted(&self) -> Vec<Sub> {
    let sets = self.sets().into_iter().map(|s| ("emote_set.update", s.into()));
    let users = self.users().into_keys().map(|u| ("user.*", u.into()));
    sets.chain(users).take(MAX_SUBS).collect()
  }

  fn subscribed(&self) -> HashSet<Sub> {
    self.wanted().into_iter().collect()
  }
}

fn frame(op: u8, kind: &str, id: &str) -> Ws {
  Ws::text(json!({"op": op, "d": {"type": kind, "condition": {"object_id": id}}}).to_string())
}

enum End {
  Quit,
  // the last channel left: close and sleep until there is one again
  Idle,
  Reconnect,
}

pub async fn run(
  http: reqwest::Client,
  mut rx: mpsc::UnboundedReceiver<Cmd>,
  me_tx: mpsc::UnboundedSender<Cmd>,
  out: Out,
) {
  let mut st = State::default();
  let mut attempts = 0u32;
  let mut had_session = false;
  loop {
    // nothing to watch: no socket, no timers, no wakeups
    while st.chans.is_empty() {
      match rx.recv().await {
        Some(c) => {
          handle(&mut st, c, &http, &me_tx);
        }
        None => return,
      }
    }
    let cfg = WebSocketConfig::default().max_message_size(Some(MAX_FRAME)).max_frame_size(Some(MAX_FRAME));
    match timeout(HANDSHAKE_LIMIT, connect_async_with_config(URL, Some(cfg), false)).await {
      Ok(Ok((ws, _))) => {
        let started = Instant::now();
        if had_session {
          // we were deaf for a while: the sets may have moved on. refetch them whole
          resync(&st, &http, &out);
        }
        had_session = true;
        match session(ws, &mut rx, &mut st, (&http, &me_tx), &out).await {
          End::Quit => return,
          End::Idle => continue,
          End::Reconnect => {}
        }
        if started.elapsed() >= STABLE {
          attempts = 0;
        }
      }
      Ok(Err(e)) => out.status(format!("7tv events connect failed: {e}")),
      Err(_) => out.status("7tv events handshake timed out"),
    }
    attempts += 1;
    let t = sleep(backoff(attempts));
    tokio::pin!(t);
    loop {
      tokio::select! {
        _ = &mut t => break,
        c = rx.recv() => match c {
          Some(c) => { handle(&mut st, c, &http, &me_tx); }
          None => return,
        },
      }
    }
  }
}

// update the desired state; the frames that bring a live socket in line with it
fn apply(st: &mut State, c: Cmd) -> Vec<Ws> {
  let before = st.subscribed();
  match c {
    Cmd::Watch { ch, set, user } => {
      if valid_id(&set) && (st.chans.len() < MAX_SUBS || st.chans.contains_key(&ch)) {
        let user = user.filter(|(u, _)| valid_id(u));
        st.chans.insert(ch, (set, user));
      }
    }
    Cmd::Unwatch(ch) => {
      st.chans.remove(&ch);
    }
    Cmd::Me(id) => st.me = Some(id),
    Cmd::MyUser(stv, tw) if valid_id(&stv) && st.me == Some(tw) => st.my_user = Some((stv, tw)),
    Cmd::MyUser(..) => {}
  }
  let after = st.subscribed();
  let mut frames: Vec<Ws> = before.difference(&after).map(|(k, id)| frame(36, k, id)).collect();
  frames.extend(after.difference(&before).map(|(k, id)| frame(35, k, id)));
  frames
}

// `Me` also kicks off the lookup of our own 7tv user id
fn handle(st: &mut State, c: Cmd, http: &reqwest::Client, me_tx: &mpsc::UnboundedSender<Cmd>) -> Vec<Ws> {
  if let Cmd::Me(id) = &c
    && st.me != Some(*id)
  {
    tokio::spawn(own_user(http.clone(), *id, me_tx.clone()));
  }
  apply(st, c)
}

async fn own_user(http: reqwest::Client, twitch_id: u64, tx: mpsc::UnboundedSender<Cmd>) {
  let Ok(v) = providers::fetch(&http, &format!("https://7tv.io/v3/users/twitch/{twitch_id}")).await else { return };
  if let Some(id) = v.get("user").and_then(|u| u.get("id")).and_then(Value::as_str).filter(|i| valid_id(i)) {
    let _ = tx.send(Cmd::MyUser(id.into(), twitch_id));
  }
}

fn resync(st: &State, http: &reqwest::Client, out: &Out) {
  for (ch, (set, _)) in &st.chans {
    let (http, out, ch, set) = (http.clone(), out.clone(), ch.clone(), set.clone());
    tokio::spawn(async move {
      if let Ok(v) = providers::fetch(&http, &format!("https://7tv.io/v3/emote-sets/{set}")).await
        && v.is_object()
      {
        out.send(NetEvent::Emotes(Some(ch), Provider::SevenTv, providers::seventv(&v)));
      }
    });
  }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn session(
  ws: Socket,
  rx: &mut mpsc::UnboundedReceiver<Cmd>,
  st: &mut State,
  (http, me_tx): (&reqwest::Client, &mpsc::UnboundedSender<Cmd>),
  out: &Out,
) -> End {
  let (mut tx, mut read) = ws.split();
  // the hello says how many subscriptions this socket may hold; fresh socket, fresh subs
  let mut limit = MAX_SUBS;
  let mut beat = DEFAULT_BEAT;
  let mut last_rx = Instant::now();
  let mut sent = false;
  let mut dog = interval(WATCHDOG);
  dog.tick().await;
  loop {
    tokio::select! {
      f = read.next() => {
        let text = match f {
          Some(Ok(Ws::Text(t))) => t,
          Some(Ok(Ws::Close(_))) | None | Some(Err(_)) => return End::Reconnect,
          Some(Ok(_)) => { last_rx = Instant::now(); continue }
        };
        last_rx = Instant::now();
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        match v.get("op").and_then(Value::as_u64) {
          Some(1) if !sent => {
            let d = v.get("d");
            if let Some(n) = d.and_then(|d| d.get("subscription_limit")).and_then(Value::as_u64).filter(|&n| n > 0) {
              limit = limit.min(n as usize);
            }
            if let Some(ms) = d.and_then(|d| d.get("heartbeat_interval")).and_then(Value::as_u64) {
              beat = Duration::from_millis(ms.clamp(5_000, 120_000));
            }
            sent = true;
            for (kind, id) in st.wanted().into_iter().take(limit) {
              if tx.send(frame(35, kind, &id)).await.is_err() {
                return End::Reconnect;
              }
            }
          }
          Some(0) => {
            for ev in dispatch(&v, st) {
              if !out.send(ev) {
                return End::Quit;
              }
            }
          }
          // server asks us to reconnect, or ends the stream
          Some(4) | Some(7) => return End::Reconnect,
          _ => {}
        }
      }
      c = rx.recv() => {
        let Some(c) = c else { return End::Quit };
        for f in handle(st, c, http, me_tx) {
          // before the hello we send nothing: the hello handler subscribes from `st`
          if sent && tx.send(f).await.is_err() {
            return End::Reconnect;
          }
        }
        if st.chans.is_empty() {
          let _ = tx.close().await;
          return End::Idle;
        }
      }
      _ = dog.tick() => {
        if last_rx.elapsed() > beat * 3 {
          out.status("7tv events silent, reconnecting");
          return End::Reconnect;
        }
      }
    }
  }
}

// an event we care about → ui events. unknown types are ignored
fn dispatch(v: &Value, st: &State) -> Vec<NetEvent> {
  let Some(d) = v.get("d") else { return Vec::new() };
  let Some(body) = d.get("body") else { return Vec::new() };
  match d.get("type").and_then(Value::as_str) {
    Some("emote_set.update") => {
      let Some((set, delta)) = live::parse(body) else { return Vec::new() };
      if delta.is_empty() {
        return Vec::new();
      }
      st.channels_of(&set).into_iter().map(|ch| NetEvent::EmoteDelta(ch, delta.clone())).collect()
    }
    Some(t) if t.starts_with("user.") => {
      let id = body.get("id").and_then(Value::as_str).or_else(|| body.get("object")?.get("id")?.as_str());
      id.and_then(|i| st.users().get(i).copied()).map(NetEvent::CosmeticsStale).into_iter().collect()
    }
    _ => Vec::new(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn watch(ch: &str, set: &str, user: Option<(&str, u64)>) -> Cmd {
    Cmd::Watch { ch: ch.into(), set: set.into(), user: user.map(|(u, t)| (u.into(), t)) }
  }

  fn kinds(frames: &[Ws]) -> Vec<(u64, String, String)> {
    frames
      .iter()
      .map(|f| {
        let v: Value = serde_json::from_str(f.to_text().unwrap()).unwrap();
        (
          v["op"].as_u64().unwrap(),
          v["d"]["type"].as_str().unwrap().into(),
          v["d"]["condition"]["object_id"].as_str().unwrap().into(),
        )
      })
      .collect()
  }

  #[test]
  fn subscribes_once_per_set_and_unsubscribes_with_the_last_channel() {
    let mut st = State::default();
    let f = kinds(&apply(&mut st, watch("a", "S1", Some(("U1", 10)))));
    assert!(f.contains(&(35, "emote_set.update".into(), "S1".into())));
    assert!(f.contains(&(35, "user.*".into(), "U1".into())));
    // a kick pair sharing the set subscribes nothing new
    assert!(apply(&mut st, watch("b", "S1", None)).is_empty());
    assert!(apply(&mut st, Cmd::Unwatch("a".into())).iter().all(|f| !f.to_text().unwrap().contains("S1")));
    let f = kinds(&apply(&mut st, Cmd::Unwatch("b".into())));
    assert!(f.contains(&(36, "emote_set.update".into(), "S1".into())));
    assert!(st.chans.is_empty());
  }

  #[test]
  fn set_change_swaps_the_subscription() {
    let mut st = State::default();
    apply(&mut st, watch("a", "S1", None));
    let f = kinds(&apply(&mut st, watch("a", "S2", None)));
    assert_eq!(f.len(), 2);
    assert!(f.contains(&(36, "emote_set.update".into(), "S1".into())));
    assert!(f.contains(&(35, "emote_set.update".into(), "S2".into())));
  }

  #[test]
  fn hostile_ids_and_caps() {
    let mut st = State::default();
    assert!(apply(&mut st, watch("a", "bad id", None)).is_empty());
    assert!(apply(&mut st, watch("a", "S", Some(("../x", 1)))).len() == 1);
    for i in 0..500 {
      apply(&mut st, watch(&format!("c{i}"), &format!("S{i}"), None));
    }
    assert!(st.chans.len() <= MAX_SUBS);
    assert!(st.wanted().len() <= MAX_SUBS);
  }

  #[test]
  fn my_user_only_for_the_twitch_id_we_asked_about() {
    let mut st = State::default();
    apply(&mut st, Cmd::Me(5));
    assert!(apply(&mut st, Cmd::MyUser("U9".into(), 6)).is_empty());
    assert_eq!(kinds(&apply(&mut st, Cmd::MyUser("U9".into(), 5))), vec![(35, "user.*".into(), "U9".into())]);
    assert_eq!(st.users().get("U9"), Some(&5));
  }

  #[test]
  fn events_route_to_their_channels_and_users() {
    use serde_json::json;
    let mut st = State::default();
    apply(&mut st, watch("a", "S1", Some(("U1", 10))));
    apply(&mut st, watch("b", "S1", None));
    let set = json!({"op": 0, "d": {"type": "emote_set.update", "body": {"id": "S1",
      "pushed": [{"key": "emotes", "value": {"id": "E1", "name": "Pog"}}]}}});
    let evs = dispatch(&set, &st);
    assert_eq!(evs.len(), 2);
    assert!(evs.iter().all(|e| matches!(e, NetEvent::EmoteDelta(_, d) if d.add.len() == 1)));
    // an unwatched set, a bare body and junk produce nothing
    let other = json!({"op": 0, "d": {"type": "emote_set.update", "body": {"id": "S9", "pushed": [{"key": "emotes", "value": {"id": "E1", "name": "Pog"}}]}}});
    assert!(dispatch(&other, &st).is_empty());
    assert!(dispatch(&json!({"op": 0, "d": {"type": "emote_set.update"}}), &st).is_empty());
    assert!(dispatch(&json!({"d": 5}), &st).is_empty());
    let user = json!({"op": 0, "d": {"type": "user.update", "body": {"id": "U1"}}});
    assert!(matches!(dispatch(&user, &st)[..], [NetEvent::CosmeticsStale(10)]));
    let stranger = json!({"op": 0, "d": {"type": "user.update", "body": {"id": "U2"}}});
    assert!(dispatch(&stranger, &st).is_empty());
  }

  // network: `cargo test --release -- --ignored live_hello`
  #[tokio::test]
  #[ignore]
  async fn live_hello_and_subscribe_ack() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (ws, _) = connect_async_with_config(URL, None, false).await.unwrap();
    let (mut tx, mut rx) = ws.split();
    let hello: Value = serde_json::from_str(rx.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    println!("{hello}");
    assert_eq!(hello["op"], 1);
    tx.send(frame(35, "emote_set.update", "01FDMJPSF8000CJ4MDR2FNZEQ3")).await.unwrap();
    let ack: Value = loop {
      let v: Value = serde_json::from_str(rx.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
      println!("{v}");
      if v["op"] == 5 || v["op"] == 6 {
        break v;
      }
    };
    assert_eq!(ack["op"], 5);
  }
}
