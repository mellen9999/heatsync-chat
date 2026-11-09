// a twitch channel's status (ext /status + chat-modes.js): live or offline with uptime and viewers, the
// title, game and language, and its chat modes. a mod changes a mode from here (enter flips it, l / h
// step slow mode's or followers-only's wait) or by typing /slow 30, /followers 10m, /emoteonly off …
// what shows is what twitch says (ROOMSTATE), so another mod's change lands here too
use super::App;
use super::screen::{Page, Target};
use crate::i18n::{f, t};
use crate::model::message::Platform;
use crate::model::modes::{self, ALL, FOLLOW_STEPS, Mode, Modes, SLOW_DEFAULT, SLOW_STEPS, Set};
use crate::model::stats;
use crate::model::status::Info;
use crate::net::Cmd;
use crate::net::modact::Fail;
use crate::ui::sheet::L;
use crate::util::time::now_ms;

// where h / l takes a mode from `cur`: None = off. the plain switches: l on, h off
fn step(m: Mode, cur: Option<u32>, up: bool) -> Option<u32> {
  let steps: &[u32] = match m {
    Mode::Slow => &SLOW_STEPS,
    Mode::Followers => &FOLLOW_STEPS,
    _ => return up.then_some(0),
  };
  match (cur, up) {
    (None, true) => Some(if m == Mode::Slow { SLOW_DEFAULT } else { 0 }),
    (None, false) => None,
    (Some(v), true) => Some(steps.iter().copied().find(|s| *s > v).unwrap_or(v)),
    (Some(v), false) => steps.iter().rev().copied().find(|s| *s < v),
  }
}

// enter: on (twitch's default wait) or off
fn flip(m: Mode, cur: Option<u32>) -> Option<u32> {
  match cur {
    Some(_) => None,
    None => Some(if m == Mode::Slow { SLOW_DEFAULT } else { 0 }),
  }
}

fn rows(chan: &str, info: Option<&Option<Info>>, modes: Option<&Modes>, can: bool) -> (Vec<L>, Vec<Target>) {
  let row = |a: String| L::Row { a, b: String::new(), pill: None };
  let (mut lines, mut marks) = (Vec::new(), Vec::new());
  let mut put = |l: L, t: Target| {
    lines.push(l);
    marks.push(t);
  };
  match info {
    None => put(row(t("app.status.loading").into()), Target::None),
    Some(None) => put(row(t("app.status.no_info").into()), Target::None),
    Some(Some(i)) => {
      put(
        row(match i.live {
          Some((n, at)) => {
            f("app.status.live", &[("span", &stats::span(now_ms() - at)), ("viewers", &stats::group(n))])
          }
          None => t("app.status.offline").into(),
        }),
        Target::None,
      );
      if !i.title.is_empty() {
        put(row(format!("\"{}\"", i.title)), Target::None);
      }
      let meta: Vec<&str> = [&*i.game, &*i.lang].into_iter().filter(|s| !s.is_empty()).collect();
      if !meta.is_empty() {
        put(row(meta.join(" · ")), Target::None);
      }
    }
  }
  put(L::Head(if can { t("app.status.modes").into() } else { t("app.status.modes_ro").into() }), Target::None);
  match modes {
    None => put(row(f("app.status.joining", &[("chan", chan)])), Target::None),
    Some(ms) => {
      for m in ALL {
        let v = ms.get(m);
        put(L::Row { a: modes::row(m, v), b: String::new(), pill: Some(v.is_some()) }, Target::Mode(m));
      }
    }
  }
  put(L::Head(t("app.status.more").into()), Target::None);
  put(row(t("app.status.summary").into()), Target::Summary);
  if can {
    put(row(t("app.pop.blocked_terms").into()), Target::BlockedTerms);
    put(row(t("app.status.mod_tools").into()), Target::ModTools);
  }
  (lines, marks)
}

impl App {
  // the active tab's twitch channel
  pub(super) fn twitch_here(&self) -> Option<Box<str>> {
    self.tabs.get(self.active)?.sources.iter().find(|s| s.0 == Platform::Twitch).map(|s| s.1.clone())
  }

  // /status (/modes)
  pub(super) fn status_here(&mut self) {
    match self.twitch_here() {
      Some(chan) => self.status_open(chan),
      None => self.say(t("app.status.needs_tab")),
    }
  }

  pub(super) fn status_open(&mut self, chan: Box<str>) {
    self.open_screen(Page::Status);
    if let Some(s) = &mut self.screen {
      s.of = chan.clone();
    }
    self.net.send(Cmd::StreamInfo(chan));
    self.status_refresh();
  }

  pub(super) fn on_stream_info(&mut self, chan: Box<str>, info: Option<Info>) {
    if self.live_asked.remove(&chan)
      && let Some(i) = &info
    {
      self.history_went_live(&chan, i);
    }
    if let Some(s) = self.screen.as_mut().filter(|s| s.page() == Page::Status && s.of == chan) {
      s.info = Some(info);
      self.wake();
    }
  }

  // the screen's lines, read fresh from what twitch last said
  pub(super) fn status_refresh(&mut self) {
    let Some(s) = self.screen.as_ref().filter(|s| s.page() == Page::Status) else { return };
    let can = self.can_mod(Platform::Twitch, &s.of);
    let (lines, marks) = rows(&s.of, s.info.as_ref(), self.modes.get(&s.of), can);
    let title = f("app.status.title", &[("chan", &s.of)]);
    if let Some(s) = &mut self.screen {
      (s.shown, s.marks, s.title) = (lines, marks, title);
    }
  }

  // ROOMSTATE: the room's modes as twitch has them (all on joining, then each change)
  pub(super) fn modes_seen(&mut self, chan: &str, sets: Vec<Set>) {
    self.lock_modes(chan, &sets);
    if sets.is_empty() {
      return;
    }
    let m = self.modes.entry(chan.into()).or_default();
    for s in sets {
      m.put(s);
    }
    if self.screen.as_ref().is_some_and(|s| s.page() == Page::Status) {
      self.wake();
    }
  }

  // a status screen row: a mode (enter flips, Some(up) = l / h), the summary, blocked terms
  pub(super) fn status_pick(&mut self, t: Target, adjust: Option<bool>) {
    let Some(chan) = self.screen.as_ref().map(|s| s.of.clone()) else { return };
    match (t, adjust) {
      (Target::Mode(m), _) => {
        let cur = self.modes.get(&chan).and_then(|ms| ms.get(m));
        let next = adjust.map_or_else(|| flip(m, cur), |up| step(m, cur, up));
        if next != cur {
          self.modes_set(chan, Set::new(m, next));
        }
      }
      (Target::Summary, None) => {
        if let Some(i) = self.tabs_with(Platform::Twitch, &chan).first() {
          let name = self.tabs[*i].name.clone();
          self.summary_open(name);
        }
      }
      (Target::BlockedTerms, None) => self.blocked_open(chan),
      (Target::ModTools, None) => self.tools_open(chan),
      _ => {}
    }
  }

  // /slow 30, /followers off …: the active tab's twitch room
  pub(super) fn modes_slash(&mut self, cmd: Result<Set, String>) {
    match (cmd, self.twitch_here()) {
      (Err(usage), _) => self.say(usage),
      (Ok(_), None) => self.say(t("app.status.modes_need_tab")),
      (Ok(set), Some(chan)) => self.modes_set(chan, set),
    }
  }

  fn modes_set(&mut self, chan: Box<str>, set: Set) {
    if self.me.is_none() {
      return self.say(t("app.login_first_ctrl_l"));
    }
    if !self.can_mod(Platform::Twitch, &chan) {
      return self.say(f("app.status.needs_mod", &[("mode", set.mode.label()), ("chan", &chan)]));
    }
    if self.modes_busy {
      return self.say(t("app.status.busy"));
    }
    self.modes_busy = true;
    self.net.send(Cmd::Modes(chan, set));
  }

  pub(super) fn on_modes_done(&mut self, chan: Box<str>, set: Set, res: Result<(), Fail>) {
    self.modes_busy = false;
    match res {
      Ok(()) => {
        // twitch's ROOMSTATE echo confirms it; shown now so the screen doesn't lag it
        self.modes.entry(chan.clone()).or_default().put(set);
        self.say(f("app.status.done", &[("what", &set.done()), ("chan", &chan)]));
      }
      Err(Fail::Auth) => self.say(t("app.login_first_ctrl_l")),
      Err(Fail::Relink) => {
        self.say(super::feed::needs_twitch("chat modes"));
        self.net.send(Cmd::Relink("mod"));
      }
      Err(Fail::Other(e)) => self.say(f("app.status.failed", &[("mode", set.mode.label()), ("why", &e.to_string())])),
    }
    self.wake();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::irc::Chat;
  use crate::net::{Net, NetEvent};

  #[test]
  fn h_and_l_walk_the_waits() {
    assert_eq!(step(Mode::Slow, None, true), Some(30));
    assert_eq!(step(Mode::Slow, Some(30), true), Some(60));
    assert_eq!(step(Mode::Slow, Some(120), true), Some(120));
    assert_eq!(step(Mode::Slow, Some(3), false), None, "below the least is off");
    assert_eq!(step(Mode::Slow, Some(45), false), Some(30), "a typed wait steps to its neighbour");
    assert_eq!(step(Mode::Followers, Some(0), false), None);
    assert_eq!(step(Mode::Followers, Some(0), true), Some(10));
    assert_eq!((step(Mode::Emote, None, true), step(Mode::Emote, Some(0), false)), (Some(0), None));
    assert_eq!((flip(Mode::Slow, None), flip(Mode::Followers, Some(10))), (Some(30), None));
  }

  fn shown(app: &App) -> Vec<String> {
    let s = app.screen.as_ref().unwrap();
    s.shown
      .iter()
      .map(|l| match l {
        L::Head(h) => format!("# {h}"),
        L::Tint { a, .. } => a.clone(),
        L::Row { a, pill: Some(on), .. } => format!("{a} [{}]", if *on { "on" } else { "off" }),
        L::Row { a, .. } => a.clone(),
      })
      .collect()
  }

  // what the app asked of the net that this screen cares about
  fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<String> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| match c {
        Cmd::StreamInfo(c) => Some(format!("info {c}")),
        Cmd::Modes(c, s) => Some(format!("{c} {}", s.done())),
        Cmd::Relink(p) => Some(format!("relink {p}")),
        _ => None,
      })
      .collect()
  }

  #[test]
  fn status_shows_twitchs_modes_and_a_mod_changes_them() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
    app.status_here();
    assert!(shown(&app).contains(&"not known yet: still joining forsen".to_owned()));
    assert_eq!(asked(&mut rx), ["info forsen"]);
    // joining: every mode; then one change
    let on = |mode, on| Set { mode, on };
    let all = vec![
      on(Mode::Emote, None),
      on(Mode::Subs, None),
      on(Mode::Unique, Some(0)),
      on(Mode::Slow, None),
      on(Mode::Followers, Some(10)),
    ];
    app.on_chat("forsen", Chat::Room { id: "22484632".into(), modes: all });
    app.on_chat("forsen", Chat::Room { id: "22484632".into(), modes: vec![on(Mode::Slow, Some(30))] });
    app.on_net(NetEvent::StreamInfo {
      chan: "forsen".into(),
      info: Some(Info { name: "forsen".into(), live: None, title: "hi".into(), ..Info::default() }),
    });
    app.status_refresh();
    let rows = shown(&app);
    assert_eq!(&rows[..3], ["offline", "\"hi\"", "# chat modes · read only"]);
    assert!(rows.contains(&"followers-only · 10m [on]".to_owned()));
    assert!(rows.contains(&"slow mode · 30s [on]".to_owned()));
    assert!(rows.contains(&"emote-only [off]".to_owned()));
    assert!(!rows.contains(&"blocked terms".to_owned()), "not a mod");
    app.redraw();
    // not a mod: nothing is asked
    app.status_pick(Target::Mode(Mode::Emote), None);
    assert!(asked(&mut rx).is_empty());
    // a mod: enter flips emote-only on, l steps slow mode up
    app.me = Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(1),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
    app.on_chat("forsen", Chat::Mod(true));
    app.status_refresh();
    assert!(shown(&app).contains(&"blocked terms".to_owned()));
    app.status_pick(Target::Mode(Mode::Emote), None);
    assert_eq!(asked(&mut rx), ["forsen emote-only on"]);
    app.status_pick(Target::Mode(Mode::Slow), Some(true));
    assert!(asked(&mut rx).is_empty(), "one change at a time");
    app.on_net(NetEvent::ModesDone { chan: "forsen".into(), set: on(Mode::Emote, Some(0)), res: Ok(()) });
    assert!(app.modes["forsen"].emote);
    app.status_pick(Target::Mode(Mode::Slow), Some(true));
    assert_eq!(asked(&mut rx), ["forsen slow mode on (60s)"]);
    app.on_net(NetEvent::ModesDone { chan: "forsen".into(), set: on(Mode::Slow, Some(60)), res: Err(Fail::Relink) });
    assert_eq!(asked(&mut rx), ["relink mod"]);
    // typed: /slowoff
    app.close_screen();
    app.line.set_text("/slowoff".into());
    app.send();
    assert_eq!(asked(&mut rx), ["forsen slow mode off"]);
  }
}
