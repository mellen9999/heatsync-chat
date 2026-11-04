// twitch irc: line parser (pure) + one anonymous websocket session per process
use crate::model::ident;
use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Message as Ws;

use super::dupe::Dupes;
use super::http::ws_config;
use super::{Out, Reply, backoff};
use crate::model::color;
use crate::model::message::{Kind, Message, Notice, Platform, REPLY_TEXT, flag};
use crate::util::clamp;
use crate::util::time::now_ms;

const URL: &str = "wss://irc-ws.chat.twitch.tv:443";
const PING_EVERY: Duration = Duration::from_secs(60);
// twitch pings ~every 5min; we ping every 60s, so 3min of silence = dead socket
const SILENCE_LIMIT: Duration = Duration::from_secs(180);
const STABLE: Duration = Duration::from_secs(30);
const MAX_BODY: usize = 2000;
const HANDSHAKE_LIMIT: Duration = Duration::from_secs(30);
// one irc line is ≤ ~8KiB with tags; twitch may batch several lines in a frame
const MAX_FRAME: usize = 256 << 10;

#[derive(Debug, PartialEq)]
pub struct Line<'a> {
  pub tags: Vec<(&'a str, &'a str)>,
  pub nick: Option<&'a str>,
  pub command: &'a str,
  pub params: Vec<&'a str>,
}

impl Line<'_> {
  pub fn tag(&self, k: &str) -> Option<String> {
    self.tags.iter().find(|(t, _)| *t == k).map(|(_, v)| unescape(v)).filter(|v| !v.is_empty())
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
  Some(Line { tags, nick, command, params })
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
      // `starts` always holds body.len(), so b + 1 is in range only when b < len - 1
      if a > b || b >= starts.len() - 1 {
        continue;
      }
      out.push((starts[a] as u32..starts[b + 1] as u32, crate::emotes::twitch_url(id).into()));
    }
  }
  out.sort_by_key(|(r, _)| r.start);
  out.truncate(super::relay::MAX_EMOTES);
  out
}

// `badges=moderator/1,subscriber/12` — bounded, junk dropped
fn badges(tag: &str) -> Vec<(Box<str>, Box<str>)> {
  tag
    .split(',')
    .take(16)
    .filter_map(|b| b.split_once('/'))
    .filter(|(s, v)| !s.is_empty() && s.len() <= 64 && v.len() <= 64)
    .map(|(s, v)| (s.into(), v.into()))
    .collect()
}

#[derive(Debug)]
pub enum Chat {
  Msg(Message),
  // (message id, whose line)
  ClearMsg(Box<str>, Box<str>),
  ClearUser { login: Box<str>, secs: Option<u32> },
  ClearAll,
  // room id keys the channel's 7tv/bttv/ffz sets (m2b); the chat modes it carries
  Room { id: Box<str>, modes: Vec<crate::model::modes::Set> },
  // our own badges here say we moderate (or own) the channel
  Mod(bool),
}

// USERSTATE's badges: a moderator or the broadcaster can use the mod tools
fn can_mod(l: &Line) -> bool {
  l.tag("badges")
    .is_some_and(|b| b.split(',').any(|x| matches!(x.split('/').next(), Some("moderator" | "broadcaster"))))
}

fn ts_of(l: &Line) -> i64 {
  l.tag("rm-received-ts").or_else(|| l.tag("tmi-sent-ts")).and_then(|v| v.parse().ok()).unwrap_or_else(now_ms)
}

fn user_message(l: &Line, body: &str, kind: Kind) -> Message {
  let login: Box<str> = l.tag("login").as_deref().or(l.nick).unwrap_or("").to_ascii_lowercase().into();
  let nick: Box<str> = l.tag("display-name").map_or_else(|| login.clone(), Into::into);
  // as sent: the view lifts it (or not) at draw time, so the setting works on lines already shown
  let color = l.tag("color").and_then(|c| color::parse_hex(&c)).unwrap_or_else(|| color::default_for(&login));
  let body = clamp(super::dupe::strip(body), MAX_BODY);
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
    user_id: l.tag("user-id").filter(|u| u.bytes().all(|b| b.is_ascii_digit())).unwrap_or_default().into(),
    nick,
    color,
    badges: l.tag("badges").map(|b| badges(&b)).unwrap_or_default().into(),
    emotes: l.tag("emotes").map(|t| twitch_emotes(&t, body)).unwrap_or_default().into(),
    hs: Box::new([]),
    body: body.into(),
    reply_to: l.tag("reply-parent-display-name").map(Into::into),
    reply_text: l.tag("reply-parent-msg-body").map(|t| clamp(super::dupe::strip(&t), REPLY_TEXT).into()),
    system: None,
    x: None,
  }
}

// what event a USERNOTICE tells of (ext main.js noticeKind): its line takes the kind's color
fn notice_of(l: &Line) -> Notice {
  let mut id = l.tag("msg-id").unwrap_or_default();
  // shared chat: the real event rides in source-msg-id
  if id == "sharedchatnotice" {
    id = l.tag("source-msg-id").unwrap_or_default();
  }
  match id.as_str() {
    "sub"
    | "resub"
    | "primepaidupgrade"
    | "extendsub"
    | "subgift"
    | "anonsubgift"
    | "submysterygift"
    | "giftpaidupgrade"
    | "anongiftpaidupgrade"
    | "rewardgift"
    | "standardpayforward"
    | "communitypayforward" => Notice::Sub,
    "raid" | "unraid" => Notice::Raid,
    "announcement" => Notice::Announce,
    "bitsbadgetier" | "charitydonation" | "mod-anniversary" | "mod_anniversary" => Notice::Bits,
    "viewermilestone" if l.tag("msg-param-category").as_deref() == Some("watch-streak") => Notice::Streak,
    "viewermilestone" => Notice::Milestone,
    _ => Notice::Plain,
  }
}

// a cheer's `bits=N` (twitch's own count); junk or zero is none
fn bits_of(l: &Line) -> u32 {
  l.tag("bits").and_then(|b| b.parse::<u32>().ok()).filter(|&n| n <= 10_000_000).unwrap_or(0)
}

// USERSTATE after our PRIVMSG: twitch's id/color/badges for the line, our own text
fn own_message(l: &Line, nick: &str, me_id: &str, text: &str) -> Message {
  let (body, kind) = match text.strip_prefix("/me ") {
    Some(b) => (b, Kind::Action),
    None => (text, Kind::Chat),
  };
  let mut m = user_message(l, body, kind);
  m.login = nick.into();
  m.user_id = me_id.into();
  m.ts = now_ms();
  // USERSTATE carries emote-sets, not positions: twitch emotes resolve through the catalogs instead
  m.emotes = Box::new([]);
  m.flags |= flag::OWN;
  m
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
      let mut m = user_message(l, body, kind);
      let bits = bits_of(l);
      if bits > 0 {
        m.extra().bits = bits;
      }
      // a line sent with a channel-point reward (ext .hs-mc-redeem-label): twitch's irc names no reward,
      // so the label is the ext's generic one. still the person's own line
      if l.tag("custom-reward-id").is_some() {
        m.system = Some("◆ channel point redeem".into());
        m.extra().notice = Notice::Redeem;
      }
      Chat::Msg(m)
    }
    "USERNOTICE" => {
      let mut m = user_message(l, l.trailing().unwrap_or(""), Kind::Chat);
      m.system = l.tag("system-msg").map(|t| clamp(&t, MAX_BODY).into());
      let notice = notice_of(l);
      if notice != Notice::Plain {
        m.extra().notice = notice;
      }
      if m.body.is_empty() {
        m.kind = Kind::System;
        m.body = m.system.take().unwrap_or_default();
      }
      Chat::Msg(m)
    }
    "NOTICE" => Chat::Msg(Message::system(ts_of(l), Platform::Twitch, clamp(l.trailing()?, MAX_BODY))),
    "CLEARMSG" => Chat::ClearMsg(l.tag("target-msg-id")?.into(), l.tag("login").unwrap_or_default().into()),
    "CLEARCHAT" => match l.trailing() {
      Some(u) => Chat::ClearUser {
        login: u.to_ascii_lowercase().into(),
        secs: l.tag("ban-duration").and_then(|d| d.parse().ok()),
      },
      None => Chat::ClearAll,
    },
    "ROOMSTATE" => Chat::Room { id: l.tag("room-id")?.into(), modes: crate::model::modes::roomstate(|k| l.tag(k)) },
    "USERSTATE" => Chat::Mod(can_mod(l)),
    _ => return None,
  };
  Some((ch, ev))
}

pub enum Cmd {
  Join(Box<str>),
  Part(Box<str>),
  // None = back to anonymous. either way the connection restarts under the new identity
  Login(Option<Creds>),
  Send { chan: Box<str>, text: String, reply: Option<Reply> },
}

pub struct Creds {
  pub token: String,
  pub nick: String,
}

// a sent line waits for twitch's USERSTATE (which carries its id, color, badges) to be shown
struct Sent {
  chan: Box<str>,
  text: String,
  reply: Option<Reply>,
}

const MAX_PENDING: usize = 20;
const MAX_SEND: usize = 500;

// what goes on the wire for a send: one line, no injection of a second irc command
fn privmsg(chan: &str, text: &str, reply: Option<&str>, dupes: &mut Dupes) -> Option<String> {
  let text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_SEND).collect();
  let text = text.trim();
  if !ident::twitch_room(chan) || text.is_empty() {
    return None;
  }
  let tag = match reply.filter(|id| id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
    Some(id) => format!("@reply-parent-msg-id={id} "),
    None => String::new(),
  };
  let text = dupes.wire(chan, text);
  Some(format!("{tag}PRIVMSG #{chan} :{text}"))
}

// twitch's answer to a chat token it won't take (a network-wide NOTICE, no channel)
fn auth_rejected(l: &Line) -> bool {
  l.channel().is_none()
    && l.trailing().is_some_and(|t| {
      ["authentication failed", "Improperly formatted auth", "Login unsuccessful", "Invalid NICK"]
        .iter()
        .any(|k| t.contains(k))
    })
}

// owns the joined set; reconnects forever with backoff until the ui drops its sender
enum End {
  Quit,
  Reconnect,
  // identity changed: reconnect now, no backoff
  Relogin,
  // twitch turned our chat token down (carried, so the session can tell it from a newer one): read
  // anonymously now, and a fresh token will log us in again
  Refused(String),
}

// how many recent lines a catch-up asks for: more the longer chat was out of reach (~10 a second is a
// busy channel), never fewer than a hundred or more than the history holds
fn catch_limit(gap: Duration) -> usize {
  (gap.as_secs() as usize).saturating_mul(10).clamp(100, super::recent::LIMIT)
}

pub async fn run(
  mut rx: mpsc::UnboundedReceiver<Cmd>,
  sess: mpsc::UnboundedSender<super::session::Cmd>,
  http: reqwest::Client,
  out: Out,
) {
  let mut chans = BTreeSet::new();
  let mut creds: Option<Creds> = None;
  let mut attempts = 0u32;
  // when we last lost twitch: set once there has been a connection, so the next one fills the gap
  let mut lost: Option<Instant> = None;
  let mut pending: VecDeque<Sent> = VecDeque::new();
  let mut dupes = Dupes::default();
  loop {
    let mut why = "can't reach twitch";
    match timeout(HANDSHAKE_LIMIT, connect_async_with_config(URL, Some(ws_config(MAX_FRAME)), false)).await {
      Ok(Ok((ws, _))) => {
        let started = Instant::now();
        out.status(if creds.is_some() { "twitch connected" } else { "twitch connected (read-only)" });
        if let Some(since) = lost.take() {
          let chans: Vec<_> = chans.iter().cloned().collect();
          tokio::spawn(super::recent::catch_up(http.clone(), chans, catch_limit(since.elapsed()), out.clone()));
        }
        let end = session(ws, &mut rx, &mut chans, &mut creds, &mut pending, &mut dupes, &out).await;
        // a line still waiting on twitch's word when the socket went: say so rather than let it vanish
        unsure(&out, &mut pending);
        lost = Some(Instant::now());
        match end {
          End::Quit => return,
          End::Relogin => {
            attempts = 0;
            continue;
          }
          End::Refused(token) => {
            let _ = sess.send(super::session::Cmd::ChatRefused(token));
            attempts = 0;
            continue;
          }
          End::Reconnect => why = "lost twitch",
        }
        if started.elapsed() >= STABLE {
          attempts = 0;
        }
      }
      Ok(Err(e)) => {
        eprintln!("twitch connect: {e}");
      }
      Err(_) => eprintln!("twitch connect: handshake timed out"),
    }
    attempts += 1;
    let wait = backoff(attempts);
    super::retrying(&out, why, attempts, wait);
    let wait = sleep(wait);
    tokio::pin!(wait);
    loop {
      tokio::select! {
        _ = &mut wait => break,
        cmd = rx.recv() => match cmd {
          Some(Cmd::Login(c)) => {
            creds = c;
            break;
          }
          Some(Cmd::Send { chan, .. }) => unsent(&out, chan, "twitch is reconnecting, so that wasn't sent"),
          Some(c) => { apply(&mut chans, c); }
          None => return,
        },
      }
    }
  }
}

// each line that was waiting on twitch when the socket died: it may or may not have gone out
fn unsure(out: &Out, pending: &mut VecDeque<Sent>) {
  for p in pending.drain(..) {
    let said: String = p.text.chars().take(60).collect();
    unsent(out, p.chan, &format!("twitch dropped, so this may not have sent: {said}"));
  }
}

fn unsent(out: &Out, chan: Box<str>, why: &str) {
  out.chat(chan, Chat::Msg(Message::system(now_ms(), Platform::Twitch, why)));
}

fn apply(chans: &mut BTreeSet<Box<str>>, c: Cmd) -> Option<String> {
  match c {
    Cmd::Join(ch) if ident::twitch_room(&ch) => chans.insert(ch.clone()).then(|| format!("JOIN #{ch}")),
    Cmd::Part(ch) => chans.remove(&ch).then(|| format!("PART #{ch}")),
    _ => None,
  }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// runs until the socket ends; `pending` (lines waiting on twitch's word) is the caller's, so what the
// socket took with it can be told to the user
async fn session(
  ws: Socket,
  rx: &mut mpsc::UnboundedReceiver<Cmd>,
  chans: &mut BTreeSet<Box<str>>,
  creds: &mut Option<Creds>,
  pending: &mut VecDeque<Sent>,
  dupes: &mut Dupes,
  out: &Out,
) -> End {
  let (mut tx, mut read) = ws.split();
  let (pass, nick) = match creds {
    Some(c) => (format!("oauth:{}", c.token), c.nick.clone()),
    None => ("SCHMOOPIIE".to_owned(), format!("justinfan{}", 10000 + now_ms() % 80000)),
  };
  let mut hello =
    vec!["CAP REQ :twitch.tv/tags twitch.tv/commands".to_owned(), format!("PASS {pass}"), format!("NICK {nick}")];
  let mut me_id: Box<str> = "".into();
  // batched joins stay under the 20-per-10s anon limit for normal tab counts
  let names: Vec<_> = chans.iter().map(|c| format!("#{c}")).collect();
  for chunk in names.chunks(20) {
    hello.push(format!("JOIN {}", chunk.join(",")));
  }
  for l in hello {
    if tx.send(Ws::text(l)).await.is_err() {
      return End::Reconnect;
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
          Some(Ok(Ws::Close(_))) | None | Some(Err(_)) => return End::Reconnect,
          Some(Ok(_)) => continue,
        };
        last_rx = Instant::now();
        for raw in text.split("\r\n").filter(|l| !l.is_empty()) {
          if let Some(p) = raw.strip_prefix("PING") {
            if tx.send(Ws::text(format!("PONG{p}"))).await.is_err() {
              return End::Reconnect;
            }
            continue;
          }
          let Some(line) = parse(raw) else { continue };
          match line.command {
            "RECONNECT" => return End::Reconnect,
            "GLOBALUSERSTATE" => {
              me_id = line.tag("user-id").filter(|u| u.bytes().all(|b| b.is_ascii_digit())).unwrap_or_default().into();
            }
            // a bad/expired token: say so and fall back to reading anonymously while the session gets a new one
            "NOTICE" if creds.is_some() && auth_rejected(&line) => {
              out.status("twitch didn't accept your login, getting a new one (read-only for now)");
              return End::Refused(creds.take().map(|c| c.token).unwrap_or_default());
            }
            // our line went through: show it the way others see it
            "USERSTATE" if line.tag("id").is_some() => {
              let ch = line.channel().unwrap_or("");
              if !out.chat(ch.into(), Chat::Mod(can_mod(&line))) {
                return End::Quit;
              }
              if let Some(i) = pending.iter().position(|p| &*p.chan == ch) {
                let Some(p) = pending.remove(i) else { continue };
                let mut m = own_message(&line, &nick, &me_id, &p.text);
                (m.reply_to, m.reply_text) = p.reply.map(|r| (r.nick, clamp(&r.text, REPLY_TEXT).into())).unzip();
                if !out.chat(p.chan, Chat::Msg(m)) {
                  return End::Quit;
                }
              }
              continue;
            }
            // twitch refused the line (rate limit, duplicate, banned…): its notice is shown, drop the wait
            "NOTICE" if line.tag("msg-id").is_some_and(|id| id.starts_with("msg_")) => {
              if let Some(i) = line.channel().and_then(|ch| pending.iter().position(|p| &*p.chan == ch)) {
                pending.remove(i);
              }
            }
            _ => {}
          }
          if let Some((ch, ev)) = to_chat(&line) && !out.chat(ch, ev) {
            return End::Quit;
          }
        }
      }
      cmd = rx.recv() => {
        let line = match cmd {
          None => return End::Quit,
          Some(Cmd::Login(c)) => {
            *creds = c;
            return End::Relogin;
          }
          Some(Cmd::Send { chan, text, reply }) => {
            if creds.is_none() {
              unsent(out, chan, "twitch isn't linked: link it at heatsync.org, or log in with twitch (ctrl-l)");
              continue;
            }
            let Some(l) = privmsg(&chan, &text, reply.as_ref().map(|r| &*r.id), dupes) else { continue };
            if pending.len() == MAX_PENDING {
              pending.pop_front();
            }
            pending.push_back(Sent { chan, text, reply });
            l
          }
          Some(c) => match apply(chans, c) {
            Some(l) => l,
            None => continue,
          },
        };
        if tx.send(Ws::text(line)).await.is_err() {
          return End::Reconnect;
        }
      }
      _ = tick.tick() => {
        if last_rx.elapsed() > SILENCE_LIMIT {
          out.status("twitch went quiet, reconnecting");
          return End::Reconnect;
        }
        if tx.send(Ws::text("PING :tmi.twitch.tv")).await.is_err() {
          return End::Reconnect;
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
  fn userstate_badges_say_if_we_moderate() {
    let st = |b: &str| format!("@badges={b};color=#fff;display-name=Me :tmi.twitch.tv USERSTATE #forsen");
    for (b, want) in [("moderator/1,subscriber/6", true), ("broadcaster/1", true), ("subscriber/6", false), ("", false)]
    {
      let raw = st(b);
      let l = parse(&raw).unwrap();
      let (_, ev) = to_chat(&l).unwrap();
      assert!(matches!(ev, Chat::Mod(x) if x == want), "{b}");
    }
  }

  // a real nl_kripp history line: valueless tags (`flags`, `emotes`) sit around the ones we read
  #[test]
  fn valueless_tags_do_not_hide_the_user_id() {
    let raw = "@flags;subscriber=1;first-msg=0;rm-received-ts=1793376500527;display-name=ennortix;emotes;color=#FF0000;turbo=0;tmi-sent-ts=1793376500347;user-type;mod=0;badge-info=subscriber/55;user-id=37005319;badges=subscriber/2048,ewc-2026-diamond/1;id=2f3d789f;room-id=29795919;historical=1 :ennortix!ennortix@ennortix.tmi.twitch.tv PRIVMSG #nl_kripp :watching the mentalist";
    let l = parse(raw).unwrap();
    let (_, Chat::Msg(m)) = to_chat(&l).unwrap() else { panic!() };
    assert_eq!(&*m.user_id, "37005319");
  }

  #[test]
  fn parses_privmsg() {
    let l = parse(PRIV).unwrap();
    assert_eq!(l.command, "PRIVMSG");
    assert_eq!(l.nick, Some("xqc"));
    assert_eq!(l.params, vec!["#forsen", "hello Kappa Keepo"]);
    let (ch, Chat::Msg(m)) = to_chat(&l).unwrap() else { panic!() };
    assert_eq!(&*ch, "forsen");
    assert_eq!(&*m.nick, "Xqc");
    assert_eq!(m.ts, 1700000000000);
    assert!(m.has(flag::FIRST));
    assert_eq!(m.emotes.len(), 2);
    assert_eq!(&*m.badges[0].0, "moderator");
    let (r, url) = &m.emotes[0];
    assert_eq!(&m.body[r.start as usize..r.end as usize], "Kappa");
    assert!(url.contains("/v2/25/"));
    // the color arrives as sent: the view lifts it (or not) when it draws
    assert_eq!(m.color, 0x0000ff);
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
    let max = usize::MAX;
    assert!(twitch_emotes(&format!("25:0-{max},{max}-{max}"), "hi").is_empty(), "no overflow");
  }

  #[test]
  fn a_line_shows_at_most_fifty_twitch_emotes() {
    let body = "Kappa ".repeat(120);
    let tag = format!("25:{}", (0..120).map(|i| format!("{}-{}", i * 6, i * 6 + 4)).collect::<Vec<_>>().join(","));
    let e = twitch_emotes(&tag, &body);
    assert_eq!(e.len(), super::super::relay::MAX_EMOTES);
    assert!(e.windows(2).all(|w| w[0].0.start < w[1].0.start), "the first fifty by position");
  }

  #[test]
  fn twitch_turning_a_login_down_is_told_from_chat() {
    let notice = |s: &str| format!(":tmi.twitch.tv NOTICE * :{s}");
    for said in ["Login authentication failed", "Improperly formatted auth", "Login unsuccessful", "Invalid NICK"] {
      let raw = notice(said);
      assert!(auth_rejected(&parse(&raw).unwrap()), "{said}");
    }
    // a room's own notice that happens to mention a login is not twitch refusing ours
    let room = ":tmi.twitch.tv NOTICE #c :Login authentication failed";
    assert!(!auth_rejected(&parse(room).unwrap()));
    let other = notice("This room is now in slow mode");
    assert!(!auth_rejected(&parse(&other).unwrap()));
  }

  #[test]
  fn a_catch_up_asks_for_more_the_longer_chat_was_gone() {
    assert_eq!(catch_limit(Duration::from_secs(0)), 100);
    assert_eq!(catch_limit(Duration::from_secs(30)), 300);
    assert_eq!(catch_limit(Duration::from_secs(86_400)), super::super::recent::LIMIT);
  }

  #[test]
  fn lines_waiting_on_twitch_when_the_socket_died_say_so() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let mut pending: VecDeque<Sent> = VecDeque::new();
    pending.push_back(Sent { chan: "xqc".into(), text: "hello chat".into(), reply: None });
    pending.push_back(Sent { chan: "forsen".into(), text: "x".repeat(300), reply: None });
    unsure(&out, &mut pending);
    assert!(pending.is_empty());
    let lines: Vec<(Box<str>, String)> = rx
      .try_iter()
      .filter_map(|e| match e {
        crate::net::NetEvent::Chat(ch, Chat::Msg(m)) => Some((ch, m.body.to_string())),
        _ => None,
      })
      .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(&*lines[0].0, "xqc");
    assert_eq!(lines[0].1, "twitch dropped, so this may not have sent: hello chat");
    assert!(lines[1].1.len() < 120, "a long line is cut: {}", lines[1].1.len());
  }

  #[test]
  fn action_and_unescape() {
    let l = parse("@display-name=A\\sB :a!a@a PRIVMSG #c :\x01ACTION waves\x01").unwrap();
    let (_, Chat::Msg(m)) = to_chat(&l).unwrap() else { panic!() };
    assert_eq!(m.kind, Kind::Action);
    assert_eq!(&*m.body, "waves");
    assert_eq!(&*m.nick, "A B");
  }

  #[test]
  fn a_reply_carries_its_parents_words() {
    let raw =
      "@reply-parent-display-name=wollip;reply-parent-msg-body=is\\sthis\\son;id=1 :b!b@b PRIVMSG #c :@wollip yes";
    let (_, Chat::Msg(m)) = to_chat(&parse(raw).unwrap()).unwrap() else { panic!() };
    assert_eq!((m.reply_to.as_deref(), m.reply_text.as_deref()), (Some("wollip"), Some("is this on")));
    let long = format!("@reply-parent-display-name=w;reply-parent-msg-body={} :b!b@b PRIVMSG #c :x", "a".repeat(5000));
    let (_, Chat::Msg(m)) = to_chat(&parse(&long).unwrap()).unwrap() else { panic!() };
    assert_eq!(m.reply_text.map(|t| t.len()), Some(REPLY_TEXT), "bounded");
  }

  #[test]
  fn clears() {
    let l = parse("@ban-duration=600 :tmi.twitch.tv CLEARCHAT #c :baduser").unwrap();
    assert!(matches!(to_chat(&l), Some((_, Chat::ClearUser { secs: Some(600), .. }))));
    let l = parse(":tmi.twitch.tv CLEARCHAT #c").unwrap();
    assert!(matches!(to_chat(&l), Some((_, Chat::ClearAll))));
    let l = parse("@login=x;target-msg-id=abc :tmi.twitch.tv CLEARMSG #c :bad").unwrap();
    assert!(matches!(to_chat(&l), Some((_, Chat::ClearMsg(id, _))) if &*id == "abc"));
  }

  #[test]
  fn usernotice_without_body_is_system() {
    let l = parse("@system-msg=x\\sraided;login=x;display-name=X :tmi.twitch.tv USERNOTICE #c").unwrap();
    let (_, Chat::Msg(m)) = to_chat(&l).unwrap() else { panic!() };
    assert_eq!(m.kind, Kind::System);
    assert_eq!(&*m.body, "x raided");
  }

  // each twitch event row says what it is by color (ext noticeKind); a cheer carries its bits
  #[test]
  fn usernotices_say_what_kind_of_event_they_are() {
    let kind = |id: &str, extra: &str| {
      let raw = format!("@msg-id={id};{extra}system-msg=x;login=a;display-name=A :tmi.twitch.tv USERNOTICE #c");
      let (_, Chat::Msg(m)) = to_chat(&parse(&raw).unwrap()).unwrap() else { panic!() };
      (m.notice(), m.kind)
    };
    assert_eq!(kind("resub", ""), (Notice::Sub, Kind::System));
    assert_eq!(kind("submysterygift", "").0, Notice::Sub);
    assert_eq!(kind("raid", "").0, Notice::Raid);
    assert_eq!(kind("bitsbadgetier", "").0, Notice::Bits);
    assert_eq!(kind("viewermilestone", "msg-param-category=watch-streak;").0, Notice::Streak);
    assert_eq!(kind("viewermilestone", "").0, Notice::Milestone);
    assert_eq!(kind("sharedchatnotice", "source-msg-id=subgift;").0, Notice::Sub);
    assert_eq!(kind("ritual", "").0, Notice::Plain);
    // a resub with words stays the person's line, the event above it
    let raw = "@msg-id=resub;system-msg=A\\ssubscribed;login=a :tmi.twitch.tv USERNOTICE #c :hype";
    let (_, Chat::Msg(m)) = to_chat(&parse(raw).unwrap()).unwrap() else { panic!() };
    assert_eq!((m.kind, m.notice(), &*m.body), (Kind::Chat, Notice::Sub, "hype"));
    // an announcement has no system words: it stays the person's line, the chip says what it is (layout)
    let raw = "@msg-id=announcement;login=a :tmi.twitch.tv USERNOTICE #c :listen up";
    let (_, Chat::Msg(m)) = to_chat(&parse(raw).unwrap()).unwrap() else { panic!() };
    assert_eq!((m.kind, m.notice(), m.system.as_deref(), &*m.body), (Kind::Chat, Notice::Announce, None, "listen up"));
    let bits = |tag: &str| {
      let raw = format!("@bits={tag};id=1 :a!a@a PRIVMSG #c :Cheer100 gg");
      let (_, Chat::Msg(m)) = to_chat(&parse(&raw).unwrap()).unwrap() else { panic!() };
      m.x.map_or(0, |x| x.bits)
    };
    assert_eq!(bits("100"), 100);
    assert_eq!((bits("-5"), bits("x"), bits("99999999999")), (0, 0, 0));
    let (_, Chat::Msg(m)) = to_chat(&parse(PRIV).unwrap()).unwrap() else { panic!() };
    assert!(m.x.is_none(), "plain chat pays nothing");
  }

  // someone's redeem with words: their own line under the ext's label
  #[test]
  fn a_redeem_is_labelled_and_stays_their_line() {
    let raw = "@custom-reward-id=0b2a-11;id=9;login=bob :bob!bob@bob PRIVMSG #c :play the song";
    let (_, Chat::Msg(m)) = to_chat(&parse(raw).unwrap()).unwrap() else { panic!() };
    assert_eq!((m.kind, m.notice(), m.system.as_deref()), (Kind::Chat, Notice::Redeem, Some("◆ channel point redeem")));
    assert_eq!(&*m.body, "play the song");
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
  fn send_line_is_one_command() {
    assert_eq!(
      privmsg("forsen", "hi\r\nJOIN #x", None, &mut Dupes::default()).as_deref(),
      Some("PRIVMSG #forsen :hi  JOIN #x")
    );
    assert_eq!(
      privmsg("forsen", "yo", Some("abc-1"), &mut Dupes::default()).as_deref(),
      Some("@reply-parent-msg-id=abc-1 PRIVMSG #forsen :yo")
    );
    assert_eq!(privmsg("forsen", "yo", Some("a b;x=1"), &mut Dupes::default()).as_deref(), Some("PRIVMSG #forsen :yo"));
    assert_eq!(privmsg("forsen", "   ", None, &mut Dupes::default()), None);
    assert_eq!(privmsg("Bad Chan", "x", None, &mut Dupes::default()), None);
  }

  #[test]
  fn repeat_goes_out_tagged_and_shows_clean() {
    let mut d = Dupes::default();
    assert_eq!(privmsg("c", "hi", None, &mut d).as_deref(), Some("PRIVMSG #c :hi"));
    let l = privmsg("c", "hi", None, &mut d).unwrap();
    assert_eq!(l, "PRIVMSG #c :hi \u{E0000}");
    // the line as twitch (or history) hands it back: shown without the tag
    let raw = format!("@id=1 :a!a@a {}", l);
    let m = match to_chat(&parse(&raw).unwrap()).unwrap().1 {
      Chat::Msg(m) => m,
      _ => panic!("msg"),
    };
    assert_eq!(&*m.body, "hi");
    let raw = "@id=2 :a!a@a PRIVMSG #c :\u{1}ACTION waves \u{E0000}\u{1}";
    let m = match to_chat(&parse(raw).unwrap()).unwrap().1 {
      Chat::Msg(m) => m,
      _ => panic!("msg"),
    };
    assert_eq!((&*m.body, m.kind), ("waves", Kind::Action));
    assert_eq!(privmsg("c", "hi", None, &mut d).as_deref(), Some("PRIVMSG #c :hi"));
  }
}
