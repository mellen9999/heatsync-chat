// stream stats (ext stream-stats.js): each channel tab counts its live lines, chatters, mentions and
// emotes while `stream stats` is on, from when this window first saw its chat; the channel going live
// starts the count over and going offline titles it "ended". /summary (or the tab's menu) shows it
use super::App;
use super::screen::Page;
use crate::model::message::{Message, Platform, flag};
use crate::model::stats::{self, MAX_WORD, MAX_WORDS, Stats};
use crate::ui::sheet::L;
use crate::util::time::now_ms;

// a tab's count, and when its channel went offline
#[derive(Default)]
pub(super) struct Count {
  pub stats: Option<Box<Stats>>,
  pub ended: bool,
}

impl App {
  // a live line landed in tab `i`, from room `room`
  pub(super) fn tally(&mut self, i: usize, room: &str, m: &Message) {
    if !self.set.stream_stats || m.has(flag::HISTORY) || m.kind == crate::model::message::Kind::System {
      return;
    }
    let now = now_ms();
    let emotes = &self.res.emotes;
    let body = &*m.body;
    // the platform's own emotes by place, then the words that are a channel's or a global emote
    let native = m.emotes.iter().filter_map(|(r, _)| body.get(r.start as usize..r.end as usize));
    let words = body
      .split(' ')
      .take(MAX_WORDS)
      .filter(|w| !w.is_empty() && w.len() <= MAX_WORD)
      .filter(|w| !m.emotes.iter().any(|(r, _)| body.get(r.start as usize..r.end as usize) == Some(*w)))
      .filter(|w| emotes.channel(room, w).or_else(|| emotes.global(w)).is_some());
    let s = self.tabs[i].stats.stats.get_or_insert_with(|| Box::new(Stats::new(now)));
    s.count(now, &m.nick, m.has(flag::ALERT), native.chain(words));
  }

  // the channel went live (a fresh count) or offline (the count is its recap)
  pub(super) fn stats_stream(&mut self, p: Platform, chan: &str, live: bool) {
    for i in self.tabs_with(p, chan) {
      let c = &mut self.tabs[i].stats;
      if live {
        *c = Count::default();
      } else {
        c.ended = c.stats.is_some();
      }
    }
  }

  // /summary: the active channel tab's
  pub(super) fn summary_here(&mut self) {
    match self.tabs.get(self.active).filter(|t| !t.sources.is_empty()) {
      Some(t) => {
        let name = t.name.clone();
        self.summary_open(name);
      }
      None => self.say("/summary needs a channel tab"),
    }
  }

  pub(super) fn summary_open(&mut self, tab: Box<str>) {
    self.open_screen(Page::Summary);
    if let Some(s) = &mut self.screen {
      s.of = tab;
    }
    self.summary_refresh();
  }

  // the screen's lines, read fresh from the count
  pub(super) fn summary_refresh(&mut self) {
    let Some(s) = self.screen.as_ref().filter(|s| s.page() == Page::Summary) else { return };
    let tab = self.tabs.iter().find(|t| t.name == s.of);
    let (title, lines) = summary(&s.of, tab.map(|t| &t.stats), self.set.stream_stats, self.res.look.twelve);
    if let Some(s) = &mut self.screen {
      (s.shown, s.title) = (lines, title);
    }
  }
}

fn summary(name: &str, c: Option<&Count>, on: bool, twelve: bool) -> (String, Vec<L>) {
  let row = |a: String, b: String| L::Row { a, b, pill: None };
  let ended = c.is_some_and(|c| c.ended);
  let title = format!("{name} stream {}", if ended { "ended" } else { "summary" });
  if !on {
    return (title, vec![row("stream stats are off: /streamstats turns them on".into(), String::new())]);
  }
  let Some(s) = c.and_then(|c| c.stats.as_deref()).filter(|s| s.msgs > 0) else {
    return (title, vec![row("no chat counted yet: it fills in as messages arrive".into(), String::new())]);
  };
  let n = |v: u64| stats::group(v);
  let mut lines = vec![
    row(
      format!("{} msgs · {} chatters · {} mentions · {}/s peak", n(s.msgs), s.chatters(), n(s.mentions), s.peak),
      String::new(),
    ),
    row(
      format!("since {} · {}", crate::util::time::clock(s.since, twelve), stats::span(now_ms() - s.since)),
      String::new(),
    ),
  ];
  for (head, top) in [("top chatters", s.top_chatters()), ("top emotes", s.top_emotes())] {
    if top.is_empty() {
      continue;
    }
    lines.push(L::Head(head.into()));
    lines.extend(top.into_iter().map(|(k, v)| row(k.into(), n(v.into()))));
  }
  (title, lines)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::Net;

  #[test]
  fn a_tab_counts_live_lines_and_the_summary_reads_them() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
    let line = |nick: &str, body: &str, flags: u8| {
      let mut m = Message::system(now_ms(), Platform::Twitch, body);
      m.kind = crate::model::message::Kind::Chat;
      (m.login, m.nick, m.flags) = (nick.to_lowercase().into(), nick.into(), flags);
      m
    };
    app.summary_here();
    assert!(matches!(&app.screen.as_ref().unwrap().shown[0], L::Row { a, .. } if a.starts_with("no chat counted yet")));
    app.close_screen();
    app.tally(tab, "forsen", &line("Bob", "hi", 0));
    app.tally(tab, "forsen", &line("Bob", "hey @mellen", flag::ALERT));
    app.tally(tab, "forsen", &line("Amy", "old", flag::HISTORY));
    app.summary_here();
    let s = app.screen.as_ref().unwrap();
    assert_eq!(s.title, "forsen stream summary");
    assert!(
      matches!(&s.shown[0], L::Row { a, .. } if a == "2 msgs · 1 chatters · 1 mentions · 2/s peak"),
      "history isn't counted"
    );
    assert!(s.shown.iter().any(|l| matches!(l, L::Row { a, b, .. } if a == "Bob" && b == "2")));
    app.redraw();
    // offline: the recap; live again: a fresh count
    app.stats_stream(Platform::Twitch, "forsen", false);
    app.summary_refresh();
    assert_eq!(app.screen.as_ref().unwrap().title, "forsen stream ended");
    app.stats_stream(Platform::Twitch, "forsen", true);
    assert!(app.tabs[tab].stats.stats.is_none());
    // off: nothing is counted
    app.set.stream_stats = false;
    app.tally(tab, "forsen", &line("Bob", "hi", 0));
    assert!(app.tabs[tab].stats.stats.is_none());
  }
}
