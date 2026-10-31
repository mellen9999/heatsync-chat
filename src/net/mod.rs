// all network io lives on one background thread running a current-thread tokio runtime.
// the ui talks to it through `Net` and hears back through winit user events
pub mod auth;
mod cosmetics;
pub mod emote_ops;
pub mod hs_ws;
pub mod irc;
mod links;
pub mod modact;
pub mod mutes;
pub mod pin;
mod recent;
pub mod relay;
mod senders;
mod session;
pub mod seventv;
pub mod social;
pub mod upload;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, mpsc};
use winit::event_loop::EventLoopProxy;

use crate::badges::{self, Source, TwitchSet, UserBadges};
use crate::emotes::{Provider, Set, providers};
use crate::heatsync::senders::Set as SenderSet;
use crate::images::{ImgId, cache, decode, url};
use crate::model::message::{Message, Platform};
use irc::Chat;

pub const USER_AGENT: &str = concat!("heatsync-chat/", env!("CARGO_PKG_VERSION"));
pub const ORIGIN: &str = "https://heatsync.org";

pub enum NetEvent {
  Chat(Box<str>, Chat),
  // recent history for a freshly joined channel, oldest first
  Backfill(Box<str>, Vec<Chat>),
  Status(String),
  // channel = None for globals
  Emotes(Option<Box<str>>, Provider, Set),
  Image(ImgId, u32, Result<decode::Decoded, String>),
  TwitchBadges(Option<Box<str>>, TwitchSet),
  // kick/youtube lines relayed by heatsync, keyed by kick slug / youtube handle
  Relay { platform: Platform, key: Box<str>, msgs: Vec<Message>, history: bool },
  RelayClear { platform: Platform, key: Box<str>, login: Box<str> },
  YtStatus { key: Box<str>, status: Box<str> },
  // a twitch tab's publicly linked accounts: kick slug, youtube (handle, UC id)
  Links { tab: Box<str>, kick: Option<Box<str>>, yt: Option<(Box<str>, Option<Box<str>>)> },
  UserBadges(Source, UserBadges),
  // heatsync inventories for twitch user ids
  Senders(Vec<(u64, SenderSet)>),
  SendersStale { ids: Vec<u64>, ver: Option<Box<str>>, removed: Option<Box<str>> },
  // logged in as / logged out
  Me(Option<auth::Me>),
  // the server's blocked-emote hashes for this login
  BlockedHashes(Vec<Box<str>>),
  // our own heatsync inventory changed (an add, or a removal of `removed`): refetch it
  Inventory { removed: Option<Box<str>> },
  Upload(upload::Upload),
  // 7tv paints + badges for twitch ids (None = wears nothing)
  Cosmetics(Vec<(u64, Option<crate::cosmetics::Cosmetic>)>),
  // 7tv says this twitch user's cosmetics changed: ask again
  CosmeticsStale(u64),
  // heatsync says these twitch ids changed their paint: ask again
  CosmeticsChanged(Vec<u64>),
  // a live edit of a channel's 7tv emote set
  EmoteDelta(Box<str>, crate::emotes::live::Delta),
  // a channel's pinned line was set or cleared (None)
  Pin { platform: Platform, key: Box<str>, pin: Option<crate::model::pin::Pin> },
  // went live / offline / switched game, already paired and de-duplicated
  Stream(crate::model::stream_events::Line),
  // a chat-rate spike in a channel (text ready to show)
  Moment { platform: Platform, key: Box<str>, text: Box<str> },
  // twitch whispers: one pushed live, or the history read when the tab first opened
  // `epoch` tags a history read with the login it was asked under (live pushes carry none)
  Whispers { items: Vec<crate::model::whisper::Whisper>, live: bool, epoch: Option<u32> },
  // a send settled: who it went to (resolved), or why not
  // the feed: a page, why it failed (http status, 0 = no answer), a thread, a vote or a post settling,
  // and a post pushed live
  Feed(social::Page),
  FeedFailed { status: u16 },
  Thread { id: Box<str>, res: Result<(crate::model::post::Post, Vec<crate::model::post::Post>), u16> },
  Voted { id: Box<str>, res: Result<(bool, Option<u32>), u16> },
  Posted { res: Result<crate::model::post::Post, u16> },
  FeedLive(crate::model::post::Post),
  // your mute / block lists on heatsync, read after a login
  Lists(mutes::Lists),
  // a mute changed on another device
  MuteSync(mutes::Sync),
  // a user card's facts (or why not: http status, 0 = no answer)
  Card { platform: Platform, login: Box<str>, res: Result<crate::model::card::Info, u16> },
  // a mod action settled
  ModDone(Box<modact::Done>),
  WhisperSent { key: u64, res: Result<crate::model::whisper::Who, crate::model::whisper::Fail> },
}

// event sink back to the ui. false = the ui is gone
#[derive(Clone)]
pub struct Out(EventLoopProxy<NetEvent>);

impl Out {
  pub fn chat(&self, ch: Box<str>, ev: Chat) -> bool {
    self.0.send_event(NetEvent::Chat(ch, ev)).is_ok()
  }

  pub fn send(&self, ev: NetEvent) -> bool {
    self.0.send_event(ev).is_ok()
  }

  pub fn status(&self, s: impl Into<String>) {
    let _ = self.0.send_event(NetEvent::Status(s.into()));
  }
}

// exponential, capped at 60s, ±20% jitter so a fleet of clients doesn't stampede
pub fn backoff(attempt: u32) -> Duration {
  let base = 2f64.powi(attempt.min(6) as i32).min(60.0);
  let jitter = 0.8 + 0.4 * (crate::util::time::now_ms() % 1000) as f64 / 1000.0;
  Duration::from_secs_f64(base * jitter)
}

// the line being answered: its platform message id, and @nick for our own echo
pub struct Reply {
  pub id: Box<str>,
  pub nick: Box<str>,
}

pub enum Cmd {
  Join(Box<str>),
  Part(Box<str>),
  // roomstate told us the channel's twitch id: fetch its 7tv/bttv/ffz sets
  Room(Box<str>, Box<str>),
  Image(ImgId, Box<str>, u32),
  // heatsync relay rooms
  Hs(hs_ws::Cmd),
  // recent history for a relayed source: (platform, routing key, archive key)
  Backfill(Platform, Box<str>, Box<str>),
  // twitch user id → their heatsync inventory (version set after a ws invalidation)
  Sender(u64, Option<Box<str>>),
  // sign in with this platform
  Login(auth::Provider),
  Logout,
  Emote(emote_ops::EmoteCmd),
  // upload this file / ask a file dialog for one, then upload it
  Upload(std::path::PathBuf),
  // a pasted picture: width, height, rgba bytes
  Pixels(u32, u32, Vec<u8>),
  Pick,
  // chatters' 7tv paints + badges
  Cosmetics(u64),
  // our own twitch id, for watching our own 7tv cosmetics
  SevenTvMe(Option<u64>),
  Send { platform: Platform, chan: Box<str>, text: String, reply: Option<Reply> },
  // whispers and feed calls bound to the login
  Social(social::Cmd),
  // log in again asking for one more permission pack (twitch whispers, mod)
  Relink(&'static str),
  // a ban, timeout, delete… in a room we moderate
  Mod(modact::Req),
  // your mute list changed (synced while logged in)
  Mute(mutes::Op),
}

const IMAGE_FETCHES: usize = 6;
const DISK_CACHE_MAX: u64 = 256 << 20;
const MAX_IMAGE_BYTES: usize = 8 << 20;

struct DecodeJob {
  id: ImgId,
  url: Box<str>,
  h: u32,
  bytes: Vec<u8>,
  // fresh from the network: persist before decoding
  store: bool,
}

// decoding is cpu work; keep it off the network loop
fn spawn_decoder(out: Out) -> Option<std::sync::mpsc::Sender<DecodeJob>> {
  let (tx, rx) = std::sync::mpsc::channel::<DecodeJob>();
  let spawned = std::thread::Builder::new().name("decode".into()).spawn(move || {
    cache::prune(DISK_CACHE_MAX);
    for j in rx {
      if j.store {
        cache::write(&j.url, &j.bytes);
      }
      let res = decode::decode(&j.bytes, j.h);
      if !out.send(NetEvent::Image(j.id, j.h, res)) {
        return;
      }
    }
  });
  spawned.ok().map(|_| tx)
}

async fn image(
  http: reqwest::Client,
  gate: Arc<Semaphore>,
  dec: std::sync::mpsc::Sender<DecodeJob>,
  id: ImgId,
  url: Box<str>,
  h: u32,
  out: Out,
) {
  let fail = |e: String| {
    out.send(NetEvent::Image(id, h, Err(e)));
  };
  if !url::allowed(&url) {
    return fail("host not allowed".into());
  }
  if let Some(bytes) = cache::read(&url) {
    let _ = dec.send(DecodeJob { id, url, h, bytes, store: false });
    return;
  }
  let Ok(_permit) = gate.acquire().await else { return };
  match capped(http.get(&*url).send().await).await {
    Ok(bytes) => {
      let _ = dec.send(DecodeJob { id, url, h, bytes, store: true });
    }
    Err(e) => fail(e),
  }
}

// the body, abandoned as soon as it passes the cap (a hostile host can't make us buffer it all)
async fn capped(res: Result<reqwest::Response, reqwest::Error>) -> Result<Vec<u8>, String> {
  let mut r = res.map_err(|e| e.to_string())?;
  if !r.status().is_success() {
    return Err(format!("http {}", r.status().as_u16()));
  }
  if r.content_length().is_some_and(|n| n > MAX_IMAGE_BYTES as u64) {
    return Err("too large".into());
  }
  let mut out = Vec::new();
  while let Some(c) = r.chunk().await.map_err(|e| e.to_string())? {
    if out.len() + c.len() > MAX_IMAGE_BYTES {
      return Err("too large".into());
    }
    out.extend_from_slice(&c);
  }
  Ok(out)
}

// image fetches follow redirects only between allowlisted hosts (`/api/img` answers with a 302)
fn image_client() -> Option<reqwest::Client> {
  use reqwest::redirect::Policy;
  reqwest::Client::builder()
    .user_agent(USER_AGENT)
    .connect_timeout(Duration::from_secs(8))
    .timeout(Duration::from_secs(12))
    .redirect(Policy::custom(|a| {
      if a.previous().len() >= 3 || !url::allowed(a.url().as_str()) { a.stop() } else { a.follow() }
    }))
    .build()
    .ok()
}

async fn channel_emotes(
  http: reqwest::Client,
  ch: Box<str>,
  room: Box<str>,
  stv: mpsc::UnboundedSender<seventv::Cmd>,
  out: Out,
) {
  if !room.bytes().all(|b| b.is_ascii_digit()) {
    return;
  }
  for (p, u) in providers::channel_urls(&room) {
    match providers::fetch(&http, &u).await {
      Ok(v) => {
        if p == Provider::SevenTv
          && let Some(set) = providers::seventv_set_id(&v)
        {
          // live edits for this channel's set; the broadcaster's own cosmetics too
          let user = providers::seventv_user_id(&v).zip(room.parse().ok());
          let _ = stv.send(seventv::Cmd::Watch { ch: ch.clone(), set, user });
        }
        out.send(NetEvent::Emotes(Some(ch.clone()), p, providers::parse_channel(p, &v)));
      }
      Err(e) => out.status(format!("{p:?} emotes for {ch}: {e}").to_lowercase()),
    }
  }
}

async fn global_badges(http: reqwest::Client, out: Out) {
  match badges::fetch::twitch_global(&http).await {
    Ok(s) => {
      out.send(NetEvent::TwitchBadges(None, s));
    }
    Err(e) => out.status(format!("twitch badges: {e}")),
  }
  type Parse = fn(&serde_json::Value) -> UserBadges;
  let third: [(Source, &str, Parse); 2] = [
    (Source::Chatterino, badges::fetch::CHATTERINO, badges::fetch::chatterino),
    (Source::Ffz, badges::fetch::FFZ_IDS, badges::fetch::ffz),
  ];
  for (src, u, parse) in third {
    match providers::fetch(&http, u).await {
      Ok(v) => {
        out.send(NetEvent::UserBadges(src, parse(&v)));
      }
      Err(e) => out.status(format!("{src:?} badges: {e}").to_lowercase()),
    }
  }
}

async fn channel_badges(http: reqwest::Client, ch: Box<str>, out: Out) {
  match badges::fetch::twitch_channel(&http, &ch).await {
    Ok(s) => {
      out.send(NetEvent::TwitchBadges(Some(ch), s));
    }
    Err(e) => out.status(format!("badges for {ch}: {e}")),
  }
}

async fn global_emotes(http: reqwest::Client, out: Out) {
  type Parse = fn(&serde_json::Value) -> Set;
  let sets: [(Provider, &str, Parse); 4] = [
    (Provider::Twitch, providers::HEATSYNC_GLOBAL, providers::twitch_global),
    (Provider::SevenTv, providers::SEVENTV_GLOBAL, providers::seventv),
    (Provider::Bttv, providers::BTTV_GLOBAL, providers::bttv_global),
    (Provider::Ffz, providers::FFZ_GLOBAL, providers::ffz_global),
  ];
  for (p, u, parse) in sets {
    match providers::fetch(&http, u).await {
      Ok(v) => {
        out.send(NetEvent::Emotes(None, p, parse(&v)));
      }
      Err(e) => out.status(format!("global {p:?} emotes: {e}").to_lowercase()),
    }
  }
}

pub struct Net {
  tx: mpsc::UnboundedSender<Cmd>,
}

impl Net {
  pub fn spawn(proxy: EventLoopProxy<NetEvent>) -> Net {
    let (tx, rx) = mpsc::unbounded_channel();
    let out = Out(proxy);
    let spawned = std::thread::Builder::new().name("net".into()).spawn(move || {
      let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => return out.status(format!("network runtime failed: {e}")),
      };
      rt.block_on(main(rx, out));
    });
    if let Err(e) = spawned {
      eprintln!("net thread: {e}");
    }
    Net { tx }
  }

  pub fn send(&self, c: Cmd) {
    let _ = self.tx.send(c);
  }
}

async fn main(mut rx: mpsc::UnboundedReceiver<Cmd>, out: Out) {
  let http = match reqwest::Client::builder()
    .user_agent(USER_AGENT)
    .connect_timeout(Duration::from_secs(8))
    .timeout(Duration::from_secs(12))
    .build()
  {
    Ok(c) => c,
    Err(e) => return out.status(format!("http client failed: {e}")),
  };
  let (irc_tx, irc_rx) = mpsc::unbounded_channel();
  tokio::spawn(irc::run(irc_rx, out.clone()));
  tokio::spawn(global_emotes(http.clone(), out.clone()));
  tokio::spawn(global_badges(http.clone(), out.clone()));
  let (hs_tx, hs_rx) = mpsc::unbounded_channel();
  let (pin_tx, pin_rx) = mpsc::unbounded_channel();
  tokio::spawn(pin::run(http.clone(), pin_rx, out.clone()));
  tokio::spawn(hs_ws::run(hs_rx, pin_tx, out.clone()));
  let hs_auth = hs_tx.clone();
  let (snd_tx, snd_rx) = mpsc::unbounded_channel();
  tokio::spawn(senders::run(http.clone(), snd_rx, out.clone()));
  let (sess_tx, sess_rx) = mpsc::unbounded_channel();
  tokio::spawn(session::run(http.clone(), sess_rx, irc_tx.clone(), hs_auth, out.clone()));
  let (cos_tx, cos_rx) = mpsc::unbounded_channel();
  tokio::spawn(cosmetics::run(http.clone(), cos_rx, out.clone()));
  let (stv_tx, stv_rx) = mpsc::unbounded_channel();
  tokio::spawn(seventv::run(http.clone(), stv_rx, stv_tx.clone(), out.clone()));
  let Some(dec) = spawn_decoder(out.clone()) else {
    return out.status("image decoder failed to start");
  };
  let Some(img_http) = image_client() else {
    return out.status("image client failed to start");
  };
  let gate = Arc::new(Semaphore::new(IMAGE_FETCHES));
  while let Some(cmd) = rx.recv().await {
    match cmd {
      Cmd::Join(ch) => {
        tokio::spawn(recent::fetch(http.clone(), ch.clone(), out.clone()));
        if irc::valid_channel(&ch) {
          tokio::spawn(channel_badges(http.clone(), ch.clone(), out.clone()));
          tokio::spawn(links::discover(http.clone(), ch.clone(), out.clone()));
          let _ = hs_tx.send(hs_ws::Cmd::Join(Platform::Twitch, ch.clone()));
        }
        let _ = irc_tx.send(irc::Cmd::Join(ch));
      }
      Cmd::Part(ch) => {
        let _ = stv_tx.send(seventv::Cmd::Unwatch(ch.clone()));
        let _ = hs_tx.send(hs_ws::Cmd::Leave(Platform::Twitch, ch.clone()));
        let _ = irc_tx.send(irc::Cmd::Part(ch));
      }
      Cmd::Hs(c) => {
        let _ = hs_tx.send(c);
      }
      Cmd::Backfill(p, key, archive) => {
        tokio::spawn(links::backfill(http.clone(), p, key, archive, out.clone()));
      }
      Cmd::Login(p) => {
        let _ = sess_tx.send(session::Cmd::Login(p, "base"));
      }
      Cmd::Mute(op) => {
        let _ = sess_tx.send(session::Cmd::Mute(op));
      }
      Cmd::Mod(r) => {
        let _ = sess_tx.send(session::Cmd::Mod(r));
      }
      Cmd::Relink(pack) => {
        let _ = sess_tx.send(session::Cmd::Login(auth::Provider::Twitch, pack));
      }
      Cmd::Social(c) => {
        let _ = sess_tx.send(session::Cmd::Social(c));
      }
      Cmd::Logout => {
        let _ = sess_tx.send(session::Cmd::Logout);
      }
      Cmd::Emote(c) => {
        let _ = sess_tx.send(session::Cmd::Emote(c));
      }
      Cmd::Upload(p) => {
        let _ = sess_tx.send(session::Cmd::Upload(p));
      }
      Cmd::Pixels(w, h, px) => {
        let _ = sess_tx.send(session::Cmd::Pixels(w, h, px));
      }
      Cmd::Pick => {
        let _ = sess_tx.send(session::Cmd::Pick);
      }
      Cmd::Send { platform: Platform::Twitch, chan, text, reply } => {
        let _ = irc_tx.send(irc::Cmd::Send { chan, text, reply });
      }
      Cmd::Send { platform: Platform::Kick, chan, text, reply } => {
        let _ = sess_tx.send(session::Cmd::Kick { chan, text, reply: reply.map(|r| r.id) });
      }
      Cmd::Send { platform: Platform::Youtube, chan, .. } => {
        let m = Message::system(crate::util::time::now_ms(), Platform::Youtube, "youtube is read-only here for now");
        out.send(NetEvent::Relay { platform: Platform::Youtube, key: chan, msgs: vec![m], history: false });
      }
      Cmd::Sender(id, ver) => {
        let _ = snd_tx.send((id, ver));
      }
      Cmd::Cosmetics(id) => {
        let _ = cos_tx.send(id);
      }
      Cmd::SevenTvMe(id) => {
        let _ = stv_tx.send(seventv::Cmd::Me(id));
      }
      Cmd::Room(ch, room) => {
        tokio::spawn(channel_emotes(http.clone(), ch, room, stv_tx.clone(), out.clone()));
      }
      Cmd::Image(id, u, h) => {
        tokio::spawn(image(img_http.clone(), gate.clone(), dec.clone(), id, u, h, out.clone()));
      }
    }
  }
}

#[cfg(test)]
mod live {
  use super::*;
  use winit::application::ApplicationHandler;
  use winit::event::WindowEvent;
  use winit::event_loop::{ActiveEventLoop, EventLoop};
  use winit::window::WindowId;

  // real network, no window: join nl_kripp, wait for ennortix's backfilled line, ask for their
  // cosmetics the way the ui does, expect the heatsync paint back. needs a wayland/x11 session
  // (an event loop, not a window): WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000
  struct Probe {
    net: Option<Net>,
    asked: bool,
    painted: Option<bool>,
    log: Vec<String>,
    deadline: std::time::Instant,
  }

  impl ApplicationHandler<NetEvent> for Probe {
    fn resumed(&mut self, el: &ActiveEventLoop) {
      self.net.as_ref().unwrap().send(Cmd::Join("nl_kripp".into()));
      el.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
        std::time::Instant::now() + Duration::from_secs(40),
      ));
    }
    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
      if std::time::Instant::now() > self.deadline {
        el.exit();
      }
    }
    fn user_event(&mut self, el: &ActiveEventLoop, ev: NetEvent) {
      match ev {
        NetEvent::Backfill(_, evs) if !self.asked => {
          let hit = evs.iter().any(|e| matches!(e, Chat::Msg(m) if &*m.user_id == "37005319"));
          self.log.push(format!("backfill {} lines, ennortix among them: {hit}", evs.len()));
          if hit {
            self.asked = true;
            self.net.as_ref().unwrap().send(Cmd::Cosmetics(37005319));
          }
        }
        NetEvent::Cosmetics(got) => {
          self.log.push(format!("cosmetics {:?}", got.iter().map(|(i, c)| (*i, c.is_some())).collect::<Vec<_>>()));
          self.painted = Some(got.iter().any(|(i, c)| *i == 37005319 && c.as_ref().is_some_and(|c| c.paint.is_some())));
          el.exit();
        }
        NetEvent::Status(s) => self.log.push(format!("status {s}")),
        _ => {}
      }
    }
  }

  #[test]
  #[ignore]
  fn live_ennortix_paint_arrives_through_net() {
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let el = EventLoop::<NetEvent>::with_user_event().with_any_thread(true).build().expect("event loop");
    let mut p = Probe {
      net: Some(Net::spawn(el.create_proxy())),
      asked: false,
      painted: None,
      log: Vec::new(),
      deadline: std::time::Instant::now() + Duration::from_secs(40),
    };
    let _ = el.run_app(&mut p);
    eprintln!("{}", p.log.join("\n"));
    assert_eq!(p.painted, Some(true));
  }
}
