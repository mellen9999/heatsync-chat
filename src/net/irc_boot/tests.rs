// the boot handshake: irc connects once, after the session's first identity answer
use super::*;
use futures_util::{SinkExt, StreamExt};

// a local stand-in for twitch: counts sockets, hands each one's PASS line to the test
async fn fake_twitch() -> (String, mpsc::UnboundedReceiver<String>) {
  let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let url = format!("ws://{}", l.local_addr().unwrap());
  let (tx, rx) = mpsc::unbounded_channel();
  tokio::spawn(async move {
    loop {
      let Ok((s, _)) = l.accept().await else { return };
      let tx = tx.clone();
      tokio::spawn(async move {
        let Ok(mut ws) = tokio_tungstenite::accept_async(s).await else { return };
        // pings keep the client from calling the line quiet while the test clock runs ahead
        let mut ping = interval(Duration::from_secs(30));
        loop {
          tokio::select! {
            m = ws.next() => {
              let Some(Ok(Ws::Text(m))) = m else { return };
              if let Some(p) = m.lines().find(|l| l.starts_with("PASS ")) {
                let _ = tx.send(p.to_owned());
              }
            }
            _ = ping.tick() => {
              if ws.send(Ws::text("PING :tmi.twitch.tv")).await.is_err() {
                return;
              }
            }
          }
        }
      });
    }
  });
  (url, rx)
}

fn boot(url: String) -> (mpsc::UnboundedSender<Cmd>, std::sync::mpsc::Receiver<super::super::NetEvent>) {
  let (tx, rx) = mpsc::unbounded_channel();
  let (stx, _srx) = mpsc::unbounded_channel();
  let (otx, orx) = std::sync::mpsc::channel();
  tokio::spawn(run_at_owned(url, rx, stx, Out::Pipe(otx)));
  (tx, orx)
}

async fn run_at_owned(
  url: String,
  rx: mpsc::UnboundedReceiver<Cmd>,
  sess: mpsc::UnboundedSender<super::super::session::Cmd>,
  out: Out,
) {
  let _ = rustls::crypto::ring::default_provider().install_default();
  run_at(&url, rx, sess, reqwest::Client::new(), out).await
}

fn statuses(rx: &std::sync::mpsc::Receiver<super::super::NetEvent>) -> Vec<String> {
  rx.try_iter()
    .filter_map(|e| match e {
      super::super::NetEvent::Status(s) => Some(s),
      _ => None,
    })
    .collect()
}

fn creds() -> Option<Creds> {
  Some(Creds { token: "tok".into(), nick: "me".into() })
}

#[tokio::test(start_paused = true)]
async fn a_logged_in_boot_connects_once_signed_in_and_never_read_only() {
  let (url, mut socks) = fake_twitch().await;
  let (tx, out) = boot(url);
  tx.send(Cmd::Join("forsen".into())).unwrap();
  tx.send(Cmd::Login(creds())).unwrap();
  assert_eq!(socks.recv().await.unwrap(), "PASS oauth:tok");
  sleep(Duration::from_secs(3)).await;
  assert!(socks.try_recv().is_err(), "one socket, not an anonymous one first");
  assert_eq!(statuses(&out), vec![t("net1.irc.connected").to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn a_logged_out_boot_says_read_only_once() {
  let (url, mut socks) = fake_twitch().await;
  let (tx, out) = boot(url);
  tx.send(Cmd::Login(None)).unwrap();
  assert_eq!(socks.recv().await.unwrap(), "PASS SCHMOOPIIE");
  // the session's later "still anonymous" is no reason to reconnect
  tx.send(Cmd::Login(None)).unwrap();
  sleep(Duration::from_secs(3)).await;
  assert!(socks.try_recv().is_err());
  assert_eq!(statuses(&out), vec![t("net1.irc.connected_ro").to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_session_still_connects_read_only_after_the_deadline() {
  let (url, mut socks) = fake_twitch().await;
  let (_tx, out) = boot(url);
  sleep(BOOT_WAIT - Duration::from_secs(1)).await;
  assert!(socks.try_recv().is_err(), "still waiting for the session");
  assert_eq!(socks.recv().await.unwrap(), "PASS SCHMOOPIIE");
  assert_eq!(statuses(&out), vec![t("net1.irc.connected_ro").to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn a_boot_login_lands_before_the_first_connect_so_nothing_is_lost_or_caught_up() {
  let (tx, mut rx) = mpsc::unbounded_channel();
  let mut chans = BTreeSet::new();
  let mut c = None;
  let (otx, _orx) = std::sync::mpsc::channel();
  tx.send(Cmd::Join("forsen".into())).unwrap();
  tx.send(Cmd::Login(creds())).unwrap();
  assert!(boot_wait(&mut rx, &mut chans, &mut c, &Out::Pipe(otx), BOOT_WAIT).await);
  assert!(c.is_some() && chans.contains("forsen"), "identity and joins are in before the first connect");
}
