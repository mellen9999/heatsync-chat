// the font lookup for the app: fc-match runs on a blocking thread and the answer comes back as one event,
// so a slow font cache costs a status line, never a frozen window
use super::{NetEvent, Out};
use crate::util::font;

pub fn find(name: Box<str>, out: Out) {
  tokio::task::spawn_blocking(move || {
    let found = font::find(&name);
    out.send(NetEvent::Font { asked: name, found });
  });
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;

  // a name nobody has comes back as a plain sentence naming it, tagged with what was asked. unix only (the
  // module): on windows this step only checks the name, and the text engine answers whether it's installed
  // (app/font.rs)
  #[tokio::test]
  async fn a_missing_family_comes_back_as_a_sentence() {
    let (tx, rx) = std::sync::mpsc::channel();
    find("hsc no such family 9".into(), Out::Pipe(tx));
    let ev = tokio::task::spawn_blocking(move || rx.recv_timeout(std::time::Duration::from_secs(10)))
      .await
      .unwrap()
      .expect("an answer");
    let NetEvent::Font { asked, found } = ev else { panic!("a font answer") };
    assert_eq!(&*asked, "hsc no such family 9");
    // no fc-match at all is a sentence too
    let said = found.expect_err("nobody has it");
    assert!(said.contains("hsc no such family 9") || said.contains("fc-match"), "{said}");
  }
}
