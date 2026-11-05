// the logged-in heatsync session: token on disk (0600), who we are, the twitch irc credential it
// brokers, and kick sends. one task, so login/logout/renewal never race each other
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::{Instant, sleep_until};

use super::auth::{self, Chat, Check, Provider};
use super::emote_ops::{self, EmoteCmd};
use super::hs_ws;
use super::http;
use super::irc::{self, Creds};
use super::upload;
use super::{NetEvent, ORIGIN, Out};
use crate::model::message::{Message, Platform};
use crate::store;
use crate::util::time::now_ms;

// the reply to one kick send
const MAX_REPLY: usize = 64 << 10;
const TOKEN: &str = "token";
// /api/auth/check slides the session once <24h is left; checking this often keeps it alive forever
const RENEW: Duration = Duration::from_secs(6 * 3600);

// when a step that failed is tried again: ~16s, ~32s, then about a minute, never a hot loop
// (`fails` counts the failures in a row, ≥1)
fn retry_in(fails: u32) -> Duration {
  super::backoff(fails.saturating_add(3))
}

// how long to wait after a step: the slow renewal when it worked, `retry_in` when it didn't
fn next_in(ok: bool, fails: u32) -> Duration {
  if ok { RENEW } else { retry_in(fails) }
}

pub enum Cmd {
  // which platform to sign in with, and the twitch permission pack to ask for ("base", or
  // "whispers" to add twitch whispers; the other platforms ignore it)
  Login(Provider, &'static str),
  // attach another platform to the signed-in account; `pack` is "base", or "post" to also allow
  // sending youtube chat (the youtube opt-in)
  Link(Provider, &'static str),
  // give up a sign-in that's waiting on the browser
  CancelLogin,
  // twitch irc turned this chat token down: get a fresh one
  ChatRefused(String),
  Logout,
  // heatsync says the account changed (a link or unlink): check it now, not at the next renewal
  Recheck,
  Emote(EmoteCmd),
  Inventory(super::inventory::Op),
  // answered once every emote op asked before it is done (they run in order): a chat line waits on it
  Barrier(tokio::sync::oneshot::Sender<()>),
  Upload(std::path::PathBuf),
  Picture(Vec<u8>),
  Pick,
  Kick { chan: Box<str>, text: String, reply: Option<Box<str>> },
  Youtube { chan: Box<str>, text: String },
  Social(super::social::Cmd),
  Mod(super::modact::Req),
  Pin(super::pinact::Op),
  Held(super::held::Cmd),
  Following,
  Blocked(Box<str>, super::blocked::Op),
  Modes(Box<str>, crate::model::modes::Set),
  StreamInfo(Box<str>),
  Pred(Box<str>),
  Suite(Box<str>, crate::model::modsuite::Cmd),
  Chan(Box<str>, crate::model::chantools::Cmd),
  Tools(Box<str>, super::modtools::Op),
  TagCard(Box<str>),
  Mute(super::mutes::Op),
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
  // a sign-in waiting on the browser, off the loop so renewals and sends carry on meanwhile
  login: Option<Waiting>,
  // the session is fine but its twitch chat login isn't in yet: only that step is retried
  chat_due: bool,
  // steps failed in a row (0 = all well): sets how long to wait before the next try
  fails: u32,
  // when heatsync last vouched for the session; the slow renewal also runs if retries drag on
  checked: Instant,
  // the chat token irc just refused: a refresh that hands back the same one is not given to irc again
  refused: Option<String>,
  refusals: u32,
  // the next step is the whole account check, even while only the chat login is being retried
  recheck: bool,
}

// one sign-in in the browser: which, and the task holding the loopback port
struct Waiting {
  provider: Provider,
  pack: &'static str,
  link: bool,
  task: tokio::task::JoinHandle<Result<String, String>>,
}

// the browser sign-in's answer (pends forever while none is waiting)
async fn login_result(w: &mut Option<Waiting>) -> Result<String, String> {
  match w {
    Some(w) => (&mut w.task).await.unwrap_or_else(|e| {
      eprintln!("login task: {e}");
      Err("login stopped, try again".into())
    }),
    None => std::future::pending().await,
  }
}

pub async fn run(
  http: reqwest::Client,
  mut rx: UnboundedReceiver<Cmd>,
  irc: UnboundedSender<irc::Cmd>,
  hs: UnboundedSender<hs_ws::Cmd>,
  out: Out,
) {
  let token = store::read(TOKEN).map(|s| s.trim().to_owned()).filter(|t| auth::valid_token(t));
  let mut st = State {
    http,
    irc,
    hs,
    hs_as: None,
    out,
    token,
    irc_as: None,
    login: None,
    chat_due: false,
    fails: 0,
    checked: Instant::now(),
    refused: None,
    refusals: 0,
    recheck: false,
  };
  let mut next = Instant::now();
  loop {
    tokio::select! {
      c = rx.recv() => match c {
        None => return,
        Some(Cmd::Login(provider, pack)) => st.start_login(provider, pack, None),
        Some(Cmd::Link(provider, pack)) => match st.token.clone() {
          None => st.out.status("log in first (ctrl-l)"),
          Some(cur) => st.start_login(provider, pack, Some(cur)),
        },
        Some(Cmd::CancelLogin) => st.cancel_login("login cancelled"),
        Some(Cmd::ChatRefused(bad)) => {
          if st.chat_refused(&bad) {
            next = Instant::now() + if st.refusals <= 1 { Duration::ZERO } else { retry_in(st.refusals) };
          }
        }
        Some(Cmd::Logout) => {
          st.cancel_login("");
          if let Some(t) = st.token.take() {
            auth::logout(&st.http, &t).await;
          }
          st.forget("logged out");
        }
        Some(Cmd::Emote(c)) => match &st.token {
          Some(t) => emote_ops::run(&st.http, t, c, &st.out).await,
          None => st.out.status("log in first (ctrl-l)"),
        },
        Some(Cmd::Inventory(op)) => {
          tokio::spawn(super::inventory::run(st.http.clone(), st.token.clone(), op, st.out.clone()));
        }
        Some(Cmd::Recheck) => {
          st.recheck = true;
          next = Instant::now();
        }
        Some(Cmd::Barrier(done)) => {
          let _ = done.send(());
        }
        Some(Cmd::Upload(p)) => st.upload(p),
        Some(Cmd::Picture(b)) => st.picture(b),
        Some(Cmd::Pick) => st.pick(),
        Some(Cmd::Kick { chan, text, reply }) => st.kick(chan, text, reply).await,
        Some(Cmd::Youtube { chan, text }) => st.youtube(chan, text).await,
        Some(Cmd::Mute(op)) => {
          tokio::spawn(super::mutes::run(st.http.clone(), st.token.clone(), op));
        }
        Some(Cmd::Mod(r)) => {
          tokio::spawn(super::modact::run(st.http.clone(), st.token.clone(), r, st.out.clone()));
        }
        Some(Cmd::Pin(op)) => {
          tokio::spawn(super::pinact::run(st.http.clone(), st.token.clone(), op, st.out.clone()));
        }
        Some(Cmd::Held(c)) => {
          tokio::spawn(super::held::run(st.http.clone(), st.token.clone(), c, st.out.clone()));
        }
        Some(Cmd::TagCard(tag)) => {
          tokio::spawn(super::tag_card(st.http.clone(), st.token.clone(), tag, st.out.clone()));
        }
        Some(Cmd::Following) => {
          tokio::spawn(super::following::run(st.http.clone(), st.token.clone(), st.out.clone()));
        }
        Some(Cmd::Blocked(room, op)) => {
          tokio::spawn(super::blocked::run(st.http.clone(), st.token.clone(), room, op, st.out.clone()));
        }
        Some(Cmd::Modes(chan, set)) => {
          tokio::spawn(super::modes::run(st.http.clone(), st.token.clone(), chan, set, st.out.clone()));
        }
        Some(Cmd::StreamInfo(chan)) => {
          tokio::spawn(super::modes::info(st.http.clone(), chan, st.out.clone()));
        }
        Some(Cmd::Pred(chan)) => {
          tokio::spawn(super::pred::fetch(st.http.clone(), chan, st.out.clone()));
        }
        Some(Cmd::Suite(chan, c)) => {
          tokio::spawn(super::modsuite::run(st.http.clone(), st.token.clone(), chan, c, st.out.clone()));
        }
        Some(Cmd::Chan(chan, c)) => {
          tokio::spawn(super::chantools::run(st.http.clone(), st.token.clone(), chan, c, st.out.clone()));
        }
        Some(Cmd::Tools(chan, op)) => {
          tokio::spawn(super::modtools::run(st.http.clone(), st.token.clone(), chan, op, st.out.clone()));
        }
        Some(Cmd::Social(c)) => {
          tokio::spawn(super::social::run(st.http.clone(), st.token.clone(), c, st.out.clone()));
        }
      },
      res = login_result(&mut st.login), if st.login.is_some() => {
        if st.login_done(res) {
          next = Instant::now();
        }
      }
      _ = sleep_until(next), if st.token.is_some() => {
        next = Instant::now() + st.tick().await;
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

  fn picture(&self, bytes: Vec<u8>) {
    match &self.token {
      Some(t) => {
        tokio::spawn(upload::run_picture(self.http.clone(), t.clone(), bytes, self.out.clone()));
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
    self.chat_due = false;
    self.fails = 0;
    self.refused = None;
    self.refusals = 0;
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

  // open the browser and wait for it off to the side. one at a time: a sign-in still waiting is
  // dropped first, which closes its loopback port
  fn start_login(&mut self, provider: Provider, pack: &'static str, link_as: Option<String>) {
    self.cancel_login("");
    let link = link_as.is_some();
    let name = provider.name();
    self.out.status(if link {
      format!("finish linking {name} in your browser (esc cancels)")
    } else {
      format!("finish logging in with {name} in your browser (esc cancels)")
    });
    eprintln!("login: opening the browser");
    let http = self.http.clone();
    let task = tokio::spawn(async move { auth::login(&http, provider, pack, link_as.as_deref()).await });
    self.login = Some(Waiting { provider, pack, link, task });
    self.out.send(NetEvent::LoginWait(true));
  }

  // stop a sign-in that's waiting (`say` is what the user hears; "" = nothing)
  fn cancel_login(&mut self, say: &str) {
    let Some(w) = self.login.take() else { return };
    w.task.abort();
    self.out.send(NetEvent::LoginWait(false));
    if !say.is_empty() {
      self.out.status(say);
    }
  }

  // the browser came back (or gave up). true = check the new session now
  fn login_done(&mut self, res: Result<String, String>) -> bool {
    let Some(w) = self.login.take() else { return false };
    self.out.send(NetEvent::LoginWait(false));
    match res {
      Ok(t) => {
        // a link keeps the same account, refreshed: the re-check picks up the new link (twitch chat
        // logs in by itself)
        self.token = Some(t);
        self.fresh();
        if w.link {
          self.out.status(if w.pack == "post" {
            "youtube posting allowed".to_owned()
          } else {
            format!("linked {}", w.provider.name())
          });
        }
        true
      }
      Err(e) => {
        eprintln!("login failed: {e}");
        self.out.status(if w.link {
          format!("couldn't link {}: {e}", w.provider.name())
        } else {
          format!("couldn't log in: {e}")
        });
        false
      }
    }
  }

  // a new session: nothing carried over from an earlier one's troubles
  fn fresh(&mut self) {
    self.chat_due = false;
    self.fails = 0;
    self.refused = None;
    self.refusals = 0;
  }

  // irc turned the chat token down. true = it was the one we handed over, so ask for another
  fn chat_refused(&mut self, bad: &str) -> bool {
    if self.irc_as.as_ref().is_none_or(|(t, _)| t != bad) {
      return false;
    }
    // irc reads anonymously now; the next token gets it signed in again
    self.irc_as = None;
    self.refused = Some(bad.to_owned());
    self.refusals += 1;
    self.chat_due = true;
    true
  }

  // the next step only retries the twitch chat login (the session itself was vouched for lately, and
  // heatsync didn't say the account changed)
  fn chat_only(&mut self) -> bool {
    !std::mem::take(&mut self.recheck) && self.chat_due && self.checked.elapsed() < RENEW
  }

  // one scheduled step; how long until the next. a failed step comes back soon, a good one in hours
  async fn tick(&mut self) -> Duration {
    // retries only redo the twitch step, but the session itself is still vouched for on schedule
    let res = if self.chat_only() { self.link_chat().await } else { self.check().await };
    match res {
      Ok(()) => {
        self.fails = 0;
        next_in(true, 0)
      }
      Err(why) => {
        self.fails = self.fails.saturating_add(1);
        let wait = next_in(false, self.fails);
        // the first tries just say it's coming back; a stubborn one says what else to try. a long
        // outage is a few lines, not one a minute
        let why = if self.fails >= 3 && why.contains("chat login") {
          format!("{why} (if it keeps happening, log in again with ctrl-l)")
        } else {
          why
        };
        super::retrying(&self.out, &why, self.fails, wait);
        wait
      }
    }
  }

  // Err = what went wrong in plain words; the caller retries
  async fn check(&mut self) -> Result<(), String> {
    let Some(t) = self.token.clone() else { return Ok(()) };
    let me = match auth::check(&self.http, &t).await {
      Ok(Check::Ok(me, renewed)) => {
        let t = renewed.unwrap_or(t);
        if let Err(e) = store::write_secret(TOKEN, &t) {
          eprintln!("saving the login: {e}");
          self.out.status("couldn't save your login, you'll have to log in again next time");
        }
        self.token = Some(t);
        self.checked = Instant::now();
        // a renewal is a clean slate for twitch's refusals too
        self.refusals = 0;
        me
      }
      Ok(Check::Expired) => {
        self.forget("you're logged out, log in again (ctrl-l)");
        return Ok(());
      }
      Err(e) => return Err(super::http::offline("heatsync", &e)),
    };
    if self.hs_as != self.token {
      self.hs_as = self.token.clone();
      let _ = self.hs.send(hs_ws::Cmd::Auth(self.token.clone()));
    }
    self.out.send(NetEvent::Me(Some(me)));
    if let Some(t) = &self.token {
      tokio::spawn(emote_ops::load_blocked(self.http.clone(), t.clone(), self.out.clone()));
      tokio::spawn(super::mutes::load(self.http.clone(), t.clone(), self.out.clone()));
    }
    self.link_chat().await
  }

  // one heatsync login covers every linked platform: ask for the twitch chat token whichever
  // provider signed in, and let heatsync say whether twitch is linked
  async fn link_chat(&mut self) -> Result<(), String> {
    let Some(t) = self.token.as_deref() else { return Ok(()) };
    let got = auth::chat_token(&self.http, t).await;
    match chat_step(got, self.refused.as_deref()) {
      Step::Login(token, nick) => {
        let now = Some((token.clone(), nick.clone()));
        if self.irc_as != now {
          self.irc_as = now;
          let _ = self.irc.send(irc::Cmd::Login(Some(Creds { token, nick })));
        }
        self.refused = None;
        self.chat_due = false;
        Ok(())
      }
      Step::Unlinked => {
        // no twitch linked: drop a twitch login left from before
        if self.irc_as.take().is_some() {
          let _ = self.irc.send(irc::Cmd::Login(None));
        }
        self.chat_due = false;
        Ok(())
      }
      Step::Expired => {
        self.forget("you're logged out, log in again (ctrl-l)");
        Ok(())
      }
      Step::Retry(why) => {
        self.chat_due = true;
        Err(why)
      }
    }
  }

  // like kick: success shows up through the relay, a refusal is a system line in the tab
  async fn youtube(&mut self, chan: Box<str>, text: String) {
    let why = match &self.token {
      None => Some("log in to send (ctrl-l)".to_owned()),
      Some(t) => super::youtube::send(&self.http, t, &chan, &text).await.err(),
    };
    if let Some(why) = why {
      let m = Message::system(now_ms(), Platform::Youtube, format!("not sent to youtube: {why}"));
      self.out.send(NetEvent::Relay { platform: Platform::Youtube, key: chan, msgs: vec![m], history: false });
    }
  }

  async fn kick(&mut self, chan: Box<str>, text: String, reply: Option<Box<str>>) {
    let why = match &self.token {
      None => Some("log in to send (ctrl-l)".to_owned()),
      Some(t) => kick_send(&self.http, t, &chan, &text, reply.as_deref()).await.err(),
    };
    // a line that got through frees the box; a refusal only speaks when its words name a lasting reason
    let told = match why.as_deref() {
      None => Some(None),
      Some(w) => crate::model::lock::from_kick(w).map(Some),
    };
    if let Some(cause) = told {
      self.out.send(NetEvent::Lock(Platform::Kick, chan.clone(), cause));
    }
    if let Some(why) = why {
      let m = Message::system(now_ms(), Platform::Kick, format!("not sent to kick: {why}"));
      self.out.send(NetEvent::Relay { platform: Platform::Kick, key: chan, msgs: vec![m], history: false });
    }
  }
}

// what the chat-token answer means for twitch chat
#[derive(Debug, PartialEq)]
enum Step {
  Login(String, String),
  // no twitch linked
  Unlinked,
  // heatsync dropped the session
  Expired,
  // try again soon; the plain reason
  Retry(String),
}

// `refused` = a token irc just turned down: handing it back would loop
fn chat_step(got: Result<Chat, String>, refused: Option<&str>) -> Step {
  match got {
    Ok(Chat::Ready(t, _)) if refused == Some(&t) => Step::Retry("twitch didn't accept your chat login".into()),
    Ok(Chat::Ready(t, n)) => Step::Login(t, n),
    Ok(Chat::Unlinked) => Step::Unlinked,
    Ok(Chat::Expired) => Step::Expired,
    Err(e) => {
      eprintln!("twitch chat token: {e}");
      Step::Retry("can't get your twitch chat login".into())
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
    .map_err(|e| http::offline("heatsync", e))?;
  if r.status().is_success() {
    return Ok(());
  }
  let code = r.status().as_u16();
  let v: Value = http::read_json(r, MAX_REPLY).await.unwrap_or(Value::Null);
  Err(match v.get("error_code").and_then(Value::as_str) {
    Some("not_linked") => "link kick on heatsync.org first".into(),
    Some("scope_missing") => "re-link kick on heatsync.org to allow sending".into(),
    Some("token_expired") => "kick login expired, re-link on heatsync.org".into(),
    Some("rate_limited") => "slow down".into(),
    _ => {
      v.get("error").and_then(Value::as_str).map_or_else(|| http::plain_status(code), |e| e.chars().take(100).collect())
    }
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicBool, Ordering};
  use tokio::sync::mpsc::unbounded_channel;

  type Rig = (State, std::sync::mpsc::Receiver<NetEvent>, UnboundedReceiver<irc::Cmd>);

  fn rig() -> Rig {
    let (itx, irx) = unbounded_channel();
    let (htx, _hrx) = unbounded_channel();
    let (otx, orx) = std::sync::mpsc::channel();
    let st = State {
      http: reqwest::Client::new(),
      irc: itx,
      hs: htx,
      hs_as: None,
      out: Out::Pipe(otx),
      token: Some("a.b.c".into()),
      irc_as: None,
      login: None,
      chat_due: false,
      fails: 0,
      checked: Instant::now(),
      refused: None,
      refusals: 0,
      recheck: false,
    };
    (st, orx, irx)
  }

  // account:changed: the next step reads the whole account again, even mid chat-login retries
  #[test]
  fn a_changed_account_is_checked_whole() {
    let (mut st, _out, _irc) = rig();
    st.chat_due = true;
    st.recheck = true;
    assert!(!st.chat_only(), "the account first");
    assert!(st.chat_only(), "then back to the chat retries");
  }

  fn said(rx: &std::sync::mpsc::Receiver<NetEvent>) -> Vec<String> {
    rx.try_iter().filter_map(|e| if let NetEvent::Status(s) = e { Some(s) } else { None }).collect()
  }

  #[test]
  fn a_failed_chat_token_asks_again_soon_not_in_six_hours() {
    // the old step said "all fine" here and left chat read-only until the next renewal
    let step = chat_step(Err("http 503".into()), None);
    assert_eq!(step, Step::Retry("can't get your twitch chat login".into()));
    assert!(matches!(chat_step(Err("x".into()), Some("tok")), Step::Retry(_)));
    for fails in 1..=8 {
      let wait = next_in(false, fails);
      assert!(wait >= Duration::from_secs(10) && wait <= Duration::from_secs(75), "{fails}: {wait:?}");
      assert!(wait < RENEW / 100);
    }
    assert_eq!(next_in(true, 0), RENEW);
    // the first retry is quicker than the last, and it never goes quiet for long
    assert!(next_in(false, 1) < next_in(false, 4));
  }

  #[test]
  fn the_chat_answer_maps_to_a_plain_next_step() {
    let ready = || Ok(Chat::Ready("tok".into(), "me".into()));
    assert_eq!(chat_step(ready(), None), Step::Login("tok".into(), "me".into()));
    assert_eq!(chat_step(ready(), Some("older")), Step::Login("tok".into(), "me".into()));
    assert_eq!(chat_step(Ok(Chat::Unlinked), None), Step::Unlinked);
    assert_eq!(chat_step(Ok(Chat::Expired), None), Step::Expired);
    // the very token twitch just turned down is not handed back
    assert!(matches!(chat_step(ready(), Some("tok")), Step::Retry(_)));
  }

  #[tokio::test]
  async fn twitch_refusing_the_login_gets_a_fresh_token_and_never_loops() {
    let (mut st, _out, _irc) = rig();
    st.irc_as = Some(("tok".into(), "me".into()));
    // a refusal of some older token is none of our business
    assert!(!st.chat_refused("older"));
    assert!(st.irc_as.is_some() && !st.chat_due);
    // the one irc is using: irc reads anonymously now, the session fetches another
    assert!(st.chat_refused("tok"));
    assert!(st.irc_as.is_none() && st.chat_due);
    assert_eq!((st.refused.as_deref(), st.refusals), (Some("tok"), 1));
    // and if heatsync hands back the same one, it waits instead of logging irc in with it again
    assert!(matches!(chat_step(Ok(Chat::Ready("tok".into(), "me".into())), st.refused.as_deref()), Step::Retry(_)));
    // a second refusal waits longer
    st.irc_as = Some(("tok2".into(), "me".into()));
    assert!(st.chat_refused("tok2"));
    assert_eq!(st.refusals, 2);
  }

  struct Flag(std::sync::Arc<AtomicBool>);
  impl Drop for Flag {
    fn drop(&mut self) {
      self.0.store(true, Ordering::SeqCst);
    }
  }

  #[tokio::test]
  async fn a_sign_in_waiting_on_the_browser_can_be_cancelled() {
    let (mut st, out, _irc) = rig();
    let dropped = std::sync::Arc::new(AtomicBool::new(false));
    let held = Flag(dropped.clone());
    let task = tokio::spawn(async move {
      let _held = held;
      std::future::pending::<Result<String, String>>().await
    });
    st.login = Some(Waiting { provider: Provider::Twitch, pack: "base", link: false, task });
    st.cancel_login("login cancelled");
    assert!(st.login.is_none());
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert!(dropped.load(Ordering::SeqCst), "the waiting task (and its loopback port) is gone");
    let events: Vec<String> = out
      .try_iter()
      .map(|e| match e {
        NetEvent::LoginWait(on) => format!("wait {on}"),
        NetEvent::Status(s) => s,
        _ => "?".into(),
      })
      .collect();
    assert_eq!(events, ["wait false", "login cancelled"]);
    // nothing waiting: nothing to say
    st.cancel_login("login cancelled");
    assert!(said(&out).is_empty());
  }

  #[tokio::test]
  async fn a_finished_sign_in_is_taken_or_explained() {
    let (mut st, out, _irc) = rig();
    let waiting = |link| Waiting {
      provider: Provider::Kick,
      pack: "base",
      link,
      task: tokio::spawn(std::future::pending::<Result<String, String>>()),
    };
    st.login = Some(waiting(false));
    assert!(st.login_done(Ok("x.y.z".into())), "a good token is checked at once");
    assert_eq!(st.token.as_deref(), Some("x.y.z"));
    st.login = Some(waiting(true));
    assert!(st.login_done(Ok("x.y.z".into())));
    assert!(said(&out).contains(&"linked kick".to_string()));
    st.login = Some(waiting(false));
    assert!(!st.login_done(Err("login timed out".into())));
    assert_eq!(said(&out), ["couldn't log in: login timed out"]);
    assert!(!st.login_done(Ok("late.token.here".into())), "a stray answer with no sign-in waiting is ignored");
  }

  #[tokio::test]
  async fn starting_another_sign_in_drops_the_old_one() {
    let (mut st, out, _irc) = rig();
    let dropped = std::sync::Arc::new(AtomicBool::new(false));
    let held = Flag(dropped.clone());
    let task = tokio::spawn(async move {
      let _held = held;
      std::future::pending::<Result<String, String>>().await
    });
    st.login = Some(Waiting { provider: Provider::Twitch, pack: "base", link: false, task });
    // what start_login does first (it would go on to open a browser, which a test must not)
    st.cancel_login("");
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert!(dropped.load(Ordering::SeqCst));
    assert!(said(&out).is_empty(), "replacing a sign-in is quiet");
  }
}
