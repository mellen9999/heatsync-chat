// the per-tab platform filter (ext platformFilters) as the app sees it: the tab menu's `show twitch: on` rows,
// the bits each tab's view hides, and the quiet that goes with hiding: a hidden line is not drawn, does not
// ping and does not light its tab. the choice is device-local (model::platfilter), never sent to the account
use super::tabs::spec_of;
use super::{App, Tab};
use crate::model::message::{Message, Platform};
use crate::model::platfilter::{bit, linked};

pub(super) const ALL: [Platform; 3] = [Platform::Twitch, Platform::Kick, Platform::Youtube];

impl Tab {
  // the key its choice is kept under: its feeds line
  fn plat_key(&self) -> String {
    spec_of(self).line()
  }
}

impl App {
  fn tab_hides(&self, i: usize) -> u8 {
    self.tabs.get(i).map_or(0, |t| self.plats.hidden(&t.plat_key(), linked(&t.sources)))
  }

  pub(super) fn tab_filtered(&self, i: usize) -> bool {
    self.tab_hides(i) != 0
  }

  // every tab's view hides what the file says (boot, a tab added, edited or laid out by the account)
  pub(super) fn sync_plats(&mut self) {
    for i in 0..self.tabs.len() {
      let bits = self.tab_hides(i);
      for v in self.tabs[i].views() {
        v.set_hidden_platforms(bits);
      }
    }
  }

  // the menu's rows: one per feed of a merged tab, `show kick: on`
  pub(super) fn plat_rows(&self, i: usize) -> Vec<(Platform, String)> {
    let Some(t) = self.tabs.get(i).filter(|t| t.sources.len() > 1) else { return Vec::new() };
    let off = self.tab_hides(i);
    ALL
      .into_iter()
      .filter(|p| t.sources.iter().any(|s| s.0 == *p))
      .map(|p| (p, format!("show {}: {}", p.name(), if off & bit(p) == 0 { "on" } else { "off" })))
      .collect()
  }

  pub(super) fn toggle_plat(&mut self, i: usize, p: Platform) {
    let Some(t) = self.tabs.get(i) else { return };
    let (key, feeds) = (t.plat_key(), linked(&t.sources));
    if !self.plats.flip(&key, p, feeds) {
      return self.say(format!("can't hide {}: a tab keeps at least one platform showing", p.name()));
    }
    self.plats.save();
    self.sync_plats();
    self.bar_dirty = true;
    self.wake();
  }

  // tab `i` hides this line's platform: it counts as nothing there
  pub(super) fn plat_quiet(&self, i: usize, m: &Message) -> bool {
    m.kind.said() && self.tab_hides(i) & bit(m.platform) != 0
  }

  // every tab carrying that feed hides it: a mention there pings nobody
  pub(super) fn plat_silenced(&self, p: Platform, key: &str) -> bool {
    let tabs = self.tabs_with(p, key);
    !tabs.is_empty() && tabs.iter().all(|&i| self.tab_hides(i) & bit(p) != 0)
  }
}
