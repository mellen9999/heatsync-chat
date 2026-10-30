// twitch irc: line parser (pure) + one anonymous websocket session per process
use std::collections::BTreeSet;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as Ws;

use super::{Out, backoff};
use crate::model::color;
use crate::model::message::{Kind, Message, Platform, flag};
use crate::util::time::now_ms;

const URL: &str = "wss://irc-ws.chat.twitch.tv:443";
const PING_EVERY: Duration = Duration::from_secs(60);
// twitch pings ~every 5min; we ping every 60s, so 3min of silence = dead socket
const SILENCE_LIMIT: Duration = Duration::from_secs(180);
const STABLE: Duration = Duration::from_secs(30);
const MAX_BODY: usize = 2000;

#[derive(Debug, PartialEq)]
pub struct Line<'a> {
  pub tags: Vec<(&'a str, &'a str)>,
  pub nick: Option<&'a str>,
  pub command: &'a str,
  pub params: Vec<&'a str>,
}

impl Line<'_> {
  pub fn tag(&self, k: &str) -> Option<String> {
    self
      .tags
      .iter()
      .find(|(t, _)| *t == k)
      .map(|(_, v)| unescape(v))
      .filter(|v| !v.is_empty())
  }

  fn channel(&self) -> Option<&str> {
    self.params.first()?.strip_prefix('#')
  }

  fn trailing(&self) -> Option<&str> {
    self.params.get(1).copied()
  }
}

pub fn parse(raw: &str) -> Option<Line<'_>> {
  let mut rest = raw.trim_end_matches(['\r', '\n']);
  let mut tags = Vec::new();
  if let Some(r) = rest.strip_prefix('@') {
    let (t, r) = r.split_once(' ')?;
    tags = t.split(';').map(|kv| kv.split_once('=').unwrap_or((kv, ""))).collect();
    rest = r;
  }
  let mut nick = None;
  if let Some(r) = rest.strip_prefix(':') {
    let (prefix, r) = r.split_once(' ')?;
    nick = prefix.split('!').next();
    rest = r;
  }
  let (command, mut rest) = rest.split_once(' ').unwrap_or((rest, ""));
  if command.is_empty() {
    return None;
  }
  let mut params = Vec::new();
  while !rest.is_empty() {
    if let Some(t) = rest.strip_prefix(':') {
      params.push(t);
      break;
    }
    let (p, r) = rest.split_once(' ').unwrap_or((rest, ""));
    params.push(p);
    rest = r;
  }
  Some(Line {
    tags,
    nick,
    command,
    params,
  })
}

// ircv3 tag escapes: \s space, \: ;, \\ \, \r, \n
pub fn unescape(v: &str) -> String {
  if !v.contains('\\') {
    return v.to_owned();
  }
  let mut out = String::with_capacity(v.len());
  let mut it = v.chars();
  while let Some(c) = it.next() {
    if c != '\\' {
      out.push(c);
      continue;
    }
    match it.next() {
      Some('s') => out.push(' '),
      Some(':') => out.push(';'),
      Some('r') => out.push('\r'),
      Some('n') => out.push('\n'),
      Some(o) => out.push(o),
      None => {}
    }
  }
  out
}

// `emotes=id:0-4,6-10/id2:12-15` — positions are unicode code points, inclusive
fn twitch_emotes(tag: &str, body: &str) -> Vec<(std::ops::Range<u32>, Box<str>)> {
  let starts: Vec<usize> = body.char_indices().map(|(i, _)| i).chain([body.len()]).collect();
  let mut out = Vec::new();
  for part in tag.split('/') {
    let Some((id, poss)) = part.split_once(':') else {
      continue;
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
      continue;
    }
    for pos in poss.split(',') {
      let Some((a, b)) = pos.split_once('-') else { continue };
      let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) else {
        continue;
      };
      if a > b || b + 1 >= starts.len() {
        continue;
      }
      out.push((starts[a] as u32..starts[b + 1] as u32, id.into()));
    }
  }
  out.sort_by_key(|(r, _)| r.start);
  out
}

fn clamp(s: &str, max: usize) -> &str {
  if s.len() <= max {
    return s;
  }
  let mut end = max;
  while !s.is_char_boundary(end) {
    end -= 1;
  }
  &s[..end]
}

#[derive(Debug)]
pub enum Chat {
  Msg(Message),
  ClearMsg(Box<str>),
  ClearUser {
    login: Box<str>,
    secs: Option<u32>,
  },
  ClearAll,
  // room id keys the channel's 7tv/bttv/ffz sets (m2b)
  #[allow(dead_code)]
  Room {
    id: Box<str>,
  },
}

fn ts_of(l: &Line) -> i64 {
  l.tag("rm-received-ts")
    .or_else(|| l.tag("tmi-sent-ts"))
    .and_then(|v| v.parse().ok())
    .unwrap_or_else(now_ms)
}

fn user_message(l: &Line, body: &str, kind: Kind) -> Message {
  let login: Box<str> = l
    .tag("login")
    .as_deref()
    .or(l.nick)
    .unwrap_or("")
    .to_ascii_lowercase()
    .into();
  let nick: Box<str> = l.tag("display-name").map_or_else(|| login.clone(), Into::into);
  let color = color::readable(
    l.tag("color")
      .and_then(|c| color::parse_hex(&c))
      .unwrap_or_else(|| color::default_for(&login)),
  );
  let body = clamp(body, MAX_BODY);
  let mut flags = 0;
  if l.tag("first-msg").as_deref() == Some("1") {
    flags |= flag::FIRST;
  }
  if l.tag("msg-id").as_deref() == Some("highlighted-message") {
    flags |= flag::HIGHLIGHTED;
  }
  if l.tag("historical").is_some() || l.tag("rm-received-ts").is_some() {
    flags |= flag::HISTORY;
  }
  Message {
    id: l.tag("id").unwrap_or_default().into(),
    ts: ts_of(l),
    platform: Platform::Twitch,
    kind,
    flags,
    login,
    nick,
    color,
    emotes: l
      .tag("emotes")
      .map(|t| twitch_emotes(&t, body))
      .unwrap_or_default()
      .into(),
    body: body.into(),
    reply_to: l.tag("reply-parent-display-name").map(Into::into),
    system: None,
  }
}

pub fn to_chat(l: &Line) -> Option<(Box<str>, Chat)> {
  let ch: Box<str> = l.channel()?.into();
  let ev = match l.command {
    "PRIVMSG" => {
      let text = l.trailing()?;
      let (body, kind) = match text.strip_prefix("\x01ACTION ").and_then(|t| t.strip_suffix('\x01')) {
        Some(b) => (b, Kind::Action),
        None => (text, Kind::Chat),
      };
      Chat::Msg(user_message(l, body, kind))
    }
    "USERNOTICE" => {
      let mut m = user_message(l, l.trailing().unwrap_or(""), Kind::Chat);
      m.system = l.tag("system-msg").map(Into::into);
      if m.body.is_empty() {
        m.kind = Kind::System;
        m.body = m.system.take().unwrap_or_default();
      }
      Chat::Msg(m)
    }
    "NOTICE" => Chat::Msg(Message::system(
      ts_of(l),
      Platform::Twitch,
      clamp(l.trailing()?, MAX_BODY),
    )),
    "CLEARMSG" => Chat::ClearMsg(l.tag("target-msg-id")?.into()),
    "CLEARCHAT" => match l.trailing() {
      Some(u) => Chat::ClearUser {
        login: u.to_ascii_lowercase().into(),
        secs: l.tag("ban-duration").and_then(|d| d.parse().ok()),
      },
      None => Chat::ClearAll,
    },
    "ROOMSTATE" => Chat::Room {
      id: l.tag("room-id")?.into(),
    },
    _ => return None,
  };
  Some((ch, ev))
}

pub enum Cmd {
  Join(Box<str>),
  Part(Box<str>),
}

fn valid_channel(c: &str) -> bool {
  (1..=25).contains(&c.len())
    && c
      .bytes()
      .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

// owns the joined set; reconnects forever with backoff until the ui drops its sender
pub async fn run(mut rx: mpsc::UnboundedReceiver<Cmd>, out: Out) {
  let mut chans = BTreeSet::new();
  let mut attempts = 0u32;
  loop {
    match connect_async(URL).await {
      Ok((ws, _)) => {
        let started = Instant::now();
        out.status("twitch connected");
        if session(ws, &mut rx, &mut chans, &out).await.is_err() {
          return;
        }
        if started.elapsed() >= STABLE {
          attempts = 0;
        }
      }
      Err(e) => out.status(format!("twitch connect failed: {e}")),
    }
    attempts += 1;
    let wait = sleep(backoff(attempts));
    tokio::pin!(wait);
    loop {
      tokio::select! {
        _ = &mut wait => break,
        cmd = rx.recv() => match cmd {
          Some(c) => { apply(&mut chans, c); }
          None => return,
        },
      }
    }
  }
}

fn apply(chans: &mut BTreeSet<Box<str>>, c: Cmd) -> Option<String> {
  match c {
    Cmd::Join(ch) if valid_channel(&ch) => chans.insert(ch.clone()).then(|| format!("JOIN #{ch}")),
    Cmd::Part(ch) => chans.remove(&ch).then(|| format!("PART #{ch}")),
    Cmd::Join(_) => None,
  }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// Err = the ui is gone, stop for good. Ok = reconnect
async fn session(
  ws: Socket,
  rx: &mut mpsc::UnboundedReceiver<Cmd>,
  chans: &mut BTreeSet<Box<str>>,
  out: &Out,
) -> Result<(), ()> {
  let (mut tx, mut read) = ws.split();
  let nick = format!("justinfan{}", 10000 + now_ms() % 80000);
  let mut hello = vec![
    "CAP REQ :twitch.tv/tags twitch.tv/commands".to_owned(),
    "PASS SCHMOOPIIE".into(),
    format!("NICK {nick}"),
  ];
  // batched joins stay under the 20-per-10s anon limit for normal tab counts
  let names: Vec<_> = chans.iter().map(|c| format!("#{c}")).collect();
  for chunk in names.chunks(20) {
    hello.push(format!("JOIN {}", chunk.join(",")));
  }
  for l in hello {
    if tx.send(Ws::text(l)).await.is_err() {
      return Ok(());
    }
  }
  let mut last_rx = Instant::now();
  let mut tick = interval(PING_EVERY);
  tick.tick().await;
  loop {
    tokio::select! {
      frame = read.next() => {
        let text = match frame {
          Some(Ok(Ws::Text(t))) => t,
          Some(Ok(Ws::Close(_))) | None | Some(Err(_)) => return Ok(()),
          Some(Ok(_)) => continue,
        };
        last_rx = Instant::now();
        for raw in text.split("\r\n").filter(|l| !l.is_empty()) {
          if let Some(p) = raw.strip_prefix("PING") {
            if tx.send(Ws::text(format!("PONG{p}"))).await.is_err() {
              return Ok(());
            }
            continue;
          }
          let Some(line) = parse(raw) else { continue };
          if line.command == "RECONNECT" {
            return Ok(());
          }
          if let Some((ch, ev)) = to_chat(&line) && !out.chat(ch, ev) {
            return Err(());
          }
        }
      }
      cmd = rx.recv() => {
        let Some(c) = cmd else { return Err(()) };
        if let Some(l) = apply(chans, c) && tx.send(Ws::text(l)).await.is_err() {
          return Ok(());
        }
      }
      _ = tick.tick() => {
        if last_rx.elapsed() > SILENCE_LIMIT {
          out.status("twitch silent, reconnecting");
          return Ok(());
        }
        if tx.send(Ws::text("PING :tmi.twitch.tv")).await.is_err() {
          return Ok(());
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const PRIV: &str = "@badge-info=;badges=moderator/1;color=#0000FF;display-name=Xqc;emotes=25:6-10/1902:12-16;first-msg=1;id=abc-123;login=xqc;tmi-sent-ts=1700000000000 :xqc!xqc@xqc.tmi.twitch.tv PRIVMSG #forsen :hello Kappa Keepo";

  #[test]
  fn parses_privmsg() {
    let l = parse(PRIV).unwrap();
    assert_eq!(l.command, "PRIVMSG");
    assert_eq!(l.nick, Some("xqc"));
    assert_eq!(l.params, vec!["#forsen", "hello Kappa Keepo"]);
    let (ch, Chat::Msg(m)) = to_chat(&l).unwrap() else {
      panic!()
    };
    assert_eq!(&*ch, "forsen");
    assert_eq!(&*m.nick, "Xqc");
    assert_eq!(m.ts, 1700000000000);
    assert!(m.has(flag::FIRST));
    assert_eq!(m.emotes.len(), 2);
    let (r, id) = &m.emotes[0];
    assert_eq!(&m.body[r.start as usize..r.end as usize], "Kappa");
    assert_eq!(&**id, "25");
    // #0000ff is unreadable on black: lifted
    assert_ne!(m.color, 0x0000ff);
  }

  #[test]
  fn emote_positions_are_code_points() {
    let body = "😂 Kappa";
    let e = twitch_emotes("25:2-6", body);
    assert_eq!(&body[e[0].0.start as usize..e[0].0.end as usize], "Kappa");
  }

  #[test]
  fn hostile_emote_tag() {
    assert!(twitch_emotes("25:9-99,5-2,x-y/:/../:0-1", "hi").is_empty());
  }

  #[test]
  fn action_and_unescape() {
    let l = parse("@display-name=A\\sB :a!a@a PRIVMSG #c :\x01ACTION waves\x01").unwrap();
    let (_, Chat::Msg(m)) = to_chat(&l).unwrap() else {
      panic!()
    };
    assert_eq!(m.kind, Kind::Action);
    assert_eq!(&*m.body, "waves");
    assert_eq!(&*m.nick, "A B");
  }

  #[test]
  fn clears() {
    let l = parse("@ban-duration=600 :tmi.twitch.tv CLEARCHAT #c :baduser").unwrap();
    assert!(matches!(
      to_chat(&l),
      Some((_, Chat::ClearUser { secs: Some(600), .. }))
    ));
    let l = parse(":tmi.twitch.tv CLEARCHAT #c").unwrap();
    assert!(matches!(to_chat(&l), Some((_, Chat::ClearAll))));
    let l = parse("@login=x;target-msg-id=abc :tmi.twitch.tv CLEARMSG #c :bad").unwrap();
    assert!(matches!(to_chat(&l), Some((_, Chat::ClearMsg(id))) if &*id == "abc"));
  }

  #[test]
  fn usernotice_without_body_is_system() {
    let l = parse("@system-msg=x\\sraided;login=x;display-name=X :tmi.twitch.tv USERNOTICE #c").unwrap();
    let (_, Chat::Msg(m)) = to_chat(&l).unwrap() else {
      panic!()
    };
    assert_eq!(m.kind, Kind::System);
    assert_eq!(&*m.body, "x raided");
  }

  #[test]
  fn garbage_does_not_panic() {
    for s in ["", "@", ":", "@a=b", ":x", "PRIVMSG", "@;; :x PRIVMSG #", "PRIVMSG #c"] {
      if let Some(l) = parse(s) {
        let _ = to_chat(&l);
      }
    }
  }

  #[test]
  fn channel_validation() {
    assert!(valid_channel("forsen_2"));
    assert!(!valid_channel("Forsen"));
    assert!(!valid_channel("a b"));
    assert!(!valid_channel(""));
  }
}
