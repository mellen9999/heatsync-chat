// who you follow is live (the ext's live-followed snapshot): a list screen over the chat from ctrl-b,
// /following or /live. a row is the person, their game, viewers and how long they have been live; enter
// opens them as a chat tab (or goes to the one you have), w plays the stream, r asks again, / filters.
// the list is kept by the socket (catch-up on sign-in, then every go-live, game switch and sign-off) and
// asked in full when the screen opens and every two minutes while it is up, like the ext's poll; the
// uptime moves once a minute, only while the screen shows. nothing runs while it is closed
use std::time::{Duration, Instant};

use winit::keyboard::Key;

use super::App;
use super::screen::{Page, Target};
use crate::model::chan::Spec;
use crate::model::following::{self, Change, Follows, Live};
use crate::model::message::Platform;
use crate::model::video;
use crate::net::Cmd;
use crate::net::following::Fail;
use crate::ui::sheet::L;
use crate::util::time::now_ms;

// the ext's poll gap, and how often the uptime is read again
const REFRESH: Duration = Duration::from_secs(120);
const MINUTE: Duration = Duration::from_secs(60);
// an ask that never came back is forgotten after this
const LOST: Duration = Duration::from_secs(20);

#[derive(Default)]
pub(super) struct Following {
  list: Follows,
  // an answer or the socket's catch-up has landed
  loaded: bool,
  failed: Option<Fail>,
  asked: Option<Instant>,
  fetched: Option<Instant>,
  // when the open screen next reads the clock
  tick: Option<Instant>,
}

// why the screen has no rows to show, in plain words
fn nothing(me: bool, loaded: bool, failed: Option<Fail>, q: &str) -> &'static str {
  match (me, failed) {
    (false, _) => "log in to see who you follow (ctrl-l)",
    (_, Some(Fail::Auth)) => "sign in to heatsync again (ctrl-l)",
    (_, Some(_)) if !loaded => "couldn't reach heatsync: r tries again",
    _ if !loaded => "loading…",
    _ if !q.is_empty() => "nothing matches",
    _ => "nobody you follow is live right now",
  }
}

// the screen's rows: one per live person that the filter keeps, each pointing at its place in the list
fn rows(f: &Following, me: bool, q: &str, now: i64) -> (Vec<L>, Vec<Target>) {
  let q = q.trim().to_lowercase();
  let shown: Vec<(usize, &Live)> =
    f.list.rows().iter().enumerate().filter(|(_, l)| me && q.split_whitespace().all(|w| l.hay().contains(w))).collect();
  if shown.is_empty() {
    let say = nothing(me, f.loaded, f.failed, &q);
    return (vec![L::Row { a: say.into(), b: String::new(), pill: None }], vec![Target::None]);
  }
  let lines = shown
    .iter()
    .map(|(_, l)| L::Row { a: format!("{} {}", following::tag(l), l.name), b: following::detail(l, now), pill: None })
    .collect();
  (lines, shown.iter().map(|(i, _)| Target::Live(*i)).collect())
}

impl Following {
  // the screen's heading: how many are live, and how many of them the filter keeps
  pub(super) fn title(&self, me: bool, filtering: bool, shown: usize) -> String {
    let n = self.list.rows().len();
    match (me && n > 0, filtering) {
      (false, _) => "following".into(),
      (true, false) => format!("following · {n} live"),
      (true, true) => format!("following · {shown} of {n} live"),
    }
  }

  // the dim line under the list: the picked stream's title, or why the list may be old
  pub(super) fn tip(&self, picked: Option<usize>) -> &str {
    if let Some(l) = picked.and_then(|i| self.list.rows().get(i)) {
      return &l.title;
    }
    match self.failed {
      Some(_) if self.loaded => "couldn't refresh: this is the last list. r tries again",
      _ => "",
    }
  }
}

impl App {
  // ctrl-b, /following, /live
  pub(super) fn toggle_following(&mut self) {
    if self.screen_is(Page::Following) {
      return self.close_screen();
    }
    self.open_screen(Page::Following);
    self.following.tick = Some(Instant::now() + MINUTE);
    self.following_ask();
    self.following_refresh();
  }

  // ask for the full list (not while one ask is out)
  pub(super) fn following_ask(&mut self) {
    let now = Instant::now();
    if self.me.is_none() || self.following.asked.is_some_and(|t| now.duration_since(t) < LOST) {
      return;
    }
    self.following.asked = Some(now);
    self.net.send(Cmd::Following);
  }

  // when the open screen next wants a look (None = it isn't showing)
  pub(super) fn following_due(&self) -> Option<Instant> {
    self.following.tick.filter(|_| self.screen_is(Page::Following) && !self.occluded)
  }

  pub(super) fn following_tick(&mut self, now: Instant) {
    self.following.tick = Some(now + MINUTE);
    if self.following.fetched.is_none_or(|t| now.duration_since(t) >= REFRESH) {
      self.following_ask();
    }
    self.wake();
  }

  // a different login starts with nobody
  pub(super) fn following_reset(&mut self) {
    self.following = Default::default();
  }

  pub(super) fn on_following(&mut self, res: Result<Vec<Live>, Fail>) {
    self.following.asked = None;
    match res {
      Ok(rows) => {
        self.following.list.replace(rows);
        (self.following.loaded, self.following.failed) = (true, None);
        self.following.fetched = Some(Instant::now());
      }
      // the list we have stays: the screen says it could not be refreshed
      Err(f) => self.following.failed = Some(f),
    }
    self.following_changed();
  }

  pub(super) fn on_follow_snapshot(&mut self, rows: Vec<Live>) {
    self.following.list.union(rows);
    self.following.loaded = true;
    self.following_changed();
  }

  pub(super) fn on_follow_change(&mut self, c: Change) {
    if self.following.list.change(c) {
      self.following_changed();
    }
  }

  fn following_changed(&mut self) {
    if self.screen_is(Page::Following) {
      self.following_refresh();
      self.wake();
    }
  }

  // the screen's rows, read fresh from the list
  pub(super) fn following_refresh(&mut self) {
    let Some(s) = self.screen.as_ref().filter(|s| s.page() == Page::Following) else { return };
    let (lines, marks) = rows(&self.following, self.me.is_some(), &s.q.text, now_ms());
    if let Some(s) = &mut self.screen {
      (s.shown, s.marks) = (lines, marks);
      let (l, _) = s.lines(&self.set);
      s.st.settle(&l);
    }
  }

  // r asks now, w plays the picked stream. false = not mine (the screen's own keys, or a filter being typed)
  pub(super) fn following_key(&mut self, key: &Key<&str>) -> bool {
    if self.mods.control_key() || self.screen.as_ref().is_some_and(|s| s.typing()) {
      return false;
    }
    match key {
      Key::Character("r") => {
        self.following.asked = None;
        self.following_ask();
        true
      }
      Key::Character("w") => {
        if let Some(Target::Live(i)) = self.screen.as_ref().and_then(|s| s.target()) {
          self.following_watch(i);
        }
        true
      }
      _ => false,
    }
  }

  // the person at `i`: the screen steps aside and the stream plays
  fn following_watch(&mut self, i: usize) {
    let Some(l) = self.following.list.rows().get(i).cloned() else { return };
    let (p, key) = l.primary();
    let Some(url) = video::stream(*p, key) else { return self.say("no stream to watch there") };
    self.close_screen();
    self.say(format!("watching {}", l.name));
    self.play(&url);
  }

  // enter or a click on a row: a chat tab for them, or the one you already have
  pub(super) fn following_pick(&mut self, i: usize) {
    let Some(l) = self.following.list.rows().get(i).cloned() else { return };
    self.close_screen();
    let has = |t: &super::Tab| t.sources.iter().any(|s| l.legs.iter().any(|g| g.0 == s.0 && g.1 == s.1));
    if let Some(t) = self.tabs.iter().position(has) {
      return if self.visible(t) {
        self.select(t)
      } else {
        self.say(format!("the {} tab is hidden: /show {}", l.name, l.primary().1))
      };
    }
    let key = |p: Platform| l.legs.iter().find(|g| g.0 == p).map_or("", |g| &*g.1);
    let spec = Spec::parse(key(Platform::Twitch), key(Platform::Kick), key(Platform::Youtube));
    match spec.and_then(|s| s.check(&self.other_specs(None)).map(|_| s)) {
      Ok(s) => self.add_tab(&s),
      Err(e) => self.say(e.msg()),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::NetEvent;
  use serde_json::json;

  fn app() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    let (net, asked) = crate::net::Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    app.me = Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: None,
      twitch_id: None,
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
    (app, asked)
  }

  fn streams() -> Vec<Live> {
    following::parse(Some(&json!([
      {"platform": "twitch", "username": "small", "displayName": "Small", "category": "Chess", "title": "late game", "viewerCount": 12},
      {"platform": "twitch", "username": "big", "displayName": "Big", "category": "Just Chatting", "title": "hello", "viewerCount": 4200,
       "startedAt": "2026-11-04T10:00:00.000Z", "platformUsernames": {"twitch": "big", "kick": "big-k"}},
      {"platform": "youtube", "username": "yt_person", "displayName": "Yt", "viewerCount": 80},
    ])))
  }

  fn asks(asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> usize {
    std::iter::from_fn(|| asked.try_recv().ok()).filter(|c| matches!(c, Cmd::Following)).count()
  }

  #[test]
  fn the_screen_asks_once_shows_the_biggest_first_and_follows_the_socket() {
    let (mut app, mut asked) = app();
    app.toggle_following();
    assert!(app.screen_is(Page::Following));
    assert_eq!(asks(&mut asked), 1);
    app.toggle_following();
    app.toggle_following();
    assert_eq!(asks(&mut asked), 0, "an ask already out is not asked twice");
    app.on_net(NetEvent::Following(Ok(streams())));
    let (lines, marks) = app.screen.as_ref().map(|s| (s.shown.clone(), s.marks.clone())).unwrap();
    let first = |l: &L| if let L::Row { a, b, .. } = l { (a.clone(), b.clone()) } else { Default::default() };
    assert_eq!(first(&lines[0]).0, "[TK] Big");
    assert!(first(&lines[0]).1.starts_with("Just Chatting · 4.2k · "), "{:?}", first(&lines[0]));
    assert_eq!(first(&lines[1]).0, "[Y] Yt");
    assert_eq!(first(&lines[2]), ("[T] Small".into(), "Chess · 12".into()));
    assert!(marks.iter().all(|t| matches!(t, Target::Live(_))));
    // the socket: a sign-off drops a row, a go-live adds one, a filter keeps the matches
    let off = json!({"platform": "twitch", "channel": "small"});
    app.on_net(NetEvent::FollowChange(Change::from_frame(crate::model::stream_events::Kind::Offline, &off).unwrap()));
    assert_eq!(app.screen.as_ref().unwrap().shown.len(), 2);
    let (lines, _) = rows(&app.following, true, "chat", 0);
    assert_eq!(lines.len(), 1, "the filter reads game and title too");
    assert_eq!(rows(&app.following, true, "zzz", 0).0.len(), 1, "one row that says nothing matches");
  }

  #[test]
  fn enter_opens_them_as_a_tab_with_every_leg_and_a_second_time_goes_to_it() {
    let (mut app, _asked) = app();
    app.toggle_following();
    app.on_net(NetEvent::Following(Ok(streams())));
    let n = app.tabs.len();
    app.following_pick(0);
    assert!(app.screen.is_none(), "the list steps aside");
    assert_eq!(app.tabs.len(), n + 1);
    let t = app.tabs.last().unwrap();
    let legs: Vec<_> = t.sources.iter().map(|s| (s.0.name(), &*s.1)).collect();
    assert_eq!(legs, [("twitch", "big"), ("kick", "big-k")], "one merged tab for a simulcaster");
    assert_eq!(app.active, n, "and you are in it");
    // again: no second tab, the one you have
    app.select(0);
    app.toggle_following();
    app.following_pick(0);
    assert_eq!((app.tabs.len(), app.active), (n + 1, n));
    // a youtube person is a tab by handle
    app.toggle_following();
    app.following_pick(1);
    assert_eq!(&*app.tabs.last().unwrap().sources[0].1, "@yt_person");
    // an open tab for one leg is where a simulcaster row goes, nothing new
    app.toggle_following();
    app.on_net(NetEvent::Following(Ok(following::parse(Some(&json!([
      {"platform": "twitch", "username": "forsen", "platformUsernames": {"kick": "forsen-k"}}
    ]))))));
    let tabs = app.tabs.len();
    app.following_pick(0);
    assert_eq!(app.tabs.len(), tabs);
    assert_eq!(&*app.tabs[app.active].name, "forsen");
  }

  #[test]
  fn w_plays_the_stream_r_asks_again_and_a_filter_keeps_its_letters() {
    use winit::keyboard::Key;
    let (mut app, mut asked) = app();
    app.toggle_following();
    app.on_net(NetEvent::Following(Ok(streams())));
    asks(&mut asked);
    app.screen_key(Key::Character("r"), Some("r"));
    assert_eq!(asks(&mut asked), 1);
    app.on_net(NetEvent::Following(Err(Fail::Other)));
    assert_eq!(app.screen.as_ref().unwrap().shown.len(), 3, "a failed ask keeps the list");
    assert!(app.following.tip(None).contains("couldn't refresh"));
    // typing a filter: r and w are letters, not commands
    app.screen_key(Key::Character("/"), Some("/"));
    app.screen_key(Key::Character("w"), Some("w"));
    app.screen_key(Key::Character("r"), Some("r"));
    assert_eq!(asks(&mut asked), 0);
    assert_eq!(app.screen.as_ref().unwrap().q.text, "wr");
    // the first row's stream goes to the player, the list steps aside
    app.screen_key(Key::Named(winit::keyboard::NamedKey::Escape), None);
    app.screen.as_mut().unwrap().q.set_text(String::new());
    app.following_refresh();
    app.screen_key(Key::Character("w"), Some("w"));
    let played: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| if let Cmd::Play(u) = c { Some(u) } else { None })
      .collect();
    assert_eq!(played, ["https://www.twitch.tv/big".into()] as [Box<str>; 1]);
    assert!(app.screen.is_none());
  }

  #[test]
  fn logged_out_and_empty_say_why_and_the_clock_only_runs_while_it_shows() {
    let (mut app, mut asked) = app();
    app.me = None;
    app.toggle_following();
    assert_eq!(asks(&mut asked), 0, "nobody to ask for");
    let (lines, _) = rows(&app.following, false, "", 0);
    assert!(matches!(&lines[0], L::Row { a, .. } if a.contains("ctrl-l")));
    app.toggle_following();
    assert!(app.following_due().is_none(), "closed: nothing wakes");
    app.me = app_me();
    app.toggle_following();
    assert!(app.following_due().is_some());
    app.on_net(NetEvent::Following(Ok(Vec::new())));
    assert!(matches!(&app.screen.as_ref().unwrap().shown[0], L::Row { a, .. } if a.contains("nobody")));
    // a tick inside the two minutes only moves the uptime; past it, it asks again
    asks(&mut asked);
    let now = Instant::now();
    app.following_tick(now);
    assert_eq!(asks(&mut asked), 0);
    app.following_tick(now + REFRESH + MINUTE);
    assert_eq!(asks(&mut asked), 1);
    // a different login starts empty
    app.on_net(NetEvent::FollowSnapshot(streams()));
    app.following_reset();
    assert!(app.following.list.rows().is_empty() && !app.following.loaded);
  }

  fn app_me() -> Option<crate::net::auth::Me> {
    app().0.me
  }

  #[test]
  fn the_picked_row_draws_reversed_and_a_row_shows_its_title_underneath() {
    let (mut app, _asked) = app();
    app.toggle_following();
    app.on_net(NetEvent::Following(Ok(streams())));
    app.redraw();
    let white = crate::ui::theme::WHITE;
    assert!(app.canvas.inked(white) > 400, "the picked row is a white bar");
    assert_eq!(app.following.tip(Some(0)), "hello");
  }
}
