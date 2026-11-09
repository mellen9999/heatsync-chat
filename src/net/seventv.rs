// 7tv EventAPI (wss://events.7tv.io/v3): ONE socket, opened only while there is something to
// watch. subscribes `emote_set.update` per joined channel's 7tv set (live emote adds, removals,
// renames) and `user.*` for the broadcasters + us (their paint/badge changed: refetch). with personal
// emotes on, also `entitlement.create|delete` per open twitch room (which chatters own a personal set) and
// `emote_set.update` per set heard (net/personal.rs reads the sets themselves).
// docs/contract.md bounds apply: every frame is hostile, subscriptions capped
use crate::i18n::t;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_tungstenite::tungstenite::Message as Ws;

use super::http::ws_connect;
use super::{NetEvent, Out, backoff, retry, retrying};
use crate::emotes::live::{self, valid_id};
use crate::emotes::personal::Ev;
use crate::emotes::{Provider, entitle, providers};

const URL: &str = "wss://events.7tv.io/v3";
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(30);
const STABLE: Duration = Duration::from_secs(30);
const WATCHDOG: Duration = Duration::from_secs(30);
// the hello names a heartbeat (45s); three missed and the socket is a zombie
const DEFAULT_BEAT: Duration = Duration::from_secs(45);
const MAX_FRAME: usize = 512 << 10;
// our own ceiling under whatever the hello allows
const MAX_SUBS: usize = 120;
// personal emotes: twitch rooms heard for (two subscriptions each), and personal sets followed live
const MAX_ROOMS: usize = 40;
const MAX_HEARD: usize = 48;
// our own personal sets kept from the user lookup
const MAX_MY_SETS: usize = 4;

pub enum Cmd {
  // a channel's 7tv emote set (+ the broadcaster's 7tv user id and twitch id, for cosmetics)
  Watch { ch: Box<str>, set: Box<str>, user: Option<(Box<str>, u64)> },
  Unwatch(Box<str>),
  // our own twitch id (None = logged out: stop), so a paint or badge you change on 7tv shows on your
  // own lines now, not when the 30 minute cosmetics cache runs out (the ext does the same, its
  // ensureSelfCosmeticSub). the cost, in docs/PRIVACY.md: 7tv.io is asked who that twitch id is, once per login
  Me(Option<u64>),
  // our own 7tv user id, found from the above, and the personal sets that same reply says we own
  MyUser(Box<str>, u64, Vec<Box<str>>),
  // 7tv personal emotes: None = off. else the twitch room ids of the open channels, to hear which chatters
  // there own a set. no id of ours goes with it
  Personal(Option<Vec<u64>>),
}

// (set id, broadcaster's 7tv user id + twitch id)
type Watched = (Box<str>, Option<(Box<str>, u64)>);

#[derive(Default)]
struct State {
  // channel → (7tv emote set id, broadcaster's 7tv user id + twitch id)
  chans: HashMap<Box<str>, Watched>,
  me: Option<u64>,
  my_user: Option<(Box<str>, u64)>,
  // personal emotes: the twitch rooms to hear entitlements for (None = off)
  personal: Option<Vec<u64>>,
  // our own personal sets, from the lookup that found our 7tv user
  my_sets: Vec<Box<str>>,
  // sets entitlements named, newest first: each gets its own live-edit subscription
  heard: VecDeque<Box<str>>,
}

// what a subscription is about: a 7tv object, or (entitlements) every chatter present in a twitch room
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Cond {
  Object(Box<str>),
  Room(u64),
}

type Sub = (&'static str, Cond);

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

  fn rooms(&self) -> &[u64] {
    self.personal.as_deref().unwrap_or(&[])
  }

  // nothing to listen to: the socket sleeps
  fn idle(&self) -> bool {
    self.chans.is_empty() && self.rooms().is_empty()
  }

  // everything we want a subscription for, bounded, the ones that matter most first
  fn wanted(&self) -> Vec<Sub> {
    let sets = self.sets().into_iter().map(|s| ("emote_set.update", Cond::Object(s.into())));
    let users = self.users().into_keys().map(|u| ("user.*", Cond::Object(u.into())));
    let mut out: Vec<Sub> = sets.chain(users).take(MAX_SUBS).collect();
    let rooms = self.rooms().iter().take(MAX_ROOMS);
    out.extend(rooms.flat_map(|&r| [("entitlement.create", Cond::Room(r)), ("entitlement.delete", Cond::Room(r))]));
    if self.personal.is_some() {
      let have: HashSet<Sub> = out.iter().cloned().collect();
      let heard = self.heard.iter().take(MAX_HEARD).map(|s| ("emote_set.update", Cond::Object(s.clone())));
      out.extend(heard.filter(|s| !have.contains(s)));
    }
    out
  }

  // a set an entitlement named: followed live from now, the oldest of MAX_HEARD let go
  fn hear(&mut self, set: &str) {
    if self.personal.is_none() || !valid_id(set) {
      return;
    }
    self.heard.retain(|s| &**s != set);
    self.heard.push_front(set.into());
    self.heard.truncate(MAX_HEARD);
  }

  fn subscribed(&self) -> HashSet<Sub> {
    self.wanted().into_iter().collect()
  }
}

fn frame(op: u8, kind: &str, on: &Cond) -> Ws {
  let condition = match on {
    Cond::Object(id) => json!({"object_id": id}),
    // the same condition chatterino's cosmetics use: everyone present in this twitch room
    Cond::Room(room) => json!({"ctx": "channel", "platform": "TWITCH", "id": room.to_string()}),
  };
  Ws::text(json!({"op": op, "d": {"type": kind, "condition": condition}}).to_string())
}

// the frames that bring a socket from `before` to what `st` wants now
fn diff(before: &HashSet<Sub>, st: &State) -> Vec<Ws> {
  let after = st.subscribed();
  let mut frames: Vec<Ws> = before.difference(&after).map(|(k, c)| frame(36, k, c)).collect();
  frames.extend(after.difference(before).map(|(k, c)| frame(35, k, c)));
  frames
}

pub(super) const API: &str = "https://7tv.io";

// the events socket can't be reached: a few plain lines for an outage, not one per try
fn down(out: &Out, attempt: u32, wait: Duration) {
  retrying(out, t("net1.stv.unreachable"), attempt, wait);
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
    while st.idle() {
      match rx.recv().await {
        Some(c) => {
          handle(&mut st, c, (&http, &me_tx), &out);
        }
        None => return,
      }
    }
    let mut failed = false;
    match timeout(HANDSHAKE_LIMIT, ws_connect(URL, MAX_FRAME)).await {
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
      Ok(Err(e)) => {
        eprintln!("7tv events connect: {e}");
        failed = true;
      }
      Err(_) => {
        eprintln!("7tv events connect: handshake timed out");
        failed = true;
      }
    }
    attempts += 1;
    let wait = backoff(attempts);
    if failed {
      down(&out, attempts, wait);
    }
    let t = sleep(wait);
    tokio::pin!(t);
    loop {
      tokio::select! {
        _ = &mut t => break,
        c = rx.recv() => match c {
          Some(c) => { handle(&mut st, c, (&http, &me_tx), &out); }
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
    Cmd::Me(id) => {
      if st.me != id {
        st.my_user = None;
        st.my_sets.clear();
      }
      st.me = id;
    }
    Cmd::MyUser(stv, tw, sets) if valid_id(&stv) && st.me == Some(tw) => {
      st.my_user = Some((stv, tw));
      st.my_sets = sets.into_iter().filter(|s| valid_id(s)).take(MAX_MY_SETS).collect();
    }
    Cmd::MyUser(..) => {}
    Cmd::Personal(rooms) => {
      st.personal = rooms.map(|mut r| {
        r.sort_unstable();
        r.dedup();
        r.truncate(MAX_ROOMS);
        r
      });
      if st.personal.is_none() {
        st.heard.clear();
      }
    }
  }
  diff(&before, st)
}

// `Me` also kicks off the lookup of our own 7tv user id. our own personal sets (that lookup names them) are
// told to the app whenever personal emotes are on and we know them
fn handle(
  st: &mut State,
  c: Cmd,
  (http, me_tx): (&reqwest::Client, &mpsc::UnboundedSender<Cmd>),
  out: &Out,
) -> Vec<Ws> {
  if let Cmd::Me(Some(id)) = &c
    && st.me != Some(*id)
  {
    tokio::spawn(own_user(http.clone(), API.into(), *id, me_tx.clone(), out.clone()));
  }
  let (found, was_on) = (matches!(c, Cmd::MyUser(..)), st.personal.is_some());
  let mut frames = apply(st, c);
  if st.personal.is_some() && (found || !was_on) {
    frames.extend(own_sets(st, out));
  }
  frames
}

// our own personal sets are owned by our twitch id like anyone's: the same event, and the same live follow
fn own_sets(st: &mut State, out: &Out) -> Vec<Ws> {
  let Some(tw) = st.my_user.as_ref().map(|u| u.1) else { return Vec::new() };
  let before = st.subscribed();
  for set in st.my_sets.clone() {
    st.hear(&set);
    out.send(NetEvent::Personal(Ev::Grant { users: vec![tw], set }));
  }
  diff(&before, st)
}

// the one place your own twitch id leaves for 7tv: `GET 7tv.io/v3/users/twitch/<id>`, for the 7tv user id
// the `user.*` subscription needs. nothing else about you is sent
async fn own_user(http: reqwest::Client, base: String, twitch_id: u64, tx: mpsc::UnboundedSender<Cmd>, out: Out) {
  // a hiccup is retried a few times; 7tv not knowing you (404) is an answer, not a failure
  let url = format!("{base}/v3/users/twitch/{twitch_id}");
  let Some(v) = retry::until_n(&out, "your 7tv user", 4, || !tx.is_closed(), || providers::fetch(&http, &url)).await
  else {
    return;
  };
  if let Some(id) = v.get("user").and_then(|u| u.get("id")).and_then(Value::as_str).filter(|i| valid_id(i)) {
    // the same reply lists the sets this user owns: the personal ones are ours (no second ask)
    let sets = v.get("user").map(entitle::own_sets).unwrap_or_default();
    let _ = tx.send(Cmd::MyUser(id.into(), twitch_id, sets));
  }
}

fn resync(st: &State, http: &reqwest::Client, out: &Out) {
  if st.personal.is_some() {
    // the sets held may have moved on too: each is read again when one of its owners speaks
    out.send(NetEvent::Personal(Ev::Stale));
  }
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
            for (kind, on) in st.wanted().into_iter().take(limit) {
              if tx.send(frame(35, kind, &on)).await.is_err() {
                return End::Reconnect;
              }
            }
          }
          Some(0) => {
            let evs = dispatch(&v, st);
            // a set 7tv just named is followed live from here
            for f in hear(st, &evs) {
              if sent && tx.send(f).await.is_err() {
                return End::Reconnect;
              }
            }
            for ev in evs {
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
        for f in handle(st, c, (http, me_tx), out) {
          // before the hello we send nothing: the hello handler subscribes from `st`
          if sent && tx.send(f).await.is_err() {
            return End::Reconnect;
          }
        }
        if st.idle() {
          let _ = tx.close().await;
          return End::Idle;
        }
      }
      _ = dog.tick() => {
        if last_rx.elapsed() > beat * 3 {
          out.status(t("net1.stv.retry"));
          return End::Reconnect;
        }
      }
    }
  }
}

// the sets a batch of events names as owned by someone: followed live from now. the frames that takes
fn hear(st: &mut State, evs: &[NetEvent]) -> Vec<Ws> {
  let named: Vec<&str> = evs
    .iter()
    .filter_map(|e| if let NetEvent::Personal(Ev::Grant { set, .. }) = e { Some(&**set) } else { None })
    .collect();
  if named.is_empty() {
    return Vec::new();
  }
  let before = st.subscribed();
  named.into_iter().for_each(|s| st.hear(s));
  diff(&before, st)
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
      let chans = st.channels_of(&set);
      if chans.is_empty() {
        // not a channel's: a personal set's edit (the app drops one for a set it does not hold)
        return if st.personal.is_some() { vec![NetEvent::Personal(Ev::Delta(set, delta))] } else { Vec::new() };
      }
      chans.into_iter().map(|ch| NetEvent::EmoteDelta(ch, delta.clone())).collect()
    }
    // who owns a personal set, only while that is on: a grant or a revoke of an EMOTE_SET and nothing else
    Some(t @ ("entitlement.create" | "entitlement.delete")) if st.personal.is_some() => {
      entitle::event(t, body).map(NetEvent::Personal).into_iter().collect()
    }
    Some(t) if t.starts_with("user.") => {
      let id = body.get("id").and_then(Value::as_str).or_else(|| body.get("object")?.get("id")?.as_str());
      id.and_then(|i| st.users().get(i).copied())
        .map(|u| NetEvent::CosmeticsStale(crate::cosmetics::id::PaintId::Twitch(u)))
        .into_iter()
        .collect()
    }
    _ => Vec::new(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::cosmetics::id::PaintId;
  use tokio_tungstenite::connect_async_with_config;

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
          // an object's id, or the twitch room an entitlement subscription listens in (`room:<id>`)
          v["d"]["condition"]["object_id"]
            .as_str()
            .map_or_else(|| format!("room:{}", v["d"]["condition"]["id"].as_str().unwrap()), Into::into),
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
    apply(&mut st, Cmd::Me(Some(5)));
    assert!(apply(&mut st, Cmd::MyUser("U9".into(), 6, Vec::new())).is_empty());
    assert_eq!(
      kinds(&apply(&mut st, Cmd::MyUser("U9".into(), 5, Vec::new()))),
      vec![(35, "user.*".into(), "U9".into())]
    );
    assert_eq!(st.users().get("U9"), Some(&5));
  }

  #[test]
  fn logout_or_a_new_account_drops_our_own_user_subscription() {
    let mut st = State::default();
    apply(&mut st, Cmd::Me(Some(5)));
    apply(&mut st, Cmd::MyUser("U9".into(), 5, Vec::new()));
    assert_eq!(kinds(&apply(&mut st, Cmd::Me(None))), vec![(36, "user.*".into(), "U9".into())]);
    apply(&mut st, Cmd::Me(Some(5)));
    apply(&mut st, Cmd::MyUser("U9".into(), 5, Vec::new()));
    // another account: the old subscription goes before the new id is known
    assert_eq!(kinds(&apply(&mut st, Cmd::Me(Some(6)))), vec![(36, "user.*".into(), "U9".into())]);
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
    assert!(matches!(dispatch(&user, &st)[..], [NetEvent::CosmeticsStale(PaintId::Twitch(10))]));
    let stranger = json!({"op": 0, "d": {"type": "user.update", "body": {"id": "U2"}}});
    assert!(dispatch(&stranger, &st).is_empty());
  }

  fn said(rx: &std::sync::mpsc::Receiver<NetEvent>) -> Vec<String> {
    rx.try_iter().filter_map(|e| if let NetEvent::Status(s) = e { Some(s) } else { None }).collect()
  }

  // a dead events socket is a few plain lines, not one per try with the error text in them
  #[test]
  fn an_unreachable_socket_is_throttled_and_plain() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    for attempt in 1..=10 {
      down(&out, attempt, Duration::from_secs(8));
    }
    let said = said(&rx);
    assert_eq!(said.len(), 5, "{said:?}");
    assert_eq!(said[0], "can't reach 7tv, trying again in 8s");
  }

  // one local server: the first asks get a 503, the next the user
  async fn flaky(fails: usize) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
      for n in 0.. {
        let Ok((mut s, _)) = l.accept().await else { return };
        let mut buf = [0u8; 1024];
        let _ = s.read(&mut buf).await;
        let r = if n < fails {
          "HTTP/1.1 503 Service Unavailable\r\nconnection: close\r\ncontent-length: 0\r\n\r\n".to_owned()
        } else {
          let b = r#"{"user":{"id":"01ABCDEF","emote_sets":[{"id":"CHAN","flags":0},{"id":"PERS","flags":4}]}}"#;
          format!("HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{b}", b.len())
        };
        let _ = s.write_all(r.as_bytes()).await;
      }
    });
    base
  }

  // the own-user lookup is tried again after a hiccup instead of once
  #[tokio::test]
  async fn the_own_user_lookup_is_retried() {
    let base = flaky(1).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (otx, _orx) = std::sync::mpsc::channel();
    own_user(reqwest::Client::new(), base, 5, tx, Out::Pipe(otx)).await;
    // the same reply names the personal sets we own (the flagged one, not the channel's)
    assert!(
      matches!(rx.try_recv(), Ok(Cmd::MyUser(id, 5, sets)) if &*id == "01ABCDEF" && *sets == [Box::<str>::from("PERS")])
    );
  }

  fn personal(rooms: &[u64]) -> Cmd {
    Cmd::Personal(Some(rooms.to_vec()))
  }

  fn room(id: u64) -> String {
    format!("room:{id}")
  }

  fn entitled(kind: &str, set: &str, twitch: &str) -> Value {
    json!({"op": 0, "d": {"type": kind, "body": {"id": "0", "kind": 5, "object": {"id": "0", "kind": "EMOTE_SET",
      "ref_id": set, "user": {"id": "U1", "connections": [{"platform": "TWITCH", "id": twitch}]}}}}})
  }

  #[test]
  fn personal_emotes_listen_in_each_open_room_and_only_there() {
    let mut st = State::default();
    assert!(st.idle());
    let f = kinds(&apply(&mut st, personal(&[10, 10, 20])));
    assert_eq!(f.len(), 4, "create + delete per room, a room once");
    for r in [10, 20] {
      assert!(f.contains(&(35, "entitlement.create".into(), room(r))));
      assert!(f.contains(&(35, "entitlement.delete".into(), room(r))));
    }
    assert!(!st.idle());
    // the frame is the chatterino one: everyone present in that twitch room
    let raw = apply(&mut st, personal(&[10, 30]));
    let create = raw
      .iter()
      .map(|f| serde_json::from_str::<Value>(f.to_text().unwrap()).unwrap())
      .find(|v| v["op"] == 35 && v["d"]["type"] == "entitlement.create");
    assert_eq!(create.unwrap()["d"]["condition"], json!({"ctx": "channel", "platform": "TWITCH", "id": "30"}));
    // a room that closed is let go; off lets go of all of it
    let f = kinds(&apply(&mut st, personal(&[30])));
    assert!(f.contains(&(36, "entitlement.create".into(), room(10))) && f.len() == 2, "{f:?}");
    let f = kinds(&apply(&mut st, Cmd::Personal(None)));
    assert_eq!(f.len(), 2);
    assert!(f.iter().all(|x| x.0 == 36));
    assert!(st.wanted().is_empty() && st.idle());
  }

  #[test]
  fn rooms_are_bounded_and_wake_a_socket_with_no_7tv_channel() {
    let mut st = State::default();
    apply(&mut st, Cmd::Personal(Some(Vec::new())));
    assert!(st.idle(), "personal on with no room open is nothing to listen to");
    apply(&mut st, Cmd::Personal(Some((0..500).collect())));
    assert!(!st.idle() && st.chans.is_empty());
    assert_eq!(st.wanted().len(), MAX_ROOMS * 2);
  }

  // PRIVACY: nothing we subscribe to for personal emotes carries our own twitch id, whoever we are
  #[test]
  fn no_id_of_ours_goes_in_a_personal_subscription() {
    let mut st = State::default();
    let mut frames = apply(&mut st, Cmd::Me(Some(777_777)));
    frames.extend(apply(&mut st, personal(&[10, 20])));
    frames.extend(apply(&mut st, watch("a", "S1", Some(("U1", 10)))));
    for f in frames.iter().chain(&hear(&mut st, &[grant(777_777, "MINE")])) {
      assert!(!f.to_text().unwrap().contains("777777"), "{f:?}");
    }
    // not in the standing list either, even once our own 7tv user is known
    apply(&mut st, Cmd::MyUser("U9".into(), 777_777, vec!["MINE".into()]));
    for (_, on) in st.wanted() {
      assert!(!format!("{on:?}").contains("777777"));
    }
  }

  fn grant(user: u64, set: &str) -> NetEvent {
    NetEvent::Personal(Ev::Grant { users: vec![user], set: set.into() })
  }

  #[test]
  fn entitlements_are_heard_only_while_personal_emotes_are_on() {
    let mut st = State::default();
    apply(&mut st, watch("a", "S1", None));
    let create = entitled("entitlement.create", "PERS", "555");
    assert!(dispatch(&create, &st).is_empty(), "off: not even read");
    apply(&mut st, personal(&[10]));
    assert!(matches!(&dispatch(&create, &st)[..], [NetEvent::Personal(Ev::Grant { users, set })]
      if *users == [555] && &**set == "PERS"));
    let gone = entitled("entitlement.delete", "PERS", "555");
    assert!(matches!(&dispatch(&gone, &st)[..], [NetEvent::Personal(Ev::Revoke { .. })]));
    // anything that is not an EMOTE_SET entitlement, or that no twitch account owns, is nothing
    let mut paint = entitled("entitlement.create", "PERS", "555");
    paint["d"]["body"]["object"]["kind"] = json!("PAINT");
    assert!(dispatch(&paint, &st).is_empty());
    assert!(dispatch(&entitled("entitlement.create", "../x", "555"), &st).is_empty());
    assert!(dispatch(&entitled("entitlement.create", "PERS", "not a number"), &st).is_empty());
    assert!(dispatch(&entitled("entitlement.update", "PERS", "555"), &st).is_empty());
    assert!(dispatch(&json!({"op": 0, "d": {"type": "entitlement.create"}}), &st).is_empty());
  }

  #[test]
  fn a_set_edit_that_is_not_a_channels_is_a_personal_one() {
    let mut st = State::default();
    apply(&mut st, watch("a", "S1", None));
    let edit = |set: &str| {
      json!({"op": 0, "d": {"type": "emote_set.update", "body": {"id": set,
        "pushed": [{"key": "emotes", "value": {"id": "E1", "name": "Pog"}}]}}})
    };
    assert!(dispatch(&edit("PERS"), &st).is_empty(), "off");
    apply(&mut st, personal(&[10]));
    assert!(matches!(&dispatch(&edit("PERS"), &st)[..], [NetEvent::Personal(Ev::Delta(set, _))] if &**set == "PERS"));
    // a channel's own set stays a channel's
    assert!(matches!(&dispatch(&edit("S1"), &st)[..], [NetEvent::EmoteDelta(..)]));
    assert!(dispatch(&edit("../x"), &st).is_empty());
  }

  #[test]
  fn a_set_an_entitlement_names_is_followed_live_and_the_oldest_let_go() {
    let mut st = State::default();
    assert!(hear(&mut st, &[grant(5, "PERS")]).is_empty(), "off: nothing is followed");
    apply(&mut st, personal(&[10]));
    let f = kinds(&hear(&mut st, &[grant(5, "PERS"), NetEvent::Status("x".into())]));
    assert_eq!(f, vec![(35, "emote_set.update".into(), "PERS".into())]);
    assert!(hear(&mut st, &[grant(6, "PERS")]).is_empty(), "heard again: nothing new");
    assert!(hear(&mut st, &[NetEvent::Status("x".into())]).is_empty());
    // past the bound the oldest goes
    let sets: Vec<NetEvent> = (0..MAX_HEARD + 5).map(|i| grant(1, &format!("X{i}"))).collect();
    let f = kinds(&hear(&mut st, &sets));
    assert!(f.contains(&(36, "emote_set.update".into(), "PERS".into())), "the oldest is let go");
    assert_eq!(st.heard.len(), MAX_HEARD);
    assert!(st.wanted().iter().filter(|s| s.0 == "emote_set.update").count() <= MAX_HEARD);
    // off lets go of all of them
    let f = kinds(&apply(&mut st, Cmd::Personal(None)));
    assert!(f.iter().all(|x| x.0 == 36) && st.heard.is_empty() && st.wanted().is_empty());
    // a bad id is never followed
    apply(&mut st, personal(&[10]));
    assert!(hear(&mut st, &[grant(1, "../x")]).is_empty());
  }

  fn told(rx: &std::sync::mpsc::Receiver<NetEvent>) -> Vec<(u64, String)> {
    rx.try_iter()
      .filter_map(|e| match e {
        NetEvent::Personal(Ev::Grant { users, set }) => Some((users[0], set.to_string())),
        _ => None,
      })
      .collect()
  }

  #[test]
  fn our_own_personal_sets_are_told_only_while_on_and_only_for_the_id_we_asked_about() {
    let (otx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(otx);
    let (http, (mtx, _mrx)) = (reqwest::Client::new(), mpsc::unbounded_channel());
    let mut st = State::default();
    let go = |st: &mut State, c: Cmd| handle(st, c, (&http, &mtx), &out);
    go(&mut st, Cmd::Me(None));
    go(&mut st, Cmd::MyUser("U9".into(), 5, vec!["PERS".into()]));
    assert!(told(&rx).is_empty(), "no such account: the lookup is for someone else");
    st.me = Some(5);
    go(&mut st, Cmd::MyUser("U9".into(), 5, vec!["PERS".into(), "../bad".into()]));
    assert!(told(&rx).is_empty(), "personal emotes are off");
    // turned on: told now, and followed live
    let f = kinds(&go(&mut st, personal(&[10])));
    assert_eq!(told(&rx), vec![(5, "PERS".to_owned())]);
    assert!(f.contains(&(35, "emote_set.update".into(), "PERS".into())));
    // turned off and on again: told again, since the app dropped what it held
    go(&mut st, Cmd::Personal(None));
    go(&mut st, personal(&[10]));
    assert_eq!(told(&rx), vec![(5, "PERS".to_owned())]);
    // another account: not ours any more
    apply(&mut st, Cmd::Me(Some(6)));
    go(&mut st, Cmd::Personal(None));
    go(&mut st, personal(&[10]));
    assert!(told(&rx).is_empty());
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
    tx.send(frame(35, "emote_set.update", &Cond::Object("01FDMJPSF8000CJ4MDR2FNZEQ3".into()))).await.unwrap();
    let ack: Value = loop {
      let v: Value = serde_json::from_str(rx.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
      println!("{v}");
      if v["op"] == 5 || v["op"] == 6 {
        break v;
      }
    };
    assert_eq!(ack["op"], 5);
  }

  // network, the whole road with the real 7tv: listen in a busy twitch room, take the owners it names, read
  // their sets, and see a word of one resolve for its owner and for no one else.
  // `cargo test --release -- --ignored live_personal --nocapture`
  #[tokio::test]
  #[ignore]
  async fn live_personal_sets_arrive_and_resolve() {
    use crate::emotes::personal::Personal;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = reqwest::Client::builder().user_agent(crate::net::USER_AGENT).build().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let (cmd, cmd_rx) = mpsc::unbounded_channel();
    tokio::spawn(run(http.clone(), cmd_rx, cmd.clone(), Out::Pipe(tx.clone())));
    // a big room: xqc's (the room id, not a person's)
    cmd.send(Cmd::Personal(Some(vec![71092938]))).unwrap();
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    let mut store = Personal::default();
    store.set_on(true);
    let end = Instant::now() + Duration::from_secs(60);
    let mut owners = Vec::new();
    while Instant::now() < end {
      tokio::time::sleep(Duration::from_millis(500)).await;
      for e in rx.try_iter().collect::<Vec<_>>() {
        if let NetEvent::Personal(ev) = e {
          if let Ev::Grant { users, .. } = &ev {
            owners.extend(users.iter().copied());
          }
          store.apply(ev);
        }
      }
      // a line of each owner being drawn is what asks for their set
      for u in &owners {
        store.lookup(*u, "x");
      }
      let sets = store.take_wanted();
      if !sets.is_empty() {
        println!("reading {} sets", sets.len());
        crate::net::personal::read(http.clone(), gate.clone(), sets, Out::Pipe(tx.clone())).await;
      }
      if owners.len() >= 3 && owners.iter().any(|u| !store.entries(*u).is_empty()) {
        break;
      }
    }
    println!("owners heard: {}", owners.len());
    let (who, name, url) = owners
      .iter()
      .find_map(|u| store.entries(*u).first().map(|(n, e)| (*u, n.to_string(), e.url.to_string())))
      .expect("an owner with a set in the room within a minute");
    println!("{who} owns {name} = {url}");
    assert!(url.starts_with("https://cdn.7tv.app/emote/"));
    assert_eq!(store.lookup(who, &name).map(|e| e.url.to_string()), Some(url));
    assert!(store.lookup(1, &name).is_none(), "a stranger gets none of it");
  }
}
