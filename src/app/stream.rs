// stream lines (went live, switched game, went offline) of tabbed and followed channels in every chat tab
// (ext stream-event + follow_stream_event handlers), and the chat-spike line in every tab (ext
// handleMomentSpike: personal, so only for channels you have a tab for)
use std::collections::HashMap;

use super::App;
use crate::model::chatters::Chatters;
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
  m.x = Some(Box::new(Extra { tail: tail.into(), edge: ink, late: l.color.is_none(), ..Extra::default() }));
  m
}

// a stream line's name without the frame's color: followed colors (twitch), else who chat or a lookup
// says they are (the lookup is queued if no one has), else twitch's palette; plus the twitch id their
// ffz badges and paints hang off
fn hue(follow: &HashMap<Box<str>, u32>, chatters: &mut Chatters, p: Platform, login: &str) -> (u32, Option<u64>) {
  let person = chatters.known(p, login);
  let c = (p == Platform::Twitch)
    .then(|| follow.get(login.to_ascii_lowercase().as_str()).copied())
    .flatten()
    .or(person.map(|x| x.color))
    .unwrap_or_else(|| color::default_for(login));
  (color::readable(c), (p == Platform::Twitch).then(|| chatters.twitch_id(login)).flatten())
}

impl App {
  // every stream line still waiting on its streamer's color learns what is known now
  pub(super) fn restyle_stream_lines(&mut self) {
    let (follow, chatters) = (&self.follow_colors, &mut self.res.chatters);
    for t in self.tabs.iter_mut() {
      for v in t.views() {
        v.restyle_late(|p, login| hue(follow, chatters, p, login));
      }
    }
    self.wake();
  }

  pub(super) fn on_follow_colors(&mut self, colors: Vec<(Box<str>, u32)>) {
    // the ext clears its map past 500; ours holds a frame's worth more
    if self.follow_colors.len() > 2000 {
      self.follow_colors.clear();
    }
    self.follow_colors.extend(colors);
    self.restyle_stream_lines();
  }

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
    self.tabs[tab].chat().push(m);
    self.touched(tab);
    // the channel's own news (not the chat-spike line) turns an unwatched tab yellow
    if tone != Tone::Moment && tab != self.active && !self.tabs[tab].event {
      self.tabs[tab].event = true;
      self.bar_dirty = true;
    }
  }

  pub(super) fn on_stream(&mut self, l: Line) {
    if matches!(l.tone, Tone::Online | Tone::Offline) {
      self.stats_stream(l.platform, &l.channel, l.tone == Tone::Online);
    }
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
    // their color: the frame's, else the followed colors, else what chat has seen of them (a lookup is
    // queued if nobody has), else twitch's name palette. a line without the frame's color follows the
    // lookup when it lands (`restyle`)
    let (c, uid) = match l.color {
      Some(c) => (
        color::readable(c),
        (l.platform == Platform::Twitch).then(|| self.res.chatters.twitch_id(&l.channel)).flatten(),
      ),
      None => hue(&self.follow_colors, &mut self.res.chatters, l.platform, &l.channel),
    };
    let m = row(&l, c, uid);
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
