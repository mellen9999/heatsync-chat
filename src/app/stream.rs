// stream lines (went live, switched game, went offline) of tabbed and followed channels in every chat tab
// (ext stream-event + follow_stream_event handlers), and the chat-spike line in every tab (ext
// handleMomentSpike: personal, so only for channels you have a tab for)
use std::collections::HashMap;

use super::App;
use crate::model::chatters::Chatters;
use crate::model::color;
use crate::model::ident;
use crate::model::message::{Extra, Kind as MsgKind, Message, Platform, Seg, flag};
use crate::model::status::Info;
use crate::model::stream_events::{Line, Tone};
use crate::net::Cmd;
use crate::ui::theme;
use crate::util::notify;
use crate::util::time::now_ms;

fn color(t: Tone) -> u32 {
  match t {
    Tone::Online => theme::RED,
    Tone::Switch => theme::YELLOW,
    Tone::Offline | Tone::Moment => theme::WHITE,
    Tone::Raid => theme::TWITCH,
    Tone::Gift => theme::GREEN,
    Tone::Redeem => theme::CYAN,
    // the ext's gold, in our palette
    Tone::Pred => theme::YELLOW,
  }
}

// a stream line (ext .hs-mc-stream-event): the streamer's lowercase login as in chat (their color, their
// paint), `◆ went live — ` in the tone's color, the game white, and a stripe of the tone down the left.
// `user_id` is their id on the line's platform, which their paint and badges hang off
fn row(l: &Line, color: u32, user_id: Option<Box<str>>) -> Message {
  let ink = self::color(l.tone);
  let mut m = Message::system(now_ms(), l.platform, "");
  m.kind = MsgKind::Event;
  m.login = l.person();
  // a redeem keeps the redeemer's own casing; the streamer is always the lowercase login
  m.nick = if l.tone == Tone::Redeem { l.name.clone() } else { m.login.clone() };
  m.color = color;
  let late_id = user_id.is_none();
  m.user_id = user_id.unwrap_or_default();
  let mut tail = vec![Seg { text: format!(" {}", l.verb).into(), ink, bold: false }];
  if !l.game.is_empty() {
    tail.push(Seg { text: l.game.clone(), ink: theme::WHITE, bold: false });
  }
  let opens = l.tone == Tone::Online;
  m.x =
    Some(Box::new(Extra { tail: tail.into(), edge: ink, late: l.color.is_none(), late_id, opens, ..Extra::default() }));
  m
}

// a stream line's name color without the frame's: followed colors (twitch only), else who chat or a
// lookup on that platform says they are (the lookup is queued if no one has), else twitch's palette
fn hue(follow: &HashMap<Box<str>, u32>, chatters: &mut Chatters, p: Platform, login: &str) -> u32 {
  let c = (p == Platform::Twitch)
    .then(|| follow.get(login.to_ascii_lowercase().as_str()).copied())
    .flatten()
    .or(chatters.known(p, login).map(|x| x.color))
    .unwrap_or_else(|| color::default_for(login));
  color::readable(c)
}

impl App {
  // every stream line still waiting on its streamer's color learns what is known now
  pub(super) fn restyle_stream_lines(&mut self) {
    let (follow, chatters) = (&self.follow_colors, &mut self.res.chatters);
    for t in self.tabs.iter_mut() {
      for v in t.views() {
        v.restyle_late(|p, login| (hue(follow, chatters, p, login), chatters.id_of(p, login)));
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
    // the channel's own news (not the chat-spike line, a gift bomb or a redeem) turns an unwatched tab yellow
    if !matches!(tone, Tone::Moment | Tone::Gift | Tone::Redeem) && tab != self.active && !self.tabs[tab].event {
      self.tabs[tab].event = true;
      self.bar_dirty = true;
    }
  }

  // the desktop notification for a stream line (ext notifyStreamEvent): a channel went live or
  // switched game while the window is away, if desktop notifications are on. no sound, like the ext.
  // the line already passed the ext's dedupe (model::stream_events::Gate) and its show toggle
  fn stream_alert(&self, l: &Line) -> Option<(String, String)> {
    if self.focused || !self.set.notify {
      return None;
    }
    let what = match l.tone {
      Tone::Online => "went live",
      Tone::Switch => "switched game",
      _ => return None,
    };
    Some((format!("{} {what}", notify::name(&l.channel)), l.game.to_string()))
  }

  // the row for a line. their color: the frame's, else the followed colors, else what chat has seen of them
  // on that platform (a lookup is queued if nobody has), else twitch's name palette. their id: the frame's
  // `userId`, else what chat learned, else a lookup (queued). a color or id still missing follows the
  // lookup when it lands (`restyle_stream_lines`); a frame's color stays
  fn stream_row(&mut self, l: &Line) -> Message {
    let who = l.person();
    let id = l.user_id.clone().or_else(|| self.res.chatters.id_of(l.platform, &who));
    let c = match l.color {
      Some(c) => color::readable(c),
      None => hue(&self.follow_colors, &mut self.res.chatters, l.platform, &who),
    };
    row(l, c, id)
  }

  // `follow:history`: switched and offline lines of the last 15 min come back at their own times. no
  // notification, no lit tab; online is never replayed; a line the live frames already showed is skipped
  pub(super) fn on_stream_history(&mut self, rows: Vec<(i64, Line)>) {
    for (at, l) in rows.into_iter().take(200) {
      let shown = match l.tone {
        Tone::Switch => self.set.ev_game,
        Tone::Offline => self.set.ev_offline,
        _ => false,
      };
      if !shown {
        continue;
      }
      let mut m = self.stream_row(&l);
      m.ts = at;
      m.id = format!("stream-history:{}:{}:{}{}:{at}", l.platform.name(), l.channel, l.verb, l.game).into();
      m.flags |= flag::HISTORY;
      let words = |m: &Message| -> Vec<Box<str>> {
        m.x.as_ref().map(|x| x.tail.iter().map(|s| s.text.clone()).collect()).unwrap_or_default()
      };
      let mine = words(&m);
      for i in 0..self.tabs.len() {
        if self.tabs[i].sources.is_empty() {
          continue;
        }
        let v = self.tabs[i].chat();
        let twin = v.msgs().rev().any(|o| {
          o.kind == MsgKind::Event
            && o.platform == m.platform
            && o.login == m.login
            && o.ts.abs_diff(at) < 60_000
            && words(o) == mine
        });
        if !twin {
          v.prepend(vec![m.clone()]);
          if i == self.active {
            self.wake();
          }
        }
      }
    }
  }

  pub(super) fn on_stream(&mut self, l: Line) {
    if matches!(l.tone, Tone::Online | Tone::Offline) {
      self.stats_stream(l.platform, &l.channel, l.tone == Tone::Online);
      self.live_mark(l.platform, &l.channel, l.tone == Tone::Online);
      // one went-live per channel until it signs off, whichever of the socket and the poll says it first
      // (ext sessionWentLiveSeen)
      let k = (l.platform, l.channel.clone());
      if l.tone == Tone::Offline {
        self.told.remove(&k);
      } else {
        if self.told.len() >= 2000 {
          self.told.clear();
        }
        if !self.told.insert(k) {
          return;
        }
      }
    }
    let shown = match l.tone {
      Tone::Online => self.set.ev_online,
      Tone::Offline => self.set.ev_offline,
      Tone::Switch => self.set.ev_game,
      Tone::Moment => self.set.moments,
      Tone::Raid => self.set.ev_raid,
      Tone::Gift => self.set.ev_sub,
      Tone::Redeem => self.set.ev_redeem,
      Tone::Pred => self.set.ev_pred,
    };
    if !shown {
      return;
    }
    if let Some((title, body)) = self.stream_alert(&l) {
      notify::send("went-live", &title, &body);
    }
    // in every chat tab (ext: wherever you are reading), followed channels too; only the channel's own
    // tab turns yellow
    let own = self.tabs_with(l.platform, &l.channel);
    let m = self.stream_row(&l);
    // a room's other news stays in its own tab (ext: the live channel's buffer)
    let everywhere = matches!(l.tone, Tone::Online | Tone::Offline | Tone::Switch);
    for i in 0..self.tabs.len() {
      if own.contains(&i) {
        self.say_line(i, m.clone(), l.tone);
      } else if everywhere && !self.tabs[i].sources.is_empty() {
        self.put_line(i, m.clone());
      }
    }
  }

  // a channel already live when its history loaded: twitch says since when (the /status query), and its
  // "went live" line goes in at that time if the history reaches back that far. the realtime line only
  // comes for a go-live while we run (the ws holds back its first 90s), so a launch had none; one from
  // before the oldest line shown would just sit at the top, so there is none then either. history: no
  // notification, no yellow tab, and the line's id keeps a second answer from doubling it
  pub(super) fn ask_went_live(&mut self, chan: &str) {
    if self.set.ev_online && ident::twitch_login(chan) && self.live_asked.insert(chan.into()) {
      self.net.send(Cmd::StreamInfo(chan.into()));
    }
  }

  pub(super) fn history_went_live(&mut self, chan: &str, info: &Info) {
    let Some((_, start)) = info.live.filter(|&(_, t)| t > 0 && t <= now_ms()) else { return };
    if !self.set.ev_online {
      return;
    }
    let l = Line::went_live(Platform::Twitch, chan, &info.game);
    let mut m = self.stream_row(&l);
    m.ts = start;
    m.id = format!("went-live:{chan}:{start}").into();
    m.flags |= flag::HISTORY;
    for i in self.tabs_with(Platform::Twitch, chan) {
      let v = self.tabs[i].chat();
      if v.oldest_ts().is_some_and(|t| t <= start) {
        v.prepend(vec![m.clone()]);
        if i == self.active {
          self.wake();
        }
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::Net;

  fn line(tone: Tone, game: &str) -> Line {
    Line {
      platform: Platform::Twitch,
      channel: "xqc".into(),
      text: "".into(),
      tone,
      name: "xqc".into(),
      color: None,
      user_id: None,
      verb: "".into(),
      game: game.into(),
    }
  }

  #[test]
  fn going_live_notifies_only_when_away_and_on() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let live = line(Tone::Online, "just chatting");
    assert_eq!(app.stream_alert(&live), None, "off by default");
    app.set.notify = true;
    app.focused = true;
    assert_eq!(app.stream_alert(&live), None, "not while you are looking");
    app.focused = false;
    assert_eq!(app.stream_alert(&live), Some(("xqc went live".into(), "just chatting".into())));
    assert_eq!(app.stream_alert(&line(Tone::Online, "")), Some(("xqc went live".into(), "".into())));
    assert_eq!(app.stream_alert(&line(Tone::Switch, "chess")), Some(("xqc switched game".into(), "chess".into())));
    for t in [Tone::Offline, Tone::Moment, Tone::Raid, Tone::Gift, Tone::Redeem] {
      assert_eq!(app.stream_alert(&line(t, "x")), None);
    }
  }

  #[test]
  fn a_hostile_channel_name_is_escaped_and_bounded() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec![], false);
    app.set.notify = true;
    app.focused = false;
    let mut l = line(Tone::Online, "");
    l.channel = format!("<b>\n{}", "z".repeat(100)).into();
    let (title, _) = app.stream_alert(&l).unwrap();
    assert!(title.starts_with("&lt;b&gt; zzz") && title.len() < 70, "{title}");
  }

  #[test]
  fn a_channel_live_when_its_history_loads_gets_its_went_live_line_in_place() {
    use crate::model::status::Info;
    use crate::net::{NetEvent, irc::Chat};
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let now = now_ms();
    let said = |id: &str, ago: i64| {
      let mut m = Message::system(now - ago, Platform::Twitch, id);
      m.id = id.into();
      Chat::Msg(m)
    };
    app.on_net(NetEvent::Backfill("forsen".into(), vec![said("old", 3 * 3_600_000), said("new", 3_600_000)]));
    let asks: Vec<_> =
      std::iter::from_fn(|| asked.try_recv().ok()).filter(|c| matches!(c, Cmd::StreamInfo(_))).collect();
    assert_eq!(asks.len(), 1, "asked once, when the history landed");
    let live =
      |ago: i64| Info { name: "Forsen".into(), live: Some((900, now - ago)), game: "Chess".into(), ..Info::default() };
    app.on_net(NetEvent::StreamInfo { chan: "forsen".into(), info: Some(live(2 * 3_600_000)) });
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    let rows = |app: &mut App| -> Vec<String> {
      app.tabs[tab]
        .chat()
        .msgs()
        .map(|m| {
          if m.id.starts_with("went-live") {
            m.x.as_ref().map(|x| x.tail.iter().map(|s| s.text.to_string()).collect()).unwrap_or_default()
          } else {
            m.id.to_string()
          }
        })
        .collect()
    };
    assert_eq!(rows(&mut app), ["old", " ◆ went live — Chess", "new"], "at the time it went live");
    // the status screen asking again changes nothing; nor does a second answer
    app.on_net(NetEvent::StreamInfo { chan: "forsen".into(), info: Some(live(2 * 3_600_000)) });
    assert_eq!(rows(&mut app).len(), 3);
    // a go-live from before the oldest line shown would just sit on top: none
    let mut app2 = App::new(Net::tap().0, vec!["forsen".into()], false);
    app2.on_net(NetEvent::Backfill("forsen".into(), vec![said("new", 3_600_000)]));
    app2.on_net(NetEvent::StreamInfo { chan: "forsen".into(), info: Some(live(5 * 3_600_000)) });
    assert_eq!(app2.tabs[tab].chat().msgs().count(), 1);
    // went-live lines off: nothing is asked
    let (net, mut asked) = Net::tap();
    let mut app3 = App::new(net, vec!["forsen".into()], false);
    app3.set.ev_online = false;
    app3.on_net(NetEvent::Backfill("forsen".into(), vec![said("new", 3_600_000)]));
    assert!(!std::iter::from_fn(|| asked.try_recv().ok()).any(|c| matches!(c, Cmd::StreamInfo(_))));
  }

  fn last(app: &App) -> Message {
    app.tabs.iter().find(|t| &*t.name == "forsen").unwrap().view.last_msg().unwrap().clone()
  }

  #[test]
  fn the_name_is_the_lowercase_login() {
    let mut app = App::new(Net::tap().0, vec!["forsen".into()], false);
    app.on_stream(Line::went_live(Platform::Twitch, "XqC", "chess"));
    let m = last(&app);
    assert_eq!((&*m.nick, &*m.login), ("xqc", "xqc"));
    let mut l = line(Tone::Switch, "chess");
    l.name = "XQC".into();
    app.on_stream(l);
    assert_eq!(&*last(&app).nick, "xqc", "a frame's display casing is not used");
  }

  #[test]
  fn a_frame_coloured_row_gains_its_id_and_keeps_its_colour() {
    use crate::model::card::Info;
    let mut app = App::new(Net::tap().0, vec!["forsen".into()], false);
    let mut l = line(Tone::Switch, "chess");
    l.color = Some(0x00ff00);
    app.on_stream(l);
    let m = last(&app);
    assert_eq!((m.color, &*m.user_id), (color::readable(0x00ff00), ""));
    assert!(app.res.chatters.take_wanted().contains(&(Platform::Twitch, "xqc".into())), "the lookup is queued");
    app.learn_card(
      Platform::Twitch,
      "xqc",
      Ok(Info { color: Some(0xff0000), twitch_id: Some(4242), ..Info::default() }),
    );
    let m = last(&app);
    assert_eq!((m.color, &*m.user_id), (color::readable(0x00ff00), "4242"));
  }

  #[test]
  fn a_kick_row_wears_a_kick_paint_id() {
    use crate::cosmetics::id::PaintId;
    use crate::model::card::Info;
    let mut app = App::new(Net::tap().0, vec!["forsen".into()], false);
    let mut l = line(Tone::Switch, "chess");
    l.platform = Platform::Kick;
    app.on_stream(l);
    assert_eq!(PaintId::of_message(&last(&app)), None);
    // a same-named twitch lookup is not theirs
    app.learn_card(Platform::Twitch, "xqc", Ok(Info { twitch_id: Some(5), ..Info::default() }));
    assert_eq!(PaintId::of_message(&last(&app)), None);
    app.learn_card(Platform::Kick, "xqc", Ok(Info { kick_id: Some(77), ..Info::default() }));
    assert_eq!(PaintId::of_message(&last(&app)), Some(PaintId::Kick(77)));
  }

  #[test]
  fn a_frame_user_id_seats_the_paint_at_once() {
    use crate::cosmetics::id::PaintId;
    let mut app = App::new(Net::tap().0, vec!["forsen".into()], false);
    let mut l = line(Tone::Switch, "chess");
    l.user_id = Some("123".into());
    app.on_stream(l);
    assert_eq!(PaintId::of_message(&last(&app)), Some(PaintId::Twitch(123)));
  }

  #[test]
  fn history_replays_switched_and_offline_at_their_times_and_never_online() {
    let mut app = App::new(Net::tap().0, vec!["forsen".into()], false);
    app.set.ev_offline = true;
    let now = now_ms();
    let mut sw = line(Tone::Switch, "chess");
    sw.verb = "◆ switched to ".into();
    let mut off = line(Tone::Offline, "");
    off.verb = "◆ went offline".into();
    let on = line(Tone::Online, "chess");
    app.on_stream_history(vec![(now - 600_000, sw.clone()), (now - 300_000, off), (now - 200_000, on)]);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    let words = |app: &mut App| -> Vec<String> {
      app.tabs[tab]
        .chat()
        .msgs()
        .map(|m| m.x.as_ref().unwrap().tail.iter().map(|s| s.text.to_string()).collect())
        .collect()
    };
    assert_eq!(words(&mut app), [" ◆ switched to chess", " ◆ went offline"]);
    let ts: Vec<i64> = app.tabs[tab].chat().msgs().map(|m| m.ts).collect();
    assert_eq!(ts, [now - 600_000, now - 300_000]);
    assert!(!app.tabs[tab].event, "no tab light");
    // a second replay (a reconnect) and a live twin do not double
    app.on_stream_history(vec![(now - 600_000, sw.clone())]);
    app.on_stream(sw.clone());
    assert_eq!(words(&mut app).len(), 3, "the live line is its own; the replay added nothing");
    app.on_stream_history(vec![(now, sw)]);
    assert_eq!(words(&mut app).len(), 3, "a replay next to a live line of the same words is skipped");
  }
}
