// per-channel notification rules (model::pings) as the tab menu sees them: a tab's state is its first
// channel's, a change goes to every channel of the tab. no slash commands (the ext has none)
use super::App;
use crate::model::pings::Bell;

impl App {
  pub(super) fn tab_bell(&self, i: usize) -> Bell {
    self.tabs.get(i).and_then(|t| t.sources.first()).map_or(Bell::Default, |(p, k)| self.pings.bell(*p, k))
  }

  pub(super) fn tab_muted(&self, i: usize) -> bool {
    self.tabs.get(i).and_then(|t| t.sources.first()).is_some_and(|(p, k)| self.pings.muted(*p, k))
  }

  // default → on → off → default, said on the status line
  pub(super) fn cycle_tab_bell(&mut self, i: usize) {
    let to = self.tab_bell(i).next();
    let Some(t) = self.tabs.get(i) else { return };
    let ok = t.sources.iter().all(|(p, k)| self.pings.set_bell(*p, k, to));
    self.pings.save();
    self.say(match (ok, to) {
      (false, _) => crate::i18n::t("app.pings.too_many"),
      (_, Bell::On) => crate::i18n::t("app.pings.bell_on"),
      (_, Bell::Off) => crate::i18n::t("app.pings.bell_off"),
      (_, Bell::Default) => crate::i18n::t("app.pings.bell_default"),
    });
  }

  pub(super) fn toggle_tab_mute(&mut self, i: usize) {
    let to = !self.tab_muted(i);
    let Some(t) = self.tabs.get(i) else { return };
    let ok = t.sources.iter().all(|(p, k)| self.pings.set_mute(*p, k, to));
    self.pings.save();
    self.bar_dirty = true;
    self.wake();
    self.say(match (ok, to) {
      (false, _) => crate::i18n::t("app.pings.too_many"),
      (_, true) => crate::i18n::t("app.pings.muted"),
      (_, false) => crate::i18n::t("app.pings.back_on"),
    });
  }
}
