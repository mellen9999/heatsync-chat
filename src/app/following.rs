// who you follow is live (the ext's live-followed snapshot): a list screen over the chat from ctrl-b,
// /following or /live. a row is the person, their game, viewers and how long they have been live; enter
// opens them as a chat tab (or goes to the one you have), w plays the stream, r asks again, / filters.
// the list is kept by the socket (catch-up on sign-in, then every go-live, game switch and sign-off) and
// asked in full every two minutes while signed in, like the ext's poll, and when the screen opens; the
// uptime moves once a minute, only while the screen shows. the poll is also how a kick or youtube go-live
// is ever seen (heatsync pushes twitch's): a channel that wasn't live at the last ask and is now gets its
// went-live line (ext announceWentLive). the first answer after sign-in is only the baseline
use std::collections::HashSet;
use std::time::{Duration, Instant};

use winit::keyboard::Key;

use super::App;
use super::screen::{Page, Target};
use crate::model::chan::Spec;
use crate::model::following::{self, Change, Follows, Live};
use crate::model::message::Platform;
use crate::model::stream_events::Line;
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
// the first background ask after sign-in: the baseline, soon, so a go-live after it is caught
const FIRST: Duration = Duration::from_secs(10);

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
  // when the background poll next asks (None = signed out)
  bg: Option<Instant>,
  // every (platform, channel) live at the last answer; None = no answer yet, the next is the baseline
  seen: Option<HashSet<(Platform, Box<str>)>>,
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
    let screen = self.following.tick.filter(|_| self.screen_is(Page::Following) && !self.occluded);
    let bg = self.following.bg.filter(|_| self.me.is_some());
    screen.into_iter().chain(bg).min()
  }

  pub(super) fn following_tick(&mut self, now: Instant) {
    let stale = self.following.fetched.is_none_or(|t| now.duration_since(t) >= REFRESH);
    if self.following.bg.is_some_and(|t| t <= now) {
      self.following.bg = Some(now + REFRESH);
      if stale {
        self.following_ask();
      }
    }
    if self.following.tick.is_some_and(|t| t <= now) && self.screen_is(Page::Following) {
      self.following.tick = Some(now + MINUTE);
      if stale {
        self.following_ask();
      }
      self.wake();
    }
  }

  // a different login starts with nobody
  pub(super) fn following_reset(&mut self) {
    self.following = Default::default();
  }

  // signed in: the background poll starts (its first answer is the baseline)
  pub(super) fn following_arm(&mut self) {
    if self.following.bg.is_none() {
      self.following.bg = Some(Instant::now() + FIRST);
    }
  }

  // the channels live now that weren't at the last answer, as went-live lines. a channel you have a tab
  // for is left to its room's own frames (ext: the follow path skips config.channels)
  fn went_live(&mut self, rows: &[Live]) -> Vec<Line> {
    let now: HashSet<(Platform, Box<str>)> = rows.iter().flat_map(|l| l.legs.iter().cloned()).collect();
    let Some(prev) = self.following.seen.replace(now) else { return Vec::new() };
    rows
      .iter()
      .flat_map(|l| l.legs.iter().map(move |leg| (l, leg)))
      .filter(|(_, leg)| !prev.contains(*leg))
      .filter(|(_, (p, k))| self.tabs_with(*p, k).is_empty())
      .map(|(l, (p, k))| {
        let name = if l.name.eq_ignore_ascii_case(k) { &*l.name } else { &**k };
        Line::went_live(*p, k, name, &l.game)
      })
      .collect()
  }

  pub(super) fn on_following(&mut self, res: Result<Vec<Live>, Fail>) {
    self.following.asked = None;
    match res {
      Ok(rows) => {
        for l in self.went_live(&rows) {
          self.on_stream(l);
        }
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

  // who the forsen tab's stream lines are about (a stream line is a row with no id)
  fn told(app: &App) -> Vec<String> {
    let t = app.tabs.iter().find(|t| &*t.name == "forsen").expect("the forsen tab");
    t.view.msgs().filter(|m| m.id.is_empty()).map(|m| m.login.to_string()).collect()
  }

  #[test]
  fn the_background_poll_announces_who_came_live_since_the_last_answer() {
    let (mut app, mut asked) = app();
    // signed in: the poll runs with the screen closed, first soon, then every two minutes
    app.following_arm();
    let due = app.following_due().expect("armed");
    assert!(due <= Instant::now() + FIRST);
    app.following_tick(due);
    assert_eq!(asks(&mut asked), 1);
    assert!(app.following_due().is_some_and(|t| t >= due + REFRESH - Duration::from_secs(1)));
    // the first answer is the baseline: nobody already live is announced
    app.on_net(NetEvent::Following(Ok(streams())));
    assert!(told(&app).is_empty(), "{:?}", told(&app));
    // a kick channel comes live (no push for kick: the poll is the only word), and forsen, who has a tab
    let more = following::parse(Some(&json!([
      {"platform": "kick", "username": "newk", "displayName": "NewK", "category": "Slots", "viewerCount": 5},
      {"platform": "twitch", "username": "forsen", "category": "Chess", "viewerCount": 9000},
    ])));
    let mut rows = streams();
    rows.extend(more.clone());
    app.on_net(NetEvent::Following(Ok(rows.clone())));
    let t = told(&app);
    assert_eq!(t.len(), 1, "newk only; forsen's own room says his: {t:?}");
    assert_eq!(t[0], "newk");
    // still live at the next answer: nothing more
    app.on_net(NetEvent::Following(Ok(rows.clone())));
    assert_eq!(told(&app).len(), 1);
    // the socket says it too (twitch push after the poll for a twitch leg): one line per go-live
    app.on_net(NetEvent::Following(Ok(streams())));
    let mut back = streams();
    back.extend(more);
    app.on_net(NetEvent::Following(Ok(back)));
    assert_eq!(told(&app).len(), 1, "newk dropped and came back without signing off: still one");
    app.on_stream(Line::went_live(Platform::Kick, "newk", "NewK", ""));
    assert_eq!(told(&app).len(), 1, "the push after the poll: no double");
    // after a sign-off the next go-live is news again
    let mut off = Line::went_live(Platform::Kick, "newk", "NewK", "");
    off.tone = crate::model::stream_events::Tone::Offline;
    app.on_stream(off);
    app.on_stream(Line::went_live(Platform::Kick, "newk", "NewK", ""));
    assert_eq!(told(&app).len(), 2);
  }
}
