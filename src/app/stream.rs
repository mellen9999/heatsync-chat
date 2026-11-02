// stream lines (went live, switched game, went offline) of tabbed and followed channels in every chat tab
// (ext stream-event + follow_stream_event handlers), and the chat-spike line in every tab (ext
// handleMomentSpike: personal, so only for channels you have a tab for)
use super::App;
use crate::model::message::{Message, Platform};
use crate::model::stream_events::{Line, Tone};
use crate::ui::theme;
use crate::util::time::now_ms;

fn color(t: Tone) -> u32 {
  match t {
    Tone::Online => theme::RED,
    Tone::Switch => theme::YELLOW,
    Tone::Offline | Tone::Moment => theme::WHITE,
  }
}

impl App {
  // a stream line in one tab
  fn put_line(&mut self, tab: usize, platform: Platform, text: &str, tone: Tone) {
    let mut m = Message::system(now_ms(), platform, text);
    m.color = color(tone);
    self.tabs[tab].view.push(m);
    self.touched(tab);
  }

  // the channel's own news in its own tab
  pub(super) fn say_line(&mut self, tab: usize, platform: Platform, text: &str, tone: Tone) {
    self.put_line(tab, platform, text, tone);
    // the channel's own news (not the chat-spike line) turns an unwatched tab yellow
    if tone != Tone::Moment && tab != self.active && !self.tabs[tab].event {
      self.tabs[tab].event = true;
      self.bar_dirty = true;
    }
  }

  pub(super) fn on_stream(&mut self, l: Line) {
    let shown = match l.tone {
      Tone::Online => self.set.ev_online,
      Tone::Offline => self.set.ev_offline,
      Tone::Switch => self.set.ev_game,
      Tone::Moment => self.set.moments,
    };
    if !shown {
      return;
    }
    // in every chat tab (ext: wherever you are reading), followed channels too; only the channel's own
    // tab turns yellow
    let own = self.tabs_with(l.platform, &l.channel);
    for i in 0..self.tabs.len() {
      if own.contains(&i) {
        self.say_line(i, l.platform, &l.text, l.tone);
      } else if !self.tabs[i].sources.is_empty() {
        self.put_line(i, l.platform, &l.text, l.tone);
      }
    }
  }

  pub(super) fn on_moment(&mut self, platform: Platform, key: &str, text: &str) {
    if !self.set.moments || self.tabs_with(platform, key).is_empty() {
      return;
    }
    if !self.moments.allow(now_ms(), platform, key) {
      return;
    }
    let text = format!("[🔥] {text}");
    for i in 0..self.tabs.len() {
      if !self.tabs[i].sources.is_empty() {
        self.put_line(i, platform, &text, Tone::Moment);
      }
    }
  }
}
