// the starter mod buttons (ext 022b2500): a new mod who never opened the settings saw no toolbar at all and had
// no way to know it existed. the FIRST time you are confirmed a moderator, and only if the buttons were never
// set, delete / timeout 10m / ban are added and the status line says where to change them. "never set" is a
// marker file: any edit of the buttons (including clearing them) writes it, and so does the seed, so it happens
// at most once and never over a choice. a reset of the settings does not bring it back
use super::App;
use crate::i18n::t;
use crate::model::modbar;
use crate::store;

const MARKER: &str = "modseed";
const STARTER: &str = "delete_message timeout_10m ban";

// the buttons were seeded or set by hand: never seed again
pub(super) fn mod_buttons_decided() {
  if store::read(MARKER).is_none() {
    // a failed write is said once by the store; the worst case is one more seed attempt next run
    let _ = store::write(MARKER, "1\n");
  }
}

impl App {
  // you are now confirmed a moderator somewhere (twitch badges or kick's own line)
  pub(super) fn mod_buttons_seed(&mut self) {
    if store::read(MARKER).is_some() {
      return;
    }
    mod_buttons_decided();
    // buttons already picked (an older settings file, or the account) are a choice: leave them
    if self.set.mod_buttons != 0 {
      return;
    }
    self.set.mod_buttons = modbar::parse(STARTER).0;
    self.set.save();
    self.say(t("mod.seed.started"));
    self.wake();
  }
}
