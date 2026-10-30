// all network io lives on one background thread running a current-thread tokio runtime.
// the ui talks to it through `Net` and hears back through winit user events
pub mod irc;
mod recent;

use std::time::Duration;

use tokio::sync::mpsc;
use winit::event_loop::EventLoopProxy;

use irc::Chat;

pub const USER_AGENT: &str = concat!("heatsync-chat/", env!("CARGO_PKG_VERSION"));
pub const ORIGIN: &str = "https://heatsync.org";

#[derive(Debug)]
pub enum NetEvent {
  Chat(Box<str>, Chat),
  // recent history for a freshly joined channel, oldest first
  Backfill(Box<str>, Vec<Chat>),
  Status(String),
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
  while let Some(cmd) = rx.recv().await {
    match cmd {
      Cmd::Join(ch) => {
        tokio::spawn(recent::fetch(http.clone(), ch.clone(), out.clone()));
        let _ = irc_tx.send(irc::Cmd::Join(ch));
      }
      Cmd::Part(ch) => {
        let _ = irc_tx.send(irc::Cmd::Part(ch));
      }
    }
  }
}
