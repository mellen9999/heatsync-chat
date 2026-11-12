// a chat line from the box to its feed: twitch over irc, kick and youtube through heatsync
use tokio::sync::mpsc;

use super::{Cmd, irc, session};
use crate::model::message::Platform;

pub(super) fn send_line(
  c: Cmd,
  irc_tx: &mpsc::UnboundedSender<irc::Cmd>,
  sess_tx: &mpsc::UnboundedSender<session::Cmd>,
) {
  match c {
    Cmd::Send { platform: Platform::Twitch, chan, text, reply, .. } => {
      let _ = irc_tx.send(irc::Cmd::Send { chan, text, reply });
    }
    Cmd::Send { platform: Platform::Kick, chan, text, reply, send } => {
      let _ = sess_tx.send(session::Cmd::Kick { chan, text, reply: reply.map(|r| r.id), send });
    }
    Cmd::Send { platform: Platform::Youtube, chan, text, send, .. } => {
      let _ = sess_tx.send(session::Cmd::Youtube { chan, text, send });
    }
    _ => {}
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // every feed's leg leaves with the line's one send id: a youtube leg without it was filed [H] apart from the
  // twitch and kick ones, and every other device drew the send twice
  #[test]
  fn each_feed_leg_leaves_with_its_send_id() {
    let (irc_tx, _irc_rx) = mpsc::unbounded_channel();
    let (sess_tx, mut sess_rx) = mpsc::unbounded_channel();
    for p in [Platform::Kick, Platform::Youtube] {
      let c = Cmd::Send { platform: p, chan: "x".into(), text: "gg".into(), reply: None, send: Some("hsc-1".into()) };
      send_line(c, &irc_tx, &sess_tx);
    }
    let kick = sess_rx.try_recv().unwrap();
    let yt = sess_rx.try_recv().unwrap();
    assert!(matches!(kick, session::Cmd::Kick { send: Some(ref s), .. } if &**s == "hsc-1"));
    assert!(matches!(yt, session::Cmd::Youtube { send: Some(ref s), .. } if &**s == "hsc-1"));
  }
}
