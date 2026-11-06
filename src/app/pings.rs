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
      (false, _) => "too many channel rules: clear one first",
      (_, Bell::On) => "bell on: you hear when this channel goes live",
      (_, Bell::Off) => "bell off: no popups from this channel",
      (_, Bell::Default) => "bell default: follows desktop notifications",
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
      (false, _) => "too many channel rules: clear one first",
      (_, true) => "pings muted here: mentions still land in the mentions tab",
      (_, false) => "pings back on",
    });
  }
}
