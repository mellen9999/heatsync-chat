// the system clipboard for the app: each call runs the clipboard tools on a blocking thread and
// reports back as a status line or a `Pasted`. a tool that hangs costs a line, never a frozen window
use super::{NetEvent, Out};
use crate::util::clip::{self, Act};

// put text on the clipboard; `done` is the status line for when it worked (empty = none)
pub fn copy(text: String, done: String, out: Out) {
  tokio::task::spawn_blocking(move || match clip::copy(&text) {
    Ok(()) if done.is_empty() => {}
    Ok(()) => out.status(done),
    Err(e) => out.status(e.said(Act::Copy)),
  });
}

pub fn paste(to_picker: bool, out: Out) {
  tokio::task::spawn_blocking(move || {
    let got = clip::paste().map_err(|e| e.said(Act::Paste));
    out.send(NetEvent::Pasted { to_picker, got });
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  // a copy that can't go says why as a status line (too big is refused before any tool is asked)
  #[tokio::test]
  async fn a_refused_copy_comes_back_as_a_status_line() {
    let (tx, rx) = std::sync::mpsc::channel();
    copy("x".repeat(clip::MAX_TEXT + 1), "copied".into(), Out::Pipe(tx));
    let ev = tokio::task::spawn_blocking(move || rx.recv_timeout(std::time::Duration::from_secs(5)))
      .await
      .unwrap()
      .expect("an answer");
    assert!(matches!(ev, NetEvent::Status(s) if s == "couldn't copy: too big (1MB max)"));
  }
}
