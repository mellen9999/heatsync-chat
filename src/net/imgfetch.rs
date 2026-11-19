// picture fetches: the disk cache first, then the network (allowlisted hosts only, a few at a time), and one
// decoder thread off the network loop that keeps the disk cache honest
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

use super::{NetEvent, Out, USER_AGENT, http};
use crate::i18n::t;
use crate::images::{ImgId, cache, decode, url};

const DISK_CACHE_MAX: u64 = 256 << 20;
const MAX_IMAGE_BYTES: usize = 8 << 20;

pub(super) struct DecodeJob {
  pub(super) id: ImgId,
  pub(super) url: Box<str>,
  pub(super) h: u32,
  pub(super) bytes: Vec<u8>,
  // fresh from the network: persist once it decodes
  pub(super) store: bool,
}

// decode one job, and keep the disk cache honest: bytes are stored only once they decode, and a cached
// file that no longer decodes is dropped, so a retry fetches again instead of reading the same junk
fn run_job(j: &DecodeJob) -> Result<decode::Decoded, String> {
  // a decoder bug on one hostile file must not end decoding for the session
  let res =
    std::panic::catch_unwind(|| decode::decode(&j.bytes, j.h)).unwrap_or_else(|_| Err("decoder crashed".into()));
  match (&res, j.store) {
    (Ok(_), true) => cache::write(&j.url, &j.bytes),
    (Err(_), false) => cache::remove(&j.url),
    _ => {}
  }
  res
}

// decoding is cpu work; keep it off the network loop
pub(super) fn spawn_decoder(out: Out) -> Option<std::sync::mpsc::Sender<DecodeJob>> {
  let (tx, rx) = std::sync::mpsc::channel::<DecodeJob>();
  let spawned = std::thread::Builder::new().name("decode".into()).spawn(move || {
    cache::prune(DISK_CACHE_MAX);
    for j in rx {
      let res = run_job(&j);
      if !out.send(NetEvent::Image(j.id, j.h, res)) {
        return;
      }
    }
  });
  spawned.ok().map(|_| tx)
}

// the decoder thread is gone: the picture fails (so it isn't "loading" forever) and, once, the app says so
static DECODER_GONE_SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(super) fn hand_to_decoder(dec: &std::sync::mpsc::Sender<DecodeJob>, job: DecodeJob, out: &Out) {
  let (id, h) = (job.id, job.h);
  if dec.send(job).is_err() {
    out.send(NetEvent::Image(id, h, Err("decoder stopped".into())));
    if !DECODER_GONE_SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
      out.status(t("net1.decoder_gone"));
    }
  }
}

pub(super) async fn image(
  http: reqwest::Client,
  gate: Arc<Semaphore>,
  dec: std::sync::mpsc::Sender<DecodeJob>,
  id: ImgId,
  url: Box<str>,
  h: u32,
  out: Out,
) {
  if !url::allowed(&url) {
    out.send(NetEvent::Image(id, h, Err("host not allowed".into())));
    return;
  }
  // a disk read must not stall the one network thread
  let file = cache::path(&url);
  if let Ok(Some(bytes)) = tokio::task::spawn_blocking(move || file.and_then(|f| cache::read_file(&f))).await {
    return hand_to_decoder(&dec, DecodeJob { id, url, h, bytes, store: false }, &out);
  }
  let Ok(_permit) = gate.acquire().await else { return };
  match http::capped(http.get(&*url).send().await, MAX_IMAGE_BYTES).await {
    Ok(bytes) => hand_to_decoder(&dec, DecodeJob { id, url, h, bytes, store: true }, &out),
    Err(e) => {
      out.send(NetEvent::Image(id, h, Err(e)));
    }
  }
}

// image fetches follow redirects only between allowlisted hosts (`/api/img` answers with a 302)
pub(super) fn image_client() -> Option<reqwest::Client> {
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

#[cfg(test)]
mod image_io {
  use super::*;

  const URL: &str = "https://cdn.7tv.app/emote/t1/1x.webp";

  fn job(store: bool, bytes: &[u8]) -> DecodeJob {
    DecodeJob { id: 0, url: URL.into(), h: 28, bytes: bytes.to_vec(), store }
  }

  fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
  }

  fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
  }

  #[test]
  fn a_reply_that_is_not_a_picture_is_never_cached() {
    cache::remove(URL);
    assert!(run_job(&job(true, b"<html>502 bad gateway</html>")).is_err());
    assert!(cache::read(URL).is_none(), "junk must not be kept for the next retry");
  }

  #[test]
  fn a_cached_file_that_no_longer_decodes_is_dropped() {
    cache::write(URL, b"<html>oops</html>");
    assert!(cache::read(URL).is_some());
    assert!(run_job(&job(false, b"<html>oops</html>")).is_err());
    assert!(cache::read(URL).is_none());
  }

  #[test]
  fn a_good_picture_is_cached_once_it_decodes() {
    cache::remove(URL);
    let mut png = Vec::new();
    {
      let mut e = png::Encoder::new(&mut png, 2, 2);
      e.set_color(png::ColorType::Rgba);
      e.set_depth(png::BitDepth::Eight);
      e.write_header().unwrap().write_image_data(&[9u8; 16]).unwrap();
    }
    assert!(run_job(&job(true, &png)).is_ok());
    assert_eq!(cache::read(URL).as_deref(), Some(&png[..]));
    // and a good cached file stays
    assert!(run_job(&job(false, &png)).is_ok());
    assert!(cache::read(URL).is_some());
  }

  #[test]
  fn a_dead_decoder_fails_the_picture_and_says_so_once() {
    cache::write(URL, b"anything");
    let (dec, rx) = std::sync::mpsc::channel::<DecodeJob>();
    drop(rx);
    let (tx, ev) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    DECODER_GONE_SAID.store(false, std::sync::atomic::Ordering::Relaxed);
    for id in [3, 4] {
      run(image(client(), Arc::new(Semaphore::new(1)), dec.clone(), id, URL.into(), 28, out.clone()));
    }
    let evs: Vec<NetEvent> = ev.try_iter().collect();
    let failed = evs.iter().filter(|e| matches!(e, NetEvent::Image(_, _, Err(_)))).count();
    let said: Vec<&String> =
      evs.iter().filter_map(|e| if let NetEvent::Status(s) = e { Some(s) } else { None }).collect();
    assert_eq!(failed, 2, "each picture fails instead of loading forever");
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(*said[0], said[0].to_lowercase());
  }

  // a cache file that blocks on read (a fifo) may hold up only its own picture, not the net thread
  #[cfg(unix)]
  #[test]
  fn the_disk_read_does_not_stall_the_network_thread() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let path = cache::path(URL).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(std::process::Command::new("mkfifo").arg(&path).status().unwrap().success());
    let p = path.clone();
    let releaser = std::thread::spawn(move || {
      std::thread::sleep(Duration::from_millis(600));
      std::fs::write(p, b"anything").unwrap();
    });
    let (dec, _rx) = std::sync::mpsc::channel::<DecodeJob>();
    let (tx, _ev) = std::sync::mpsc::channel();
    let image_done = Arc::new(AtomicBool::new(false));
    let ticker_saw_done = run({
      let image_done = image_done.clone();
      async move {
        let seen = image_done.clone();
        let ticker = tokio::spawn(async move {
          tokio::time::sleep(Duration::from_millis(50)).await;
          seen.load(Ordering::SeqCst)
        });
        image(client(), Arc::new(Semaphore::new(1)), dec, 1, URL.into(), 28, Out::Pipe(tx)).await;
        image_done.store(true, Ordering::SeqCst);
        ticker.await.unwrap()
      }
    });
    releaser.join().unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(!ticker_saw_done, "the timer fired while the file read was still waiting");
  }
}
