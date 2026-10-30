// all network io lives on one background thread running a current-thread tokio runtime.
// the ui talks to it through `Net` and hears back through winit user events
pub mod irc;
mod recent;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, mpsc};
use winit::event_loop::EventLoopProxy;

use crate::emotes::{Provider, Set, providers};
use crate::images::{ImgId, cache, decode, url};
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

pub enum Cmd {
  Join(Box<str>),
  Part(Box<str>),
  // roomstate told us the channel's twitch id: fetch its 7tv/bttv/ffz sets
  Room(Box<str>, Box<str>),
  Image(ImgId, Box<str>, u32),
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
    let _ = dec.send(DecodeJob {
      id,
      url,
      h,
      bytes,
      store: false,
    });
    return;
  }
  let Ok(_permit) = gate.acquire().await else { return };
  let res = match http.get(&*url).send().await {
    Ok(r) if r.status().is_success() => r.bytes().await.map_err(|e| e.to_string()),
    Ok(r) => Err(format!("http {}", r.status().as_u16())),
    Err(e) => Err(e.to_string()),
  };
  match res {
    Ok(b) if b.len() <= MAX_IMAGE_BYTES => {
      let _ = dec.send(DecodeJob {
        id,
        url,
        h,
        bytes: b.to_vec(),
        store: true,
      });
    }
    Ok(_) => fail("too large".into()),
    Err(e) => fail(e),
  }
}

async fn channel_emotes(http: reqwest::Client, ch: Box<str>, room: Box<str>, out: Out) {
  if !room.bytes().all(|b| b.is_ascii_digit()) {
    return;
  }
  for (p, u) in providers::channel_urls(&room) {
    match providers::fetch(&http, &u).await {
      Ok(v) => {
        out.send(NetEvent::Emotes(Some(ch.clone()), p, providers::parse_channel(p, &v)));
      }
      Err(e) => out.status(format!("{p:?} emotes for {ch}: {e}").to_lowercase()),
    }
  }
}

async fn global_emotes(http: reqwest::Client, out: Out) {
  type Parse = fn(&serde_json::Value) -> Set;
  let sets: [(Provider, &str, Parse); 3] = [
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
  let Some(dec) = spawn_decoder(out.clone()) else {
    return out.status("image decoder failed to start");
  };
  let gate = Arc::new(Semaphore::new(IMAGE_FETCHES));
  while let Some(cmd) = rx.recv().await {
    match cmd {
      Cmd::Join(ch) => {
        tokio::spawn(recent::fetch(http.clone(), ch.clone(), out.clone()));
        let _ = irc_tx.send(irc::Cmd::Join(ch));
      }
      Cmd::Part(ch) => {
        let _ = irc_tx.send(irc::Cmd::Part(ch));
      }
      Cmd::Room(ch, room) => {
        tokio::spawn(channel_emotes(http.clone(), ch, room, out.clone()));
      }
      Cmd::Image(id, u, h) => {
        tokio::spawn(image(http.clone(), gate.clone(), dec.clone(), id, u, h, out.clone()));
      }
    }
  }
}
