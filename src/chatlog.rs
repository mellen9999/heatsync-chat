// chat logs on disk: off until you turn on "save chat logs". every line of every open channel is
// appended to <data>/heatsync-chat/logs/<platform>/<channel>/<yyyy-mm-dd>.log, local only, never sent
// anywhere. whispers and dms are never logged.
//
// the ui only builds a line and hands it to a thread of its own (bounded queue, a full queue drops the
// line rather than stall a frame). the thread batches: nothing waits more than 2s, and it sleeps on the
// queue, so idle costs nothing. a write that fails is said once and logging stops until the switch is
// flipped off and on. the folders are owner-only (0700), the files too (0600)
mod line;

use crate::model::message::{Message, Platform};
use crate::util::owner;
use crate::util::time::now_ms;
use line::Entry;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, mpsc::TrySendError};
use std::time::{Duration, Instant};

// nothing waits longer than this to reach the disk
const BATCH: Duration = Duration::from_secs(2);
// a burst past this many bytes is written early, so memory stays small
const BATCH_BYTES: usize = 256 * 1024;
// lines waiting for the thread; a stalled disk drops the rest instead of growing
const QUEUE: usize = 8192;
// on the way out the app waits this long for the last batch
const EXIT_WAIT: Duration = Duration::from_secs(2);

enum Msg {
  Line(Entry),
  // write everything now, then answer
  Flush(SyncSender<()>),
}

pub struct ChatLog {
  tx: Option<SyncSender<Msg>>,
  // a line was dropped because the queue was full: said once, cleared by the writer when it catches up
  behind: Arc<AtomicBool>,
}

impl ChatLog {
  pub fn new(on: bool) -> Self {
    let mut c = ChatLog { tx: None, behind: Arc::default() };
    c.set(on);
    c
  }

  // the switch: on starts a writer, off lets it write what it holds and end
  pub fn set(&mut self, on: bool) {
    if !on {
      self.tx = None;
    } else if self.tx.is_none() {
      self.tx = start(crate::store::logs_dir(), crate::store::tell, Arc::clone(&self.behind));
    }
  }

  // one live line of a channel. off costs one branch
  pub fn log(&self, p: Platform, chan: &str, m: &Message) {
    let Some(tx) = &self.tx else { return };
    if let Some(e) = line::entry(p, chan, m, now_ms())
      && let Err(TrySendError::Full(_)) = tx.try_send(Msg::Line(e))
      && !self.behind.swap(true, Ordering::Relaxed)
    {
      crate::store::tell("chat logs fell behind: some lines weren't saved".into());
    }
  }

  // on the way out or into a restart: what is held is written now
  pub fn flush(&self) {
    let Some(tx) = &self.tx else { return };
    let (ack, done) = sync_channel(1);
    if tx.send(Msg::Flush(ack)).is_ok() {
      let _ = done.recv_timeout(EXIT_WAIT);
    }
  }
}

fn start(
  root: Option<PathBuf>,
  tell: impl Fn(String) + Send + 'static,
  behind: Arc<AtomicBool>,
) -> Option<SyncSender<Msg>> {
  let (tx, rx) = sync_channel(QUEUE);
  let sink = Sink { root, pending: Vec::new(), bytes: 0, dead: false };
  match std::thread::Builder::new().name("chat-log".into()).spawn(move || run(rx, sink, tell, &behind)) {
    Ok(_) => Some(tx),
    Err(_) => {
      crate::store::tell("couldn't start the chat log writer; chat logs are off".into());
      None
    }
  }
}

fn run(rx: Receiver<Msg>, mut sink: Sink, tell: impl Fn(String), behind: &AtomicBool) {
  let mut due: Option<Instant> = None;
  loop {
    let got = match due {
      None => {
        // about to wait on an empty queue: caught up, so a later overflow is news again
        behind.store(false, Ordering::Relaxed);
        rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
      }
      Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
    };
    let mut now = matches!(got, Err(RecvTimeoutError::Timeout));
    if now {
      behind.store(false, Ordering::Relaxed);
    }
    match got {
      Ok(Msg::Line(e)) => {
        due.get_or_insert_with(|| Instant::now() + BATCH);
        now = sink.push(e);
      }
      Ok(Msg::Flush(ack)) => {
        report(sink.flush(), &tell);
        due = None;
        let _ = ack.send(());
        continue;
      }
      Err(RecvTimeoutError::Timeout) => {}
      Err(RecvTimeoutError::Disconnected) => return report(sink.flush(), &tell),
    }
    if now {
      report(sink.flush(), &tell);
      due = None;
    }
  }
}

fn report(r: std::io::Result<()>, tell: &impl Fn(String)) {
  if let Err(e) = r {
    eprintln!("chat log: {e}");
    tell(format!("couldn't save chat logs ({}): stopped. turn \"save chat logs\" off and on to try again", e.kind()));
  }
}

// what waits for the disk. after a failed write it is `dead`: it says so once (the error is returned
// once) and then drops lines without trying again
struct Sink {
  root: Option<PathBuf>,
  pending: Vec<Entry>,
  bytes: usize,
  dead: bool,
}

impl Sink {
  // true when enough is waiting to write now
  fn push(&mut self, e: Entry) -> bool {
    if self.dead {
      return false;
    }
    self.bytes += e.line.len();
    self.pending.push(e);
    self.bytes >= BATCH_BYTES
  }

  fn flush(&mut self) -> std::io::Result<()> {
    self.bytes = 0;
    let mut all = std::mem::take(&mut self.pending);
    if all.is_empty() || self.dead {
      return Ok(());
    }
    let res = self.write(&mut all);
    self.dead = res.is_err();
    res
  }

  // one append per file: lines of a file stay in the order they came
  fn write(&self, all: &mut [Entry]) -> std::io::Result<()> {
    let root = self.root.as_ref().ok_or_else(|| std::io::Error::other("no home directory"))?;
    all.sort_by(|a, b| (a.platform, &a.chan, &a.day).cmp(&(b.platform, &b.chan, &b.day)));
    let mut i = 0;
    while i < all.len() {
      let k = (all[i].platform, &all[i].chan, &all[i].day);
      let n = all[i..].iter().take_while(|e| (e.platform, &e.chan, &e.day) == k).count();
      let dir = root.join(k.0).join(k.1);
      owner::dir(&dir)?;
      let mut f = owner::log(&dir.join(format!("{}.log", k.2)))?;
      let mut buf = String::with_capacity(all[i..i + n].iter().map(|e| e.line.len()).sum());
      for e in &all[i..i + n] {
        buf.push_str(&e.line);
      }
      f.write_all(buf.as_bytes())?;
      i += n;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::{Arc, Mutex};

  fn root(name: &str) -> PathBuf {
    let r = crate::store::scratch().join(name);
    let _ = std::fs::remove_dir_all(&r);
    r
  }

  fn e(chan: &str, day: &str, line: &str) -> Entry {
    Entry { platform: "twitch", chan: chan.into(), day: day.into(), line: format!("{line}\n") }
  }

  #[test]
  fn lines_append_by_channel_and_day_owner_only() {
    let r = root("log-a");
    let mut s = Sink { root: Some(r.clone()), pending: vec![], bytes: 0, dead: false };
    s.push(e("a", "2026-11-06", "[23:59:59] x: one"));
    s.push(e("b", "2026-11-06", "[23:59:59] y: other"));
    // past midnight: the same channel starts the next day's file
    s.push(e("a", "2026-11-07", "[00:00:00] x: two"));
    s.push(e("a", "2026-11-06", "[23:59:59] x: three"));
    s.flush().unwrap();
    let day = |c: &str, d: &str| std::fs::read_to_string(r.join("twitch").join(c).join(format!("{d}.log"))).unwrap();
    assert_eq!(day("a", "2026-11-06"), "[23:59:59] x: one\n[23:59:59] x: three\n");
    assert_eq!(day("a", "2026-11-07"), "[00:00:00] x: two\n");
    assert_eq!(day("b", "2026-11-06"), "[23:59:59] y: other\n");
    s.push(e("a", "2026-11-06", "[23:59:59] x: four"));
    s.flush().unwrap();
    assert!(day("a", "2026-11-06").ends_with("three\n[23:59:59] x: four\n"));
    #[cfg(unix)]
    {
      use crate::util::owner::mode;
      assert_eq!(mode(&r), 0o700);
      assert_eq!(mode(&r.join("twitch")), 0o700);
      assert_eq!(mode(&r.join("twitch/a")), 0o700);
      assert_eq!(mode(&r.join("twitch/a/2026-11-06.log")), 0o600);
    }
  }

  #[test]
  fn a_burst_asks_to_be_written_early() {
    let mut s = Sink { root: None, pending: vec![], bytes: 0, dead: false };
    let big = "x".repeat(BATCH_BYTES);
    assert!(!s.push(e("a", "d", "hi")));
    assert!(s.push(e("a", "d", &big)));
  }

  #[test]
  fn a_failed_write_is_said_once_and_stops() {
    let r = root("log-b");
    std::fs::create_dir_all(r.parent().unwrap()).unwrap();
    // a file sits where the folder goes
    std::fs::write(&r, "in the way").unwrap();
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::clone(&said);
    let tx = start(Some(r.clone()), move |s| sink.lock().unwrap().push(s), Arc::default()).unwrap();
    let send = |t: &str| tx.send(Msg::Line(e("a", "2026-11-06", t))).unwrap();
    let sync = || {
      let (a, d) = sync_channel(1);
      tx.send(Msg::Flush(a)).unwrap();
      d.recv_timeout(Duration::from_secs(5)).unwrap();
    };
    send("one");
    sync();
    assert_eq!(said.lock().unwrap().len(), 1, "{:?}", said.lock().unwrap());
    assert!(said.lock().unwrap()[0].starts_with("couldn't save chat logs"));
    // even once the way is clear, it does not try again until the switch is toggled
    std::fs::remove_file(&r).unwrap();
    send("two");
    sync();
    send("three");
    sync();
    assert_eq!(said.lock().unwrap().len(), 1);
    assert!(!r.exists());
  }

  #[test]
  fn the_writer_flushes_on_its_own_and_when_the_switch_goes_off() {
    let r = root("log-c");
    let tx = start(Some(r.clone()), |_| {}, Arc::default()).unwrap();
    tx.send(Msg::Line(e("a", "d", "kept"))).unwrap();
    drop(tx);
    let p = r.join("twitch/a/d.log");
    let t = Instant::now();
    while !p.exists() && t.elapsed() < Duration::from_secs(5) {
      std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(std::fs::read_to_string(p).unwrap(), "kept\n");
  }

  #[test]
  fn a_full_queue_says_so_once_and_again_after_it_drains() {
    let _ = crate::store::take_notes();
    let (tx, rx) = sync_channel(2);
    let c = ChatLog { tx: Some(tx), behind: Arc::default() };
    let m = Message::system(1, Platform::Twitch, "hi");
    for _ in 0..6 {
      c.log(Platform::Twitch, "a", &m);
    }
    let said = crate::store::take_notes();
    assert_eq!(said, ["chat logs fell behind: some lines weren't saved"]);
    // the writer catches up (no thread here: drain by hand and clear the flag as it does)
    while rx.try_recv().is_ok() {}
    c.behind.store(false, Ordering::Relaxed);
    for _ in 0..6 {
      c.log(Platform::Twitch, "a", &m);
    }
    assert_eq!(crate::store::take_notes().len(), 1);
  }

  #[test]
  fn the_writer_clears_the_flag_once_it_is_caught_up() {
    let behind = Arc::new(AtomicBool::new(true));
    let tx = start(Some(root("log-d")), |_| {}, Arc::clone(&behind)).unwrap();
    let (a, d) = sync_channel(1);
    tx.send(Msg::Flush(a)).unwrap();
    d.recv_timeout(Duration::from_secs(5)).unwrap();
    let t = Instant::now();
    while behind.load(Ordering::Relaxed) && t.elapsed() < Duration::from_secs(5) {
      std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!behind.load(Ordering::Relaxed));
  }

  #[test]
  fn off_does_nothing_and_on_logs_the_open_channels() {
    let mut c = ChatLog::new(false);
    let m = Message::system(1, Platform::Twitch, "hi");
    c.log(Platform::Twitch, "a", &m);
    c.flush();
    assert!(c.tx.is_none());
    let base = crate::store::logs_dir().unwrap();
    let _ = std::fs::remove_dir_all(&base);
    c.set(true);
    c.log(Platform::Twitch, "../a", &m);
    c.flush();
    let files: Vec<_> = std::fs::read_dir(base.join("twitch/___a")).unwrap().collect();
    assert_eq!(files.len(), 1);
  }
}
