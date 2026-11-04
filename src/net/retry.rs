// a fetch that fails because we're offline or a server is down is tried again with backoff, not given
// up on until a restart. every status line lands in the chat on screen, so an outage says it once
// (and again when it's over), not once per channel per provider
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::sleep;

use super::{Out, backoff, secs};
use crate::util::time::now_ms;

// at most one "can't load" line per this long, across every retrying fetch
const QUIET_MS: i64 = 30_000;
static LAST_SAID: AtomicI64 = AtomicI64::new(0);

// which twitch tabs are open, as the net side sees it: a retry for a tab that closed stops, and one for
// a tab that was closed and opened again hands over to the newer ask
#[derive(Clone, Default)]
pub struct Open(Arc<Mutex<Tabs>>);

#[derive(Default)]
struct Tabs {
  // counts every opening ever, so a reopened tab never looks like the one it replaced
  born: u64,
  open: HashMap<Box<str>, u64>,
}

impl Open {
  fn tabs(&self) -> std::sync::MutexGuard<'_, Tabs> {
    self.0.lock().unwrap_or_else(PoisonError::into_inner)
  }

  pub fn join(&self, ch: &str) -> Watch {
    let mut t = self.tabs();
    t.born += 1;
    let born = t.born;
    t.open.insert(ch.into(), born);
    Watch { open: self.clone(), ch: ch.into(), born }
  }

  pub fn part(&self, ch: &str) {
    self.tabs().open.remove(ch);
  }

  // the tab as it's open right now (None = not open)
  pub fn watch(&self, ch: &str) -> Option<Watch> {
    let born = *self.tabs().open.get(ch)?;
    Some(Watch { open: self.clone(), ch: ch.into(), born })
  }
}

#[derive(Clone)]
pub struct Watch {
  open: Open,
  ch: Box<str>,
  born: u64,
}

impl Watch {
  pub fn alive(&self) -> bool {
    self.open.tabs().open.get(&self.ch) == Some(&self.born)
  }
}

// `f` until it gives Ok, waiting longer after each failure (`backoff`), for as long as `alive`. `what` is
// the plain name of the thing ("7tv emotes"); the technical reason goes to stderr. None = gave up
pub async fn until<T, Fut>(out: &Out, what: &str, alive: impl Fn() -> bool, f: impl FnMut() -> Fut) -> Option<T>
where
  Fut: Future<Output = Result<T, String>>,
{
  until_with(out, &LAST_SAID, what, u32::MAX, alive, backoff, f).await
}

// the same, but gives up (and says so) after `max` failures
pub async fn until_n<T, Fut>(
  out: &Out,
  what: &str,
  max: u32,
  alive: impl Fn() -> bool,
  f: impl FnMut() -> Fut,
) -> Option<T>
where
  Fut: Future<Output = Result<T, String>>,
{
  until_with(out, &LAST_SAID, what, max, alive, backoff, f).await
}

async fn until_with<T, Fut>(
  out: &Out,
  last_said: &AtomicI64,
  what: &str,
  max: u32,
  alive: impl Fn() -> bool,
  wait_for: impl Fn(u32) -> Duration,
  mut f: impl FnMut() -> Fut,
) -> Option<T>
where
  Fut: Future<Output = Result<T, String>>,
{
  let mut tries = 0u32;
  let mut told = false;
  while alive() {
    match f().await {
      Ok(v) => {
        if told {
          out.status(format!("{what} loaded"));
        }
        return Some(v);
      }
      Err(e) => {
        tries += 1;
        eprintln!("{what}: {e}");
        if tries >= max {
          out.status(format!("couldn't load {what}"));
          return None;
        }
        let wait = wait_for(tries);
        let (now, last) = (now_ms(), last_said.load(Ordering::Relaxed));
        if tries == 1
          && now - last >= QUIET_MS
          && last_said.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
        {
          out.status(format!("can't load {what}, trying again in {}s", secs(wait)));
          told = true;
        }
        sleep(wait).await;
      }
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::NetEvent;

  fn said(rx: &std::sync::mpsc::Receiver<NetEvent>) -> Vec<String> {
    rx.try_iter().filter_map(|e| if let NetEvent::Status(s) = e { Some(s) } else { None }).collect()
  }

  #[tokio::test]
  async fn a_failing_fetch_is_tried_again_until_it_works_and_says_so_once() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let calls = std::cell::Cell::new(0);
    let quiet = AtomicI64::new(0);
    let got = until_with(
      &out,
      &quiet,
      "7tv emotes",
      u32::MAX,
      || true,
      |_| Duration::from_millis(1),
      || {
        calls.set(calls.get() + 1);
        let n = calls.get();
        async move { if n < 4 { Err("http 503".into()) } else { Ok(n) } }
      },
    )
    .await;
    assert_eq!(got, Some(4), "three failures, then it lands");
    let said = said(&rx);
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said[0].starts_with("can't load 7tv emotes, trying again in "), "{}", said[0]);
    assert_eq!(said[1], "7tv emotes loaded");
  }

  #[tokio::test]
  async fn an_outage_is_one_line_not_one_per_fetch() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let quiet = AtomicI64::new(0);
    for what in ["a", "b", "c"] {
      let tries = std::cell::Cell::new(0);
      until_with(
        &out,
        &quiet,
        what,
        u32::MAX,
        || true,
        |_| Duration::from_millis(1),
        || {
          tries.set(tries.get() + 1);
          let n = tries.get();
          async move { if n < 2 { Err("down".into()) } else { Ok(()) } }
        },
      )
      .await;
    }
    let said = said(&rx);
    assert_eq!(said.iter().filter(|s| s.starts_with("can't load")).count(), 1, "{said:?}");
    assert_eq!(said.iter().filter(|s| s.ends_with("loaded")).count(), 1, "only the one that was told: {said:?}");
  }

  #[tokio::test]
  async fn a_bounded_fetch_gives_up_and_says_so() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let calls = std::cell::Cell::new(0);
    let quiet = AtomicI64::new(0);
    let got: Option<()> = until_with(
      &out,
      &quiet,
      "kick history",
      3,
      || true,
      |_| Duration::from_millis(1),
      || {
        calls.set(calls.get() + 1);
        async { Err("http 500".into()) }
      },
    )
    .await;
    assert_eq!(got, None);
    assert_eq!(calls.get(), 3);
    assert_eq!(said(&rx).last().map(String::as_str), Some("couldn't load kick history"));
  }

  #[tokio::test]
  async fn a_closed_tab_stops_its_retries() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let open = Open::default();
    let w = open.join("xqc");
    let calls = std::cell::Cell::new(0);
    let quiet = AtomicI64::new(0);
    let got: Option<()> = until_with(
      &out,
      &quiet,
      "xqc's emotes",
      u32::MAX,
      || w.alive(),
      |_| Duration::from_millis(1),
      || {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
          open.part("xqc");
        }
        async { Err("down".into()) }
      },
    )
    .await;
    assert_eq!(got, None);
    assert_eq!(calls.get(), 2, "no third try for a tab that's gone");
  }

  #[test]
  fn reopening_a_tab_hands_over_to_the_newer_ask() {
    let open = Open::default();
    let old = open.join("xqc");
    assert!(old.alive());
    open.part("xqc");
    let new = open.join("xqc");
    assert!(!old.alive(), "the first ask is over once the tab was closed and opened again");
    assert!(new.alive());
    assert!(open.watch("nobody").is_none());
    assert!(open.watch("xqc").is_some_and(|w| w.alive()));
  }
}
