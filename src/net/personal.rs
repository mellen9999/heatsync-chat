// reads 7tv personal emote sets whole (`GET /v3/emote-sets/<id>`) when the app asks. the one thing sent is a
// set's id, never a person's, and never ours (docs/PRIVACY.md). every answer is hostile: capped, checked
// against the set asked for, emotes bounded and their urls built by us (`emotes::entitle`)
use std::sync::Arc;

use tokio::sync::Semaphore;

use super::seventv::API;
use super::{NetEvent, Out, http};
use crate::emotes::live::valid_id;
use crate::emotes::personal::Ev;
use crate::emotes::{Set, entitle};

// a personal set is a handful of emotes at a few KB each; this is far past any real one
const MAX_BODY: usize = 1 << 20;
// sets read per ask, and reads in flight at once across all asks
const MAX_ASK: usize = 32;
pub const IN_FLIGHT: usize = 2;

// one set: the emotes, or an empty set when 7tv has none by that id (a deleted or never-filled set).
// None = no answer worth keeping (network, a server error, a body that is not that set)
async fn one(http: &reqwest::Client, base: &str, set: &str) -> Option<Set> {
  let r = http.get(format!("{base}/v3/emote-sets/{set}")).send().await;
  if r.as_ref().is_ok_and(|r| r.status() == reqwest::StatusCode::NOT_FOUND) {
    return Some(Set::new());
  }
  let v = http::capped_json(r, MAX_BODY).await.ok()?;
  entitle::set_emotes(&v, set)
}

pub async fn read(http: reqwest::Client, gate: Arc<Semaphore>, sets: Vec<Box<str>>, out: Out) {
  read_from(&http, API, &gate, sets, &out).await;
}

async fn read_from(http: &reqwest::Client, base: &str, gate: &Semaphore, sets: Vec<Box<str>>, out: &Out) {
  for set in sets.into_iter().take(MAX_ASK).filter(|s| valid_id(s)) {
    let Ok(_slot) = gate.acquire().await else { return };
    if let Some(emotes) = one(http, base, &set).await
      && !out.send(NetEvent::Personal(Ev::Set(set, emotes)))
    {
      return;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokio::io::{AsyncReadExt, AsyncWriteExt};

  // a local server: each request gets the next canned reply; the paths asked are kept
  async fn serve(replies: Vec<String>) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let asked: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let seen = asked.clone();
    tokio::spawn(async move {
      for r in replies {
        let Ok((mut s, _)) = l.accept().await else { return };
        let mut buf = [0u8; 2048];
        let n = s.read(&mut buf).await.unwrap_or(0);
        let line = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_owned();
        seen.lock().unwrap().push(line);
        let _ = s.write_all(r.as_bytes()).await;
      }
    });
    (base, asked)
  }

  fn ok(body: &str) -> String {
    format!("HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len())
  }

  fn code(c: &str) -> String {
    format!("HTTP/1.1 {c}\r\nconnection: close\r\ncontent-length: 0\r\n\r\n")
  }

  fn row(id: &str, name: &str) -> String {
    format!(r#"{{"id":"{id}","name":"{name}","flags":0,"data":{{"flags":0,"host":{{"url":"//x/e","files":[]}}}}}}"#)
  }

  async fn run(replies: Vec<String>, sets: &[&str]) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    let (base, asked) = serve(replies).await;
    let (tx, rx) = std::sync::mpsc::channel();
    let sets = sets.iter().map(|s| Box::<str>::from(*s)).collect();
    read_from(&reqwest::Client::new(), &base, &Semaphore::new(IN_FLIGHT), sets, &Out::Pipe(tx)).await;
    let got = rx
      .try_iter()
      .filter_map(|e| match e {
        NetEvent::Personal(Ev::Set(id, set)) => {
          let mut names: Vec<String> = set.keys().map(|k| k.to_string()).collect();
          names.sort_unstable();
          Some((id.to_string(), names))
        }
        _ => None,
      })
      .collect();
    let paths = asked.lock().unwrap().iter().map(|l| l.split(' ').nth(1).unwrap_or("").to_owned()).collect();
    (got, paths)
  }

  #[tokio::test]
  async fn a_set_is_read_by_its_id_and_nothing_else_is_asked() {
    let body = format!(
      r#"{{"id":"S1","name":"Personal Emotes","flags":4,"emotes":[{},{}]}}"#,
      row("A1", "Kek"),
      row("B2", "Pog")
    );
    let (got, paths) = run(vec![ok(&body)], &["S1"]).await;
    assert_eq!(got, [("S1".to_owned(), vec!["Kek".to_owned(), "Pog".to_owned()])]);
    assert_eq!(paths, ["/v3/emote-sets/S1"]);
  }

  #[tokio::test]
  async fn hostile_answers_leave_nothing_behind() {
    let wrong = format!(r#"{{"id":"OTHER","emotes":[{}]}}"#, row("A1", "Kek"));
    let replies = vec![ok(&wrong), ok("not json"), code("503 Service Unavailable"), ok("[1,2]"), code("404 Not Found")];
    let (got, paths) = run(replies, &["S1", "S2", "S3", "S4", "S5"]).await;
    // only the 404 is an answer (an empty set); a body for another set, junk and a server error are not
    assert_eq!(got, [("S5".to_owned(), Vec::<String>::new())]);
    assert_eq!(paths.len(), 5);
  }

  #[tokio::test]
  async fn a_huge_answer_is_abandoned_and_a_bad_id_is_never_asked() {
    let huge = ok(&"x".repeat(MAX_BODY + 10));
    let (got, paths) = run(vec![huge], &["../etc", "has space", "", "S1"]).await;
    assert!(got.is_empty());
    assert_eq!(paths, ["/v3/emote-sets/S1"], "ids that are not ids are dropped before any request");
  }

  #[tokio::test]
  async fn one_ask_reads_a_bounded_number_of_sets() {
    let replies: Vec<String> = (0..MAX_ASK + 10).map(|_| code("404 Not Found")).collect();
    let ids: Vec<String> = (0..MAX_ASK + 10).map(|i| format!("S{i}")).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (got, paths) = run(replies, &refs).await;
    assert_eq!((got.len(), paths.len()), (MAX_ASK, MAX_ASK));
  }
}
