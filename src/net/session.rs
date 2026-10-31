// the logged-in heatsync session: token on disk (0600), who we are, the twitch irc credential it
// brokers, and kick sends. one task, so login/logout/renewal never race each other
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::{Instant, sleep_until};

use super::auth::{self, Check, Provider};
use super::emote_ops::{self, EmoteCmd};
use super::hs_ws;
use super::irc::{self, Creds};
use super::upload;
use super::{NetEvent, ORIGIN, Out};
use crate::model::message::{Message, Platform};
use crate::store;
use crate::util::time::now_ms;

const TOKEN: &str = "token";
// /api/auth/check slides the session once <24h is left; checking this often keeps it alive forever
const RENEW: Duration = Duration::from_secs(6 * 3600);
const RETRY: Duration = Duration::from_secs(60);

pub enum Cmd {
  // which platform to sign in with, and the twitch permission pack to ask for ("base", or
  // "whispers" to add twitch whispers; the other platforms ignore it)
  Login(Provider, &'static str),
  Logout,
  Emote(EmoteCmd),
  Upload(std::path::PathBuf),
  Pixels(u32, u32, Vec<u8>),
  Pick,
  Kick { chan: Box<str>, text: String, reply: Option<Box<str>> },
  Social(super::social::Cmd),
  Mod(super::modact::Req),
}

struct State {
  http: reqwest::Client,
  irc: UnboundedSender<irc::Cmd>,
  // the heatsync socket signs in with the same token (whispers are pushed to signed-in sockets)
  hs: UnboundedSender<hs_ws::Cmd>,
  hs_as: Option<String>,
  out: Out,
  token: Option<String>,
  // what irc is logged in as, so a renewal that changes nothing doesn't reconnect chat
  irc_as: Option<(String, String)>,
}

pub async fn run(
  http: reqwest::Client,
  mut rx: UnboundedReceiver<Cmd>,
  irc: UnboundedSender<irc::Cmd>,
  hs: UnboundedSender<hs_ws::Cmd>,
  out: Out,
) {
  let token = store::read(TOKEN).map(|s| s.trim().to_owned()).filter(|t| auth::valid_token(t));
  let mut st = State { http, irc, hs, hs_as: None, out, token, irc_as: None };
  let mut next = Instant::now();
  loop {
    tokio::select! {
      c = rx.recv() => match c {
        None => return,
        Some(Cmd::Login(provider, pack)) => {
          st.out.status(format!("finish logging in with {} in your browser…", provider.name()));
          eprintln!("login: opening the browser");
          match auth::login(&st.http, provider, pack).await {
            Ok(t) => {
              st.token = Some(t);
              next = Instant::now();
            }
            Err(e) => {
              eprintln!("login failed: {e}");
              st.out.status(format!("login failed: {e}"));
            }
          }
        }
        Some(Cmd::Logout) => {
          if let Some(t) = st.token.take() {
            auth::logout(&st.http, &t).await;
          }
          st.forget("logged out");
        }
        Some(Cmd::Emote(c)) => match &st.token {
          Some(t) => emote_ops::run(&st.http, t, c, &st.out).await,
          None => st.out.status("log in first (ctrl-l)"),
        },
        Some(Cmd::Upload(p)) => st.upload(p),
        Some(Cmd::Pixels(w, h, px)) => st.pixels(w, h, px),
        Some(Cmd::Pick) => st.pick(),
        Some(Cmd::Kick { chan, text, reply }) => st.kick(chan, text, reply).await,
        Some(Cmd::Mod(r)) => {
          tokio::spawn(super::modact::run(st.http.clone(), st.token.clone(), r, st.out.clone()));
        }
        Some(Cmd::Social(c)) => {
          tokio::spawn(super::social::run(st.http.clone(), st.token.clone(), c, st.out.clone()));
        }
      },
      _ = sleep_until(next), if st.token.is_some() => {
        next = Instant::now() + if st.check().await { RENEW } else { RETRY };
      }
    }
  }
}

impl State {
  // uploads run on their own task: a 45MB video must not stall renewals or kick sends
  fn upload(&self, path: std::path::PathBuf) {
    match &self.token {
      Some(t) => {
        tokio::spawn(upload::run(self.http.clone(), t.clone(), path, self.out.clone()));
      }
      None => {
        self.out.send(NetEvent::Upload(upload::Upload::Failed("log in to upload (ctrl-l)".into())));
      }
    }
  }

  fn pixels(&self, w: u32, h: u32, px: Vec<u8>) {
    match &self.token {
      Some(t) => {
        tokio::spawn(upload::run_pixels(self.http.clone(), t.clone(), w, h, px, self.out.clone()));
      }
      None => {
        self.out.send(NetEvent::Upload(upload::Upload::Failed("log in to upload (ctrl-l)".into())));
      }
    }
  }

  // the dialog blocks, so it gets a blocking thread; its pick is then uploaded like a drop
  fn pick(&self) {
    let (http, token, out) = (self.http.clone(), self.token.clone(), self.out.clone());
    tokio::spawn(async move {
      let r = tokio::task::spawn_blocking(upload::choose).await.unwrap_or(upload::Upload::Cancelled);
      match (r, token) {
        (upload::Upload::Url(p), Some(t)) => {
          out.status("uploading…");
          upload::run(http, t, p.into(), out).await
        }
        (upload::Upload::Url(_), None) => {
          out.send(NetEvent::Upload(upload::Upload::Failed("log in to upload (ctrl-l)".into())));
        }
        (other, _) => {
          out.send(NetEvent::Upload(other));
        }
      }
    });
  }

  fn forget(&mut self, why: &str) {
    self.token = None;
    store::remove(TOKEN);
    if self.irc_as.take().is_some() {
      let _ = self.irc.send(irc::Cmd::Login(None));
    }
    if self.hs_as.take().is_some() {
      let _ = self.hs.send(hs_ws::Cmd::Auth(None));
    }
    self.out.send(NetEvent::Me(None));
    self.out.status(why);
  }

  // false = couldn't reach heatsync; try again soon
  async fn check(&mut self) -> bool {
    let Some(t) = self.token.clone() else { return true };
    let me = match auth::check(&self.http, &t).await {
      Ok(Check::Ok(me, renewed)) => {
        let t = renewed.unwrap_or(t);
        if let Err(e) = store::write_secret(TOKEN, &t) {
          self.out.status(format!("couldn't save login: {e}"));
        }
        self.token = Some(t);
        me
      }
      Ok(Check::Expired) => {
        self.forget("session expired, log in again (ctrl-l)");
        return true;
      }
      Err(e) => {
        self.out.status(format!("heatsync unreachable ({e})"));
        return false;
      }
    };
    let twitch = me.twitch.is_some();
    if self.hs_as != self.token {
      self.hs_as = self.token.clone();
      let _ = self.hs.send(hs_ws::Cmd::Auth(self.token.clone()));
    }
    self.out.send(NetEvent::Me(Some(me)));
    if let Some(t) = &self.token {
      tokio::spawn(emote_ops::load_blocked(self.http.clone(), t.clone(), self.out.clone()));
    }
    if !twitch {
      // a kick-only or youtube-only login has no twitch chat: drop a twitch login left from before
      if self.irc_as.take().is_some() {
        let _ = self.irc.send(irc::Cmd::Login(None));
      }
      return true;
    }
    let Some(t) = self.token.as_deref() else { return true };
    match auth::chat_token(&self.http, t).await {
      Ok((token, nick)) => {
        let now = Some((token.clone(), nick.clone()));
        if self.irc_as != now {
          self.irc_as = now;
          let _ = self.irc.send(irc::Cmd::Login(Some(Creds { token, nick })));
        }
      }
      Err(e) => self.out.status(format!("twitch chat: {e}")),
    }
    true
  }

  async fn kick(&mut self, chan: Box<str>, text: String, reply: Option<Box<str>>) {
    let why = match &self.token {
      None => Some("log in to send (ctrl-l)".to_owned()),
      Some(t) => kick_send(&self.http, t, &chan, &text, reply.as_deref()).await.err(),
    };
    if let Some(why) = why {
      let m = Message::system(now_ms(), Platform::Kick, format!("not sent: {why}"));
      self.out.send(NetEvent::Relay { platform: Platform::Kick, key: chan, msgs: vec![m], history: false });
    }
  }
}

// the line comes back through the heatsync relay like everyone else's
async fn kick_send(
  http: &reqwest::Client,
  token: &str,
  chan: &str,
  text: &str,
  reply: Option<&str>,
) -> Result<(), String> {
  let mut body = json!({ "channel": chan, "text": text });
  if let Some(id) = reply {
    body["reply_to_message_id"] = json!(id);
  }
  let r = http
    .post(format!("{ORIGIN}/api/kick/chat/send"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(body.to_string())
    .send()
    .await
    .map_err(|e| e.to_string())?;
  if r.status().is_success() {
    return Ok(());
  }
  let code = r.status().as_u16();
  let v: Value = r.bytes().await.ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
  Err(match v.get("error_code").and_then(Value::as_str) {
    Some("not_linked") => "link kick on heatsync.org first".into(),
    Some("scope_missing") => "re-link kick on heatsync.org to allow sending".into(),
    Some("token_expired") => "kick login expired, re-link on heatsync.org".into(),
    Some("rate_limited") => "slow down".into(),
    _ => {
      v.get("error").and_then(Value::as_str).map_or_else(|| format!("http {code}"), |e| e.chars().take(100).collect())
    }
  })
}
