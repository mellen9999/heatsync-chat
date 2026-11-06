// work that must keep its order per key but never hold up anything else: jobs with the same key run one
// after another on their own task, jobs with different keys run side by side, and the caller never waits.
// a kick or youtube send can take heatsync's whole 12s; the login loop must not sit through that
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

#[derive(Default)]
pub struct Lanes(HashMap<Box<str>, UnboundedSender<Job>>);

impl Lanes {
  // queue `job` behind the earlier ones with this key
  pub fn run(&mut self, key: &str, job: impl Future<Output = ()> + Send + 'static) {
    let mut job: Job = Box::pin(job);
    // a lane whose task died (a job panicked) is replaced, and the job goes to the new one
    for _ in 0..2 {
      let lane = self.0.entry(key.into()).or_insert_with(|| {
        let (tx, mut rx) = unbounded_channel::<Job>();
        tokio::spawn(async move {
          while let Some(j) = rx.recv().await {
            j.await;
          }
        });
        tx
      });
      match lane.send(job) {
        Ok(()) => return,
        Err(back) => {
          job = back.0;
          self.0.remove(key);
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::{Arc, Mutex};
  use std::time::Duration;
  use tokio::sync::oneshot;

  // a slow job in one lane holds up its own lane only, in order; the caller returns at once
  #[tokio::test]
  async fn a_lane_keeps_its_order_and_never_blocks_the_caller_or_other_lanes() {
    let mut lanes = Lanes::default();
    let log = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let (slow_done, slow_wait) = oneshot::channel::<()>();
    let l = log.clone();
    lanes.run("kick/a", async move {
      let _ = slow_wait.await;
      l.lock().unwrap().push("a1");
    });
    let l = log.clone();
    lanes.run("kick/a", async move { l.lock().unwrap().push("a2") });
    let (b_done, b_wait) = oneshot::channel::<()>();
    let l = log.clone();
    lanes.run("kick/b", async move {
      l.lock().unwrap().push("b1");
      let _ = b_done.send(());
    });
    // we got here with a1 still stuck: the caller was not held, and b ran past it
    tokio::time::timeout(Duration::from_secs(2), b_wait).await.unwrap().unwrap();
    assert_eq!(*log.lock().unwrap(), ["b1"]);
    let _ = slow_done.send(());
    for _ in 0..200 {
      if log.lock().unwrap().len() == 3 {
        break;
      }
      tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(*log.lock().unwrap(), ["b1", "a1", "a2"], "a2 stayed behind a1");
  }

  // a job that panics kills its lane's task; the next job still runs, on a fresh lane
  #[tokio::test]
  async fn a_panicked_lane_is_replaced() {
    let mut lanes = Lanes::default();
    lanes.run("k", async { panic!("boom") });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (tx, rx) = oneshot::channel();
    lanes.run("k", async move {
      let _ = tx.send(());
    });
    tokio::time::timeout(Duration::from_secs(2), rx).await.unwrap().unwrap();
  }
}
