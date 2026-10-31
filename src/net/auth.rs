// heatsync login for the app: rfc 8252 loopback + pkce (server side: heatsync lib/native-auth.ts).
// the browser does the twitch oauth; the app only ever sees a one-time code on 127.0.0.1, and that
// code is worthless without the verifier that never leaves this process
use std::time::Duration;

use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::ORIGIN;

const WAIT: Duration = Duration::from_secs(300);
const MAX_REQUEST: usize = 8 << 10;

fn b64url(b: &[u8]) -> String {
  const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
  for c in b.chunks(3) {
    let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
    for i in 0..=c.len() {
      s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
    }
  }
  s
}

fn random(n: usize) -> Result<Vec<u8>, String> {
  let mut b = vec![0u8; n];
  SystemRandom::new().fill(&mut b).map_err(|_| "no system randomness".to_string())?;
  Ok(b)
}

pub struct Pkce {
  pub verifier: String,
  pub challenge: String,
  pub state: String,
}

pub fn pkce() -> Result<Pkce, String> {
  let verifier = b64url(&random(32)?);
  let challenge = b64url(digest(&SHA256, verifier.as_bytes()).as_ref());
  Ok(Pkce { verifier, challenge, state: b64url(&random(18)?) })
}

// `GET /cb?code=..&state=.. HTTP/1.1` → (code, state)
fn callback(req: &str) -> Option<(String, String)> {
  let target = req.lines().next()?.strip_prefix("GET ")?.split(' ').next()?;
  let query = target.strip_prefix("/cb?")?;
  let (mut code, mut state) = (None, None);
  for kv in query.split('&') {
    match kv.split_once('=') {
      Some(("code", v)) => code = Some(v.to_string()),
      Some(("state", v)) => state = Some(v.to_string()),
      _ => {}
    }
  }
  Some((code?, state?))
}

fn page(body: &str) -> String {
  let html = format!(
    "<!doctype html><meta charset=utf-8><title>heatsync</title>\
     <body style=\"background:#000;color:#fff;font:15px monospace;padding:2em\">{body}</body>"
  );
  format!(
    "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{html}",
    html.len()
  )
}

// which platform's sign-in the browser opens. every one ends in the same heatsync session
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
  Twitch,
  Kick,
  Youtube,
}

impl Provider {
  pub fn name(self) -> &'static str {
    match self {
      Provider::Twitch => "twitch",
      Provider::Kick => "kick",
      Provider::Youtube => "youtube",
    }
  }

  // the server's login route. only twitch has permission packs (`scopes`); the others take none
  fn path(self) -> &'static str {
    match self {
      Provider::Twitch => "/api/auth/login",
      Provider::Kick => "/api/auth/kick/login",
      Provider::Youtube => "/api/auth/youtube/login",
    }
  }
}

// the browser address for one login. `pack` only rides on twitch (a fixed word, never user text)
// `link` is a one-shot ticket (see `link_ticket`): with it the browser attaches this platform to the
// account we're signed in as, instead of signing in as the platform's own account
fn login_url(provider: Provider, port: u16, p: &Pkce, pack: &str, link: Option<&str>) -> String {
  let mut url = format!("{ORIGIN}{}?native={port}&cc={}&ns={}", provider.path(), p.challenge, p.state);
  if let Some(t) = link {
    url.push_str("&nl=");
    url.push_str(t);
  }
  if provider == Provider::Twitch {
    url.push_str("&scopes=");
    url.push_str(pack);
  }
  url
}

// trades the signed-in session for a one-time ticket that lets a browser flow link onto this account
async fn link_ticket(http: &reqwest::Client, token: &str) -> Result<String, String> {
  let r =
    http.post(format!("{ORIGIN}/api/auth/native-link")).bearer_auth(token).send().await.map_err(|e| e.to_string())?;
  let ok = r.status().is_success();
  let expired = r.status().as_u16() == 401;
  let v = body(r).await.unwrap_or(Value::Null);
  match v.get("ticket").and_then(Value::as_str) {
    Some(t) if ok && t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()) => Ok(t.to_string()),
    _ if expired => Err("session expired, log in again (ctrl-l)".to_string()),
    _ => Err(v.get("error").and_then(Value::as_str).unwrap_or("couldn't start linking").chars().take(100).collect()),
  }
}

// opens the browser and waits (≤5 min) for the loopback; returns the heatsync session token.
// `link_as` = the current session token: the platform is then linked onto that account and the
// returned token is the same account's, refreshed
pub async fn login(
  http: &reqwest::Client,
  provider: Provider,
  pack: &str,
  link_as: Option<&str>,
) -> Result<String, String> {
  let ticket = match link_as {
    Some(t) => Some(link_ticket(http, t).await?),
    None => None,
  };
  let p = pkce()?;
  let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| format!("loopback: {e}"))?;
  let port = listener.local_addr().map_err(|e| e.to_string())?.port();
  let done = if ticket.is_some() {
    "linked. you can close this tab"
  } else {
    "logged in to heatsync chat. you can close this tab"
  };
  let url = login_url(provider, port, &p, pack, ticket.as_deref());
  open_browser(&url)?;
  let code = timeout(WAIT, async {
    loop {
      let Ok((mut sock, _)) = listener.accept().await else { continue };
      let mut buf = vec![0u8; MAX_REQUEST];
      let n = match timeout(Duration::from_secs(5), sock.read(&mut buf)).await {
        Ok(Ok(n)) => n,
        _ => continue,
      };
      let req = String::from_utf8_lossy(&buf[..n]);
      // stray hits (favicon, a port scan) get nothing and the wait goes on
      let Some((code, state)) = callback(&req) else { continue };
      if state != p.state {
        let _ = sock.write_all(page("login didn't match this app — try again from the app").as_bytes()).await;
        return Err("login state mismatch".to_string());
      }
      let _ = sock.write_all(page(done).as_bytes()).await;
      return Ok(code);
    }
  })
  .await
  .map_err(|_| "login timed out".to_string())??;
  exchange(http, &code, &p.verifier).await
}

async fn exchange(http: &reqwest::Client, code: &str, verifier: &str) -> Result<String, String> {
  let r = http
    .post(format!("{ORIGIN}/api/auth/native-exchange"))
    .header("content-type", "application/json")
    .body(json!({ "code": code, "verifier": verifier }).to_string())
    .send()
    .await
    .map_err(|e| e.to_string())?;
  let ok = r.status().is_success();
  let v = body(r).await?;
  match v.get("token").and_then(Value::as_str) {
    Some(t) if ok && valid_token(t) => Ok(t.to_string()),
    _ => Err(v.get("error").and_then(Value::as_str).unwrap_or("login failed").chars().take(100).collect()),
  }
}

async fn body(r: reqwest::Response) -> Result<Value, String> {
  let b = r.bytes().await.map_err(|e| e.to_string())?;
  serde_json::from_slice(&b).map_err(|e| e.to_string())
}

// a jwt: three base64url segments, bounded
pub fn valid_token(t: &str) -> bool {
  t.len() <= 4096 && t.split('.').count() == 3 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn open_browser(url: &str) -> Result<(), String> {
  std::process::Command::new("xdg-open")
    .arg(url)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .map(drop)
    .map_err(|e| format!("couldn't open a browser (xdg-open: {e})"))
}

pub struct Me {
  pub username: Box<str>,
  // the platform this heatsync account was founded on ("twitch", "kick", "youtube"; "" when unknown)
  pub platform: Box<str>,
  pub twitch: Option<Box<str>>,
  // twitch user id: our own heatsync inventory is the sender set under this id
  pub twitch_id: Option<u64>,
  // every name linked to the account (heatsync, twitch, kick, youtube): all of them ping you
  pub names: Vec<Box<str>>,
  // linked kick / youtube names: a kick-only or youtube-only login has no twitch at all
  pub kick: Option<Box<str>>,
  pub youtube: Option<Box<str>>,
  // twitch permission packs already granted (empty when unknown)
  pub packs: Vec<Box<str>>,
}

impl Me {
  pub fn has_pack(&self, pack: &str) -> bool {
    self.packs.iter().any(|p| &**p == pack)
  }
}

pub enum Check {
  Ok(Me, Option<String>),
  // the server says this token is dead: drop it
  Expired,
}

// who the token is; Some(new token) when the server slid the session forward
pub async fn check(http: &reqwest::Client, token: &str) -> Result<Check, String> {
  let r = http.get(format!("{ORIGIN}/api/auth/check")).bearer_auth(token).send().await.map_err(|e| e.to_string())?;
  if !r.status().is_success() {
    return Err(format!("http {}", r.status().as_u16()));
  }
  let v = body(r).await?;
  if v.get("authenticated").and_then(Value::as_bool) != Some(true) {
    return Ok(Check::Expired);
  }
  let u = v.get("user").ok_or("no user")?;
  let s = |k: &str| u.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(|x| x.chars().take(64).collect());
  let twitch_id = match u.get("twitch_id") {
    Some(Value::Number(n)) => n.as_u64(),
    Some(Value::String(x)) => x.parse().ok(),
    _ => None,
  };
  let names: Vec<Box<str>> =
    ["username", "twitch_username", "kick_username", "youtube_username"].into_iter().filter_map(s).collect();
  // a kick-only or youtube-only account has no twitch fields (null / missing): every one is optional
  let packs = u
    .get("twitch_scope_packs")
    .and_then(Value::as_array)
    .map(|a| a.iter().filter_map(Value::as_str).take(32).map(|p| p.chars().take(32).collect()).collect())
    .unwrap_or_default();
  let me = Me {
    platform: s("platform").unwrap_or_default(),
    username: s("username").or_else(|| names.first().cloned()).unwrap_or_else(|| "?".into()),
    twitch: s("twitch_username"),
    twitch_id,
    names,
    kick: s("kick_username"),
    youtube: s("youtube_username"),
    packs,
  };
  Ok(Check::Ok(me, v.get("token").and_then(Value::as_str).filter(|t| valid_token(t)).map(Into::into)))
}

// the user's twitch irc credential, brokered by heatsync (refreshed server-side)
// Ok(None) = this account has no twitch linked (a 404 from heatsync), not a failure
pub async fn chat_token(http: &reqwest::Client, token: &str) -> Result<Option<(String, String)>, String> {
  let r = http
    .get(format!("{ORIGIN}/api/auth/twitch/chat-token"))
    .bearer_auth(token)
    .send()
    .await
    .map_err(|e| e.to_string())?;
  let status = r.status().as_u16();
  if status == 404 {
    return Ok(None);
  }
  parse_chat_token((200..300).contains(&status), &body(r).await?)
}

fn parse_chat_token(ok: bool, v: &Value) -> Result<Option<(String, String)>, String> {
  let t =
    v.get("token").and_then(Value::as_str).filter(|t| t.len() <= 200 && t.bytes().all(|b| b.is_ascii_alphanumeric()));
  // heatsync may hand back a mixed-case handle: irc wants lowercase
  let u = v.get("username").and_then(Value::as_str).map(str::to_lowercase).filter(|u| super::irc::valid_channel(u));
  match (ok, t, u) {
    (true, Some(t), Some(u)) => Ok(Some((t.to_string(), u))),
    _ => Err(v.get("error").and_then(Value::as_str).unwrap_or("no twitch chat token").chars().take(100).collect()),
  }
}

pub async fn logout(http: &reqwest::Client, token: &str) {
  let _ = http.post(format!("{ORIGIN}/api/auth/logout")).bearer_auth(token).send().await;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn chat_token_parses_for_any_sign_in() {
    let v = json!({ "token": "abc123", "username": "Mellen" });
    assert_eq!(parse_chat_token(true, &v), Ok(Some(("abc123".into(), "mellen".into()))));
    assert!(parse_chat_token(true, &json!({ "token": "bad token!", "username": "x" })).is_err());
    assert_eq!(
      parse_chat_token(false, &json!({ "error": "twitch token expired" })),
      Err("twitch token expired".into())
    );
  }

  #[test]
  fn base64url() {
    assert_eq!(b64url(b""), "");
    assert_eq!(b64url(b"f"), "Zg");
    assert_eq!(b64url(b"fo"), "Zm8");
    assert_eq!(b64url(b"foo"), "Zm9v");
    assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
  }

  #[test]
  fn pkce_shape() {
    let p = pkce().unwrap();
    assert_eq!(p.verifier.len(), 43);
    assert_eq!(p.challenge.len(), 43);
    assert!(p.state.len() >= 16);
    // rfc 7636 appendix b
    let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert_eq!(b64url(digest(&SHA256, v.as_bytes()).as_ref()), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
  }

  #[test]
  fn loopback_request() {
    let req = "GET /cb?code=ab12&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    assert_eq!(callback(req), Some(("ab12".into(), "xyz".into())));
    assert_eq!(callback("GET /favicon.ico HTTP/1.1\r\n"), None);
    assert_eq!(callback("POST /cb?code=a&state=b HTTP/1.1\r\n"), None);
    assert_eq!(callback("GET /cb?code=a HTTP/1.1\r\n"), None);
  }

  #[test]
  fn login_urls() {
    let p = Pkce { verifier: "v".into(), challenge: "c".into(), state: "s".into() };
    assert_eq!(
      login_url(Provider::Twitch, 5, &p, "whispers", None),
      "https://heatsync.org/api/auth/login?native=5&cc=c&ns=s&scopes=whispers"
    );
    assert_eq!(
      login_url(Provider::Kick, 5, &p, "base", None),
      "https://heatsync.org/api/auth/kick/login?native=5&cc=c&ns=s"
    );
    assert_eq!(
      login_url(Provider::Youtube, 5, &p, "whispers", None),
      "https://heatsync.org/api/auth/youtube/login?native=5&cc=c&ns=s"
    );
  }

  #[test]
  fn link_url_carries_the_ticket() {
    let p = Pkce { verifier: "v".into(), challenge: "c".into(), state: "s".into() };
    assert_eq!(
      login_url(Provider::Kick, 5, &p, "base", Some("ab12")),
      "https://heatsync.org/api/auth/kick/login?native=5&cc=c&ns=s&nl=ab12"
    );
    assert!(login_url(Provider::Twitch, 5, &p, "base", Some("ab12")).ends_with("&nl=ab12&scopes=base"));
  }

  #[test]
  fn tokens() {
    assert!(valid_token("aaa.bbb.ccc"));
    assert!(!valid_token("aaa.bbb"));
    assert!(!valid_token("a b.c.d"));
  }
}
