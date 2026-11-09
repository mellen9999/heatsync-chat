// the one place a network body or socket frame gets its size cap. every server byte is hostile:
// nothing reads `.bytes()` / `.json()` / `.text()` unbounded, nothing opens a ws on tungstenite's 64MiB default
use crate::i18n::t;
use serde_json::Value;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

// a failed request for the screen: plain words, the technical reason goes to stderr
pub fn offline(what: &str, why: impl std::fmt::Display) -> String {
  eprintln!("{what}: {why}");
  crate::i18n::f("net2.http.offline", &[("what", what)])
}

// an http status as a short, plain sentence for the screen (when the server gave no words of its own)
pub fn plain_status(code: u16) -> String {
  match code {
    401 => t("net2.http.logged_out").into(),
    403 => t("net2.http.said_no").into(),
    429 => t("net2.http.slow_bit").into(),
    500.. => t("net2.http.trouble").into(),
    _ => t("net2.http.failed").into(),
  }
}

// what the server said, in our words. its text is hostile and often a code or a developer sentence
// ("Rate limit exceeded", "invalid_params"): the ones it sends often get our copy, the rest come out
// lowercase, one line, no trailing period, at most 100 chars
pub fn server_words(raw: &str) -> String {
  let clean: String = raw.chars().filter(|c| !c.is_control()).take(100).collect();
  let low = clean.trim().to_lowercase().replace('_', " ").replace('’', "'");
  let low = low.trim_end_matches('.').trim_end();
  let has = |ks: &[&str]| ks.iter().any(|k| low.contains(k));
  if low.is_empty() {
    String::new()
  } else if has(&["rate limit", "too many", "slow down"]) {
    t("net2.http.slow_minute").into()
  } else if has(&["unauthorized", "login required", "not authenticated", "invalid session", "no twitch token"]) {
    t("net2.http.logged_out").into()
  } else if has(&["not linked"]) {
    t("net2.http.link_first").into()
  } else if has(&["not a member", "not moderator", "not a moderator"]) {
    t("net2.http.not_mod").into()
  } else if has(&[
    "internal server error",
    "upstream error",
    "unavailable",
    "health check",
    "query failed",
    "search failed",
    "busy",
  ]) || low.starts_with("failed to")
  {
    t("net2.http.trouble").into()
  } else if low.starts_with("invalid") || low.starts_with("bad ") {
    t("net2.http.bad_input").into()
  } else if low.contains("not found") {
    t("net2.http.not_found").into()
  } else {
    low.to_owned()
  }
}

// the body whatever its status (callers that read an error body), abandoned as soon as it passes `max`
pub async fn read(mut r: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
  if r.content_length().is_some_and(|n| n > max as u64) {
    return Err("too large".into());
  }
  let mut out = Vec::new();
  while let Some(c) = r.chunk().await.map_err(|e| e.to_string())? {
    if out.len() + c.len() > max {
      return Err("too large".into());
    }
    out.extend_from_slice(&c);
  }
  Ok(out)
}

// `read` as json, any status
pub async fn read_json(r: reqwest::Response, max: usize) -> Result<Value, String> {
  serde_json::from_slice(&read(r, max).await?).map_err(|e| e.to_string())
}

// a successful body, abandoned as soon as it passes `max` (a hostile host can't make us buffer it all)
pub async fn capped(res: Result<reqwest::Response, reqwest::Error>, max: usize) -> Result<Vec<u8>, String> {
  let r = res.map_err(|e| e.to_string())?;
  if !r.status().is_success() {
    return Err(format!("http {}", r.status().as_u16()));
  }
  read(r, max).await
}

pub async fn capped_json(res: Result<reqwest::Response, reqwest::Error>, max: usize) -> Result<Value, String> {
  serde_json::from_slice(&capped(res, max).await?).map_err(|e| e.to_string())
}

// a socket that drops any message or frame past `max` instead of buffering it
pub fn ws_config(max: usize) -> WebSocketConfig {
  WebSocketConfig::default().max_message_size(Some(max)).max_frame_size(Some(max))
}

type Wss = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// opens a socket the way every http request goes out: capped frames and our user agent (docs/PRIVACY.md)
pub async fn ws_connect(
  url: &str,
  max: usize,
) -> Result<(Wss, tokio_tungstenite::tungstenite::handshake::client::Response), tokio_tungstenite::tungstenite::Error> {
  use tokio_tungstenite::tungstenite::client::IntoClientRequest;
  let mut req = url.into_client_request()?;
  req
    .headers_mut()
    .insert("user-agent", tokio_tungstenite::tungstenite::http::HeaderValue::from_static(super::USER_AGENT));
  tokio_tungstenite::connect_async_with_config(req, Some(ws_config(max)), false).await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn a_socket_says_who_it_is_like_every_request() {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", l.local_addr().unwrap());
    let seen = tokio::spawn(async move {
      let (mut s, _) = l.accept().await.unwrap();
      let mut buf = [0u8; 2048];
      let n = s.read(&mut buf).await.unwrap();
      String::from_utf8_lossy(&buf[..n]).to_lowercase()
    });
    // the server never finishes the handshake: only what the client sent matters
    let _ = ws_connect(&url, 1000).await;
    let head = seen.await.unwrap();
    assert!(head.contains(&format!("user-agent: {}", crate::net::USER_AGENT)), "{head}");
  }

  #[test]
  fn the_servers_words_come_out_plain() {
    let said = server_words;
    assert_eq!(said("Rate limit exceeded"), "slow down, try again in a minute");
    assert_eq!(said("rate_limit"), "slow down, try again in a minute");
    assert_eq!(said("Too many requests"), "slow down, try again in a minute");
    assert_eq!(said("Unauthorized"), "you're logged out, log in again (ctrl-l)");
    assert_eq!(said("platform account not linked"), "link that account on heatsync.org first");
    assert_eq!(said("internal_server_error"), "heatsync is having trouble, try again soon");
    assert_eq!(said("Failed to fetch streams"), "heatsync is having trouble, try again soon");
    assert_eq!(said("Invalid channel name"), "that didn't look right, check it and try again");
    assert_eq!(said("Set not found"), "couldn't find that");
    // anything else: lowercase, one line, no trailing period
    assert_eq!(said("You can't react to your own line."), "you can't react to your own line");
    assert_eq!(said("name held by your upload"), "name held by your upload");
    assert_eq!(said(""), "");
    assert_eq!(said(&"x\n".repeat(200)).chars().count(), 100);
  }
  use tokio::io::{AsyncReadExt, AsyncWriteExt};

  // a one-shot local server answering with `body`, chunked when `len` is None
  async fn serve(status: &str, body: Vec<u8>, chunked: bool) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", l.local_addr().unwrap());
    let status = status.to_string();
    tokio::spawn(async move {
      let (mut s, _) = l.accept().await.unwrap();
      let mut buf = [0u8; 1024];
      let _ = s.read(&mut buf).await;
      let mut head = format!("HTTP/1.1 {status}\r\nconnection: close\r\n");
      if chunked {
        head.push_str("transfer-encoding: chunked\r\n\r\n");
        let _ = s.write_all(head.as_bytes()).await;
        for c in body.chunks(100) {
          let _ = s.write_all(format!("{:x}\r\n", c.len()).as_bytes()).await;
          let _ = s.write_all(c).await;
          let _ = s.write_all(b"\r\n").await;
        }
        let _ = s.write_all(b"0\r\n\r\n").await;
      } else {
        head.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
        let _ = s.write_all(head.as_bytes()).await;
        let _ = s.write_all(&body).await;
      }
    });
    url
  }

  #[tokio::test]
  async fn capped_enforces_status_and_size() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let c = reqwest::Client::new();
    let u = serve("200 OK", vec![b'x'; 500], false).await;
    assert_eq!(capped(c.get(&u).send().await, 500).await.unwrap().len(), 500);
    let u = serve("200 OK", vec![b'x'; 500], false).await;
    assert_eq!(capped(c.get(&u).send().await, 499).await.unwrap_err(), "too large");
    // no content-length: aborted mid-stream
    let u = serve("200 OK", vec![b'x'; 500], true).await;
    assert_eq!(capped(c.get(&u).send().await, 300).await.unwrap_err(), "too large");
    let u = serve("404 Not Found", b"{}".to_vec(), false).await;
    assert_eq!(capped(c.get(&u).send().await, 10).await.unwrap_err(), "http 404");
    let u = serve("200 OK", br#"{"a":1}"#.to_vec(), false).await;
    assert_eq!(capped_json(c.get(&u).send().await, 100).await.unwrap()["a"], 1);
  }

  #[test]
  fn statuses_read_plain_for_the_screen() {
    for code in [0u16, 400, 401, 403, 404, 429, 500, 503, 599] {
      let s = plain_status(code);
      assert!(!s.starts_with("http"), "{code}: {s}");
      assert_eq!(s, s.to_lowercase(), "{code}: lowercase");
    }
    assert!(plain_status(503).contains("try again"));
    assert!(plain_status(401).contains("ctrl-l"));
    assert_eq!(offline("twitch", "connection refused (os error 111)"), "can't reach twitch");
  }

  #[test]
  fn ws_config_sets_both_limits() {
    let c = ws_config(1234);
    assert_eq!((c.max_message_size, c.max_frame_size), (Some(1234), Some(1234)));
  }
}
