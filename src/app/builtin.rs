// the built-in tabs (feed, whispers, mentions): created once at boot in the ext's order, shown or
// hidden with /show and /hide, and the helpers that keep every tab-stepping key off hidden ones
use super::{App, SCROLLBACK, Tab};
use crate::model::tabs::{BUILTIN, Kind};
use crate::ui::chat_view::ChatView;

// the ext keeps 500 mentions (MAX_BUFFER) and 150 feed posts
const MENTIONS: usize = 500;
const POSTS: usize = 150;
const WHISPERS: usize = 500;

pub(super) fn make_all() -> Vec<Tab> {
  BUILTIN.into_iter().map(make).collect()
}

fn make(kind: Kind) -> Tab {
  let name = kind.name();
  let cap = match kind {
    Kind::Feed => POSTS,
    Kind::Whispers => WHISPERS,
    Kind::Mentions => MENTIONS,
    Kind::Chat => SCROLLBACK,
  };
  let mut view = ChatView::new(cap, name.into());
  // a copied or synthesized line is no chatter: no first-line glow, no relative-time clash
  view.glow = false;
  view.top = kind == Kind::Feed;
  view.timestamps = kind != Kind::Feed;
  view.tags = kind == Kind::Mentions;
  view.set_empty(match kind {
    Kind::Mentions => "no mentions yet. lines that @ you or reply to you, in any joined channel, land here",
    Kind::Whispers => "log in to read and send twitch whispers (ctrl-l)",
    Kind::Feed => "loading...",
    Kind::Chat => "",
  });
  Tab {
    name: name.into(),
    kind,
    view,
    unread: false,
    event: false,
    sources: Vec::new(),
    target: 0,
    under: None,
    count: Default::default(),
    stats: Default::default(),
  }
}

impl App {
  pub(super) fn visible(&self, i: usize) -> bool {
    self.tabs.get(i).is_some_and(|t| self.set.shown(t.kind))
  }

  pub(super) fn tab_of(&self, k: Kind) -> Option<usize> {
    self.tabs.iter().position(|t| t.kind == k)
  }

  // the next shown tab from `from` (wraps); `from` itself when it is the only one
  pub(super) fn step_tab(&self, from: usize, back: bool) -> usize {
    let n = self.tabs.len();
    (1..=n)
      .map(|d| if back { (from + n - d % n) % n } else { (from + d) % n })
      .find(|&i| self.visible(i))
      .unwrap_or(from)
  }

  // the n-th shown tab (alt-1 .. alt-9)
  pub(super) fn nth_visible(&self, n: usize) -> Option<usize> {
    (0..self.tabs.len()).filter(|&i| self.visible(i)).nth(n)
  }

  // a tab came into view: load what it shows the first time (or when stale)
  pub(super) fn entered(&mut self) {
    match self.kind() {
      Kind::Whispers => self.whispers_load(),
      Kind::Feed => self.feed_entered(),
      _ => {}
    }
  }

  // after tabs come and go: never rest on a hidden one
  pub(super) fn fix_active(&mut self) {
    if !self.visible(self.active) {
      let i = self.step_tab(self.active, false);
      self.active = i;
      if let Some(t) = self.tabs.get_mut(i) {
        t.unread = false;
        t.view.invalidate();
      }
      self.input_dirty = true;
    }
    self.bar_dirty = true;
    self.wake();
  }

  pub(super) fn set_shown(&mut self, k: Kind, on: bool) {
    self.set.show(k, on);
    self.set.save();
    if on && let Some(i) = self.tab_of(k) {
      self.select(i);
    }
    self.fix_active();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn builtins_are_in_the_ext_order_and_sized_like_it() {
    let tabs = make_all();
    let names: Vec<&str> = tabs.iter().map(|t| &*t.name).collect();
    assert_eq!(names, ["feed", "whispers", "mentions"]);
    assert!(tabs[0].view.top && !tabs[1].view.top && !tabs[2].view.top);
    assert!(tabs.iter().all(|t| t.sources.is_empty() && !t.view.glow));
  }
}
