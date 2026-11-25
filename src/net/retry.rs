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
  until_with(out, &LAST_SAID, what, u32::MAX, alive, backoff, || async {}, f).await
}

// the same, with a stand-in: `stand_in` runs once, when the first try fails, before the wait. it is where
// a kept copy of the thing is put on screen while its source is asked again
pub async fn until_or<T, Fut, S>(
  out: &Out,
  what: &str,
  alive: impl Fn() -> bool,
  stand_in: impl FnOnce() -> S,
  f: impl FnMut() -> Fut,
) -> Option<T>
where
  Fut: Future<Output = Result<T, String>>,
  S: Future<Output = ()>,
{
  until_with(out, &LAST_SAID, what, u32::MAX, alive, backoff, stand_in, f).await
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
  until_with(out, &LAST_SAID, what, max, alive, backoff, || async {}, f).await
}

// a failure that will come back the same: the server said no (4xx, but not "slow down" or "timed out"), a
// redirect the client's policy refused to follow (a 3xx it was left holding: asking again is bounced the same
// way), or what it sent is past our size cap. fetch errors read "http <code>" / "too large" (net/http.rs)
fn permanent(e: &str) -> bool {
  e == "too large"
    || e
      .strip_prefix("http ")
      .and_then(|c| c.parse::<u16>().ok())
      .is_some_and(|c| (300..500).contains(&c) && !matches!(c, 408 | 425 | 429))
}

#[allow(clippy::too_many_arguments)]
async fn until_with<T, Fut, S>(
  out: &Out,
  last_said: &AtomicI64,
  what: &str,
  max: u32,
  alive: impl Fn() -> bool,
  wait_for: impl Fn(u32) -> Duration,
  stand_in: impl FnOnce() -> S,
  mut f: impl FnMut() -> Fut,
) -> Option<T>
where
  Fut: Future<Output = Result<T, String>>,
  S: Future<Output = ()>,
{
  let mut tries = 0u32;
  let mut told = false;
  let mut stand_in = Some(stand_in);
  while alive() {
    match f().await {
      Ok(v) => {
        if told {
          out.status(crate::i18n::f("net2.retry.loaded", &[("what", what)]));
        }
        return Some(v);
      }
      Err(e) => {
        tries += 1;
        eprintln!("{what}: {e}");
        // a source that said no for good leaves its stand-in up too
        if let Some(s) = stand_in.take() {
          s().await;
        }
        // asking again can't change a "no" or a body that is too big: stop instead of every minute forever
        if tries >= max || permanent(&e) {
          out.status(crate::i18n::f("net2.retry.gave_up", &[("what", what)]));
          return None;
        }
        let wait = wait_for(tries);
        let (now, last) = (now_ms(), last_said.load(Ordering::Relaxed));
        if tries == 1
          && now - last >= QUIET_MS
          && last_said.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
        {
          out.status(crate::i18n::f("net2.retry.again", &[("what", what), ("secs", &secs(wait).to_string())]));
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
      || async {},
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
        || async {},
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
      || async {},
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

  // a "no" or a body too big is the same next time: one try, a plain line, no minute-by-minute re-download
  #[tokio::test]
  async fn a_permanent_failure_is_not_retried_but_a_busy_server_is() {
    assert!(permanent("http 403") && permanent("http 410") && permanent("too large"));
    assert!(permanent("http 301") && permanent("http 302") && permanent("http 308"), "a refused redirect");
    assert!(!permanent("http 429") && !permanent("http 408") && !permanent("http 503") && !permanent("down"));
    for (err, calls) in [("http 403", 1), ("http 302", 1), ("too large", 1), ("http 503", 3)] {
      let (tx, rx) = std::sync::mpsc::channel();
      let out = Out::Pipe(tx);
      let n = std::cell::Cell::new(0);
      let quiet = AtomicI64::new(0);
      let got: Option<()> = until_with(
        &out,
        &quiet,
        "x",
        3,
        || true,
        |_| Duration::from_millis(1),
        || async {},
        || {
          n.set(n.get() + 1);
          async move { Err(err.into()) }
        },
      )
      .await;
      assert_eq!((got, n.get()), (None, calls), "{err}");
      assert_eq!(said(&rx).last().map(String::as_str), Some("couldn't load x"), "{err}");
    }
    // forever-retrying fetches stop on a permanent one as well
    let (tx, _rx) = std::sync::mpsc::channel();
    let n = std::cell::Cell::new(0);
    let quiet = AtomicI64::new(0);
    let got: Option<()> = until_with(
      &Out::Pipe(tx),
      &quiet,
      "y",
      u32::MAX,
      || true,
      |_| Duration::from_millis(1),
      || async {},
      || {
        n.set(n.get() + 1);
        async { Err("http 404".into()) }
      },
    )
    .await;
    assert_eq!((got, n.get()), (None, 1));
  }

  // a source that is down: its kept copy goes up once, at the first failure, and the source's own answer
  // still arrives after it. a source that answers first try never reads the copy
  #[tokio::test]
  async fn a_stand_in_goes_up_once_when_the_first_try_fails_and_the_answer_still_comes() {
    let (tx, _rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    let log = std::cell::RefCell::new(Vec::new());
    let quiet = AtomicI64::new(0);
    let n = std::cell::Cell::new(0);
    let got = until_with(
      &out,
      &quiet,
      "x",
      u32::MAX,
      || true,
      |_| Duration::from_millis(1),
      || async { log.borrow_mut().push("copy") },
      || {
        n.set(n.get() + 1);
        let i = n.get();
        log.borrow_mut().push(if i < 3 { "down" } else { "answer" });
        async move { if i < 3 { Err("http 503".to_string()) } else { Ok(i) } }
      },
    )
    .await;
    assert_eq!(got, Some(3));
    assert_eq!(*log.borrow(), ["down", "copy", "down", "answer"], "once, after the first failure, before the wait");

    log.borrow_mut().clear();
    let got = until_with(
      &out,
      &quiet,
      "x",
      u32::MAX,
      || true,
      |_| Duration::from_millis(1),
      || async { log.borrow_mut().push("copy") },
      || async { Ok(1) },
    )
    .await;
    assert_eq!((got, log.borrow().len()), (Some(1), 0), "an answer on the first try reads no copy");

    // a source that says no for good (it is not asked again) leaves the copy up
    let got: Option<()> = until_with(
      &out,
      &quiet,
      "x",
      u32::MAX,
      || true,
      |_| Duration::from_millis(1),
      || async { log.borrow_mut().push("copy") },
      || async { Err("http 403".to_string()) },
    )
    .await;
    assert_eq!((got, &**log.borrow()), (None, &["copy"][..]));
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
      || async {},
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
