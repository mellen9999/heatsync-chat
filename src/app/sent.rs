// what you sent from the box survives a restart (ext 66af7102): up / down recall it. kept on this device only in
// the `sent` file (`model::sent_file`), written soon after each send, restored at boot. `remember_sent` off keeps
// it in memory only and removes the file; turning it on starts a fresh list, so the disk only ever holds lines
// sent while it was on. /clearsent forgets it all, whatever the switch says
use super::App;
use crate::model::sent_file;
use crate::store;

impl App {
  // a send happened: the file follows the box's history
  pub(super) fn sent_save(&self) {
    if self.set.remember_sent {
      store::write_later(sent_file::FILE, &sent_file::encode(self.line.sent()));
    }
  }

  // at boot: the last run's lines come back behind anything already sent. a file that is not ours is set aside
  pub(super) fn sent_restore(&mut self) {
    if !self.set.remember_sent {
      return store::remove(sent_file::FILE);
    }
    let Some(text) = store::read_capped(sent_file::FILE, sent_file::MAX_BYTES) else { return };
    let lines = sent_file::decode(&text);
    if lines.is_empty() && !text.trim().is_empty() {
      return store::set_aside(sent_file::FILE, None);
    }
    self.line.restore_sent(lines);
  }

  // the switch moved
  pub(super) fn sent_follow(&mut self) {
    if self.set.remember_sent {
      self.line.clear_sent();
    } else {
      store::remove(sent_file::FILE);
    }
  }

  // /clearsent
  pub(super) fn sent_clear(&mut self) {
    self.line.clear_sent();
    store::remove(sent_file::FILE);
    self.say("sent history cleared");
  }
}
