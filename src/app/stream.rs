// stream lines (went live, switched game, went offline) of tabbed and followed channels in every chat tab
// (ext stream-event + follow_stream_event handlers), and the chat-spike line in every tab (ext
// handleMomentSpike: personal, so only for channels you have a tab for)
use super::App;
use crate::model::color;
use crate::model::message::{Extra, Kind as MsgKind, Message, Platform, Seg};
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

// a stream line (ext .hs-mc-stream-event): the streamer's name as in chat (their color, their paint),
// `◆ went live — ` in the tone's color, the game white, and a stripe of the tone down the left
fn row(l: &Line, color: u32, user_id: Option<u64>) -> Message {
  let ink = self::color(l.tone);
  let mut m = Message::system(now_ms(), l.platform, "");
  m.kind = MsgKind::Action;
  m.login = l.channel.clone();
  m.nick = l.name.clone();
  m.color = color;
  m.user_id = user_id.map(|n| n.to_string().into()).unwrap_or_default();
  let mut tail = vec![Seg { text: format!(" {}", l.verb).into(), ink, bold: false }];
  if !l.game.is_empty() {
    tail.push(Seg { text: l.game.clone(), ink: theme::WHITE, bold: false });
  }
  m.x = Some(Box::new(Extra { tail: tail.into(), edge: ink, ..Extra::default() }));
  m
}

impl App {
  // a stream line in one tab. news about other channels never lights a tab: only chat does (unread)
  // and the channel's own news (yellow)
  fn put_line(&mut self, tab: usize, m: Message) {
    self.tabs[tab].chat().push(m);
    if tab == self.active {
      self.wake();
    }
  }

  // the channel's own news in its own tab
  pub(super) fn say_line(&mut self, tab: usize, m: Message, tone: Tone) {
    self.put_line(tab, m);
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
    // their color: the frame's, else what chat has seen of them, else twitch's name palette
    let known = self.res.chatters.person(l.platform, &l.channel, true);
    let c = l.color.or(known.as_ref().map(|p| p.color)).unwrap_or_else(|| color::default_for(&l.channel));
    let uid = (l.platform == Platform::Twitch).then(|| self.res.chatters.twitch_id(&l.channel)).flatten();
    let m = row(&l, color::readable(c), uid);
    for i in 0..self.tabs.len() {
      if own.contains(&i) {
        self.say_line(i, m.clone(), l.tone);
      } else if !self.tabs[i].sources.is_empty() {
        self.put_line(i, m.clone());
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
    let mut m = Message::system(now_ms(), platform, format!("[🔥] {text}"));
    m.color = color(Tone::Moment);
    for i in 0..self.tabs.len() {
      if !self.tabs[i].sources.is_empty() {
        self.put_line(i, m.clone());
      }
    }
  }
}
