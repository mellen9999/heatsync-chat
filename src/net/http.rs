// the one place a network body or socket frame gets its size cap. every server byte is hostile:
// nothing reads `.bytes()` / `.json()` / `.text()` unbounded, nothing opens a ws on tungstenite's 64MiB default
use serde_json::Value;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

// a failed request for the screen: plain words, the technical reason goes to stderr
pub fn offline(what: &str, why: impl std::fmt::Display) -> String {
  eprintln!("{what}: {why}");
  format!("can't reach {what}")
}

// an http status as a short, plain sentence for the screen (when the server gave no words of its own)
pub fn plain_status(code: u16) -> String {
  match code {
    401 => "you're logged out, log in again (ctrl-l)".into(),
    403 => "heatsync said no".into(),
    429 => "slow down, try again in a bit".into(),
    500.. => "heatsync is having trouble, try again soon".into(),
    _ => format!("that didn't work (heatsync said {code})"),
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

#[cfg(test)]
mod tests {
  use super::*;
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
