// chat lines go out one after another, each after the emote adds it waits for. the server stamps a line
// with the sender's emotes as they stand when it fans it out, for every viewer and for good: a line that
// beats its own add/swap out the door carries the OLD emote of that name. so a line waits for them,
// bounded (a slow or failed add still sends the text), and a later line queues behind it, so it neither
// overtakes the line before it nor beats that same add. no add in flight = no wait
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use super::Cmd;

pub const WAIT: Duration = Duration::from_millis(1500);

// a `Cmd::Send`, and what it waits for (fires when the emote ops asked before it are done)
pub type Queued = (Option<oneshot::Receiver<()>>, Cmd);

pub async fn run(mut rx: mpsc::UnboundedReceiver<Queued>, mut send: impl FnMut(Cmd)) {
  while let Some((hold, c)) = rx.recv().await {
    if let Some(h) = hold {
      let _ = timeout(WAIT, h).await;
    }
    send(c);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::Platform;
  use std::time::Instant;

  fn line(text: &str) -> Cmd {
    Cmd::Send { platform: Platform::Twitch, chan: "forsen".into(), text: text.into(), reply: None }
  }

  fn sent(rx: &mut mpsc::UnboundedReceiver<String>) -> Vec<String> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
  }

  // a line waits for its add; the next line, with no add of its own, still goes after it
  #[tokio::test]
  async fn a_line_waits_for_its_add_and_the_next_waits_behind_it() {
    let (tx, rx) = mpsc::unbounded_channel();
    let (out_tx, mut out) = mpsc::unbounded_channel();
    tokio::spawn(run(rx, move |c| {
      if let Cmd::Send { text, .. } = c {
        let _ = out_tx.send(text);
      }
    }));
    let (done, hold) = oneshot::channel();
    tx.send((Some(hold), line("one"))).unwrap();
    tx.send((None, line("two"))).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(sent(&mut out).is_empty(), "the add is still in flight");
    done.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sent(&mut out), ["one", "two"]);
    // an add that fails (its signal dropped) or never answers still lets the line out
    let (dropped, hold) = oneshot::channel::<()>();
    drop(dropped);
    tx.send((Some(hold), line("three"))).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sent(&mut out), ["three"]);
    let (_stuck, hold) = oneshot::channel::<()>();
    let t = Instant::now();
    tx.send((Some(hold), line("four"))).unwrap();
    while sent(&mut out).is_empty() {
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!((WAIT..WAIT + Duration::from_millis(500)).contains(&t.elapsed()), "bounded");
  }
}
