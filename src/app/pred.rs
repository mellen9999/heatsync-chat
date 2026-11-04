// a twitch channel's prediction (ext pred-view.js + the chat banner), read only. a one-line banner under
// the tabs while bets are open or locked on the tab you are looking at; click it, `space p`,
// /predictions or the tab menu opens the whole view (each outcome's share as a bar, points, bettors,
// the clock, who won). asking: twitch's public gql every 15s while one is up and a slow look otherwise,
// only for the tab on screen while the window shows, one request at a time; nothing when no twitch tab
// is up, the window is hidden, or `prediction banner` is off (opening the view asks regardless)
use std::time::{Duration, Instant};

use winit::keyboard::Key;

use super::App;
use super::screen::Page;
use crate::model::message::Platform;
use crate::model::pred::{self, Banner, Entry, Pred, State, bar, bettors, points};
use crate::net::Cmd;
use crate::ui::canvas::Rect;
use crate::ui::{pred_bar, sheet::L};
use crate::util::time::now_ms;

// the prediction banner's last paint: its text and clock, the width, line height, and the pin bar above it
type Sig = (Option<(String, String)>, i32, i32, i32);

// predictions per twitch channel, the banner's height (0 = none) and what it last drew, and when its
// clock next reads differently
#[derive(Default)]
pub(super) struct PredBar {
  pub(super) preds: crate::model::pred::Preds,
  pub(super) h: i32,
  pub(super) sig: Sig,
  pub(super) sec: Option<Instant>,
}

impl App {
  // the channel whose prediction is on screen: the open view's, else the active tab's (banner on)
  fn pred_look(&self) -> Option<Box<str>> {
    if let Some(s) = self.screen.as_ref().filter(|s| s.page() == Page::Pred) {
      return Some(s.of.clone());
    }
    if self.set.ev_pred { self.twitch_here() } else { None }
  }

  // …and the one worth asking about: not while the window is hidden
  fn pred_chan(&self) -> Option<Box<str>> {
    self.pred_look().filter(|_| !self.occluded)
  }

  // when the next ask is due (None = nothing to watch, or one is out)
  pub(super) fn pred_due(&self, now: Instant) -> Option<Instant> {
    let chan = self.pred_chan()?;
    self.pred_bar.preds.due(&chan, now)
  }

  pub(super) fn pred_tick(&mut self, now: Instant) {
    let Some(chan) = self.pred_chan() else { return };
    if self.pred_bar.preds.due(&chan, now).is_none_or(|t| t > now) {
      return;
    }
    // a closed tab's channel is forgotten
    let open: Vec<Box<str>> = self.tabs.iter().flat_map(|t| t.sources.iter()).map(|s| s.1.clone()).collect();
    self.pred_bar.preds.keep(|c| c == &*chan || open.iter().any(|o| **o == *c));
    self.pred_bar.preds.asked(&chan, now);
    self.net.send(Cmd::Pred(chan));
  }

  pub(super) fn on_pred(&mut self, chan: Box<str>, res: Result<Option<Pred>, ()>) {
    // a tab closed while it was out
    let open = self.tabs.iter().any(|t| t.sources.iter().any(|s| s.0 == Platform::Twitch && s.1 == chan));
    if !open && self.screen.as_ref().is_none_or(|s| s.page() != Page::Pred || s.of != chan) {
      return;
    }
    self.pred_bar.preds.got(&chan, res);
    self.pred_refresh();
    self.wake();
  }

  // the countdown crossed a second
  pub(super) fn pred_second(&mut self) {
    self.pred_bar.sec = None;
    self.pred_refresh();
    self.wake();
  }

  fn pred_banner(&self, now: i64) -> Option<Banner> {
    if !self.set.ev_pred || self.screen.as_ref().is_some_and(|s| s.page() == Page::Pred) {
      return None;
    }
    let chan = self.twitch_here()?;
    pred::banner(self.pred_bar.preds.get(&chan)?.shown(now)?, now)
  }

  pub(super) fn pred_area(&self) -> Rect {
    let b = self.body();
    Rect::new(b.x, b.y + self.pin_bar.h, b.w, self.pred_bar.h)
  }

  // repaint the banner if what it shows changed (and re-arm the clock); the chat below re-lays out
  pub(super) fn sync_pred(&mut self) -> Option<Rect> {
    let now = now_ms();
    let b = self.pred_banner(now);
    // the clock: wake when what the banner (or the open view) reads next differs
    self.pred_bar.sec = self
      .pred_look()
      .and_then(|c| self.pred_bar.preds.get(&c)?.shown(now)?.tick_in(now))
      .filter(|_| !self.occluded)
      .map(|ms| Instant::now() + Duration::from_millis(ms));
    let sig = (
      b.as_ref().map(|b| (b.body.clone(), b.tail.clone())),
      self.body().w,
      self.res.text.line_px as i32,
      self.pin_bar.h,
    );
    if sig == self.pred_bar.sig {
      return None;
    }
    self.pred_bar.sig = sig;
    let old = self.pred_bar.h;
    self.pred_bar.h = if b.is_some() { self.res.text.line_px as i32 } else { 0 };
    let area = Rect { h: self.pred_bar.h.max(old), ..self.pred_area() };
    self.canvas.fill(area, crate::ui::theme::BLACK);
    let bar = self.pred_area();
    if let Some(b) = &b {
      pred_bar::draw(&mut self.canvas, &mut self.res.text, bar, b);
    }
    if self.pred_bar.h != old
      && let Some(t) = self.tabs.get_mut(self.active)
    {
      t.view.invalidate();
    }
    Some(area)
  }

  // true when the click landed on the banner
  pub(super) fn pred_click(&mut self) -> bool {
    let (x, y) = self.pointer;
    if self.pred_bar.h == 0 || !self.pred_area().contains(x, y) {
      return false;
    }
    self.pred_here();
    true
  }

  // /predictions, space p, the banner
  pub(super) fn pred_here(&mut self) {
    match self.twitch_here() {
      Some(chan) => self.pred_open(chan),
      None => self.say("/predictions needs a twitch channel tab"),
    }
  }

  pub(super) fn pred_open(&mut self, chan: Box<str>) {
    self.open_screen(Page::Pred);
    if let Some(s) = &mut self.screen {
      s.of = chan.clone();
    }
    // what we know now shows at once; a fresh answer follows
    self.pred_bar.preds.expire(&chan);
    self.pred_refresh();
    self.pred_tick(Instant::now());
  }

  // the view's lines, read fresh from what twitch last said
  pub(super) fn pred_refresh(&mut self) {
    let Some(s) = self.screen.as_ref().filter(|s| s.page() == Page::Pred) else { return };
    let (title, lines) = (format!("predictions · {}", s.of), lines(self.pred_bar.preds.get(&s.of), now_ms()));
    if let Some(s) = &mut self.screen {
      (s.shown, s.title) = (lines, title);
      let (l, _) = s.lines(&self.set);
      s.st.settle(&l);
    }
  }

  // r: ask now
  pub(super) fn pred_key(&mut self, key: &Key<&str>) -> bool {
    if self.mods.control_key() || !matches!(key, Key::Character("r")) {
      return false;
    }
    if let Some(chan) = self.screen.as_ref().map(|s| s.of.clone()) {
      self.pred_bar.preds.expire(&chan);
      self.pred_tick(Instant::now());
    }
    true
  }
}

// the view: the question and how it stands, then each outcome's name, share and bar
fn lines(e: Option<&Entry>, now: i64) -> Vec<L> {
  let row = |a: &str| L::Row { a: a.into(), b: String::new(), pill: None };
  let Some(pred) = e.and_then(|e| e.pred.as_ref()) else {
    let msg = if e.is_some_and(Entry::failed) { "couldn't reach twitch: trying again" } else { "loading…" };
    return vec![row(msg)];
  };
  let mut out = Vec::new();
  let Some(p) = pred.as_ref().filter(|p| p.fresh(now)) else {
    out.push(row("no prediction right now"));
    out.push(row("one shows here while bets are open or locked, and for 5 minutes after it ends"));
    return out;
  };
  let state = p.when(now);
  out.push(L::Row { a: p.title.to_string(), b: state, pill: None });
  out.push(L::Row { a: format!("{} pts · {}", points(p.points()), bettors(p.users())), b: String::new(), pill: None });
  for (i, o) in p.outcomes.iter().enumerate() {
    let win = if o.won { " ✓ winner" } else { "" };
    out.push(L::Tint { a: format!("{}{win}", o.title), b: format!("{}%", p.pct(i)), ink: o.ink });
    out.push(L::Tint {
      a: format!("  {} {} pts · {}", bar(p.share(i, pred::BAR as u64)), points(o.points), bettors(o.users)),
      b: String::new(),
      ink: o.ink,
    });
  }
  if e.is_some_and(Entry::failed) {
    out.push(row("couldn't reach twitch: this may be old, trying again"));
  }
  if p.state == State::Canceled {
    out.push(row("canceled: points were refunded"));
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::Net;
  use serde_json::json;

  fn app() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    let (net, rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into(), "kick:xqc".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    (app, rx)
  }

  fn pending(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> usize {
    std::iter::from_fn(|| rx.try_recv().ok()).filter(|c| matches!(c, Cmd::Pred(_))).count()
  }

  fn iso(ms: i64) -> String {
    format!("{}T{}Z", crate::util::time::utc_day(ms), &crate::util::time::utc_stamp(ms)[6..])
  }

  // twitch's answer for a prediction in `status`, opened `ago` ms back
  fn answer(status: &str, ago: i64) -> Option<Pred> {
    let o = |id, t, p, u, c| json!({"id": id, "title": t, "totalPoints": p, "totalUsers": u, "color": c});
    let e = json!({"id": "e1", "title": "boss down?", "status": status, "createdAt": iso(now_ms() - ago),
      "predictionWindowSeconds": 120, "outcomes": [o("a", "yes", 3000, 4, "BLUE"), o("b", "no", 1000, 1, "PINK")]});
    let v = json!({"data": {"user": {"channel": {"activePredictionEvents": [e], "lockedPredictionEvents": [],
      "resolvedPredictionEvents": {"edges": []}}}}});
    pred::parse(&v, now_ms()).unwrap()
  }

  fn go(app: &mut App, name: &str) {
    let tab = app.tabs.iter().position(|t| t.name.contains(name)).unwrap();
    app.select(tab);
  }

  fn shown(app: &App) -> Vec<String> {
    app
      .screen
      .as_ref()
      .unwrap()
      .shown
      .iter()
      .map(|l| match l {
        L::Head(h) => format!("# {h}"),
        L::Row { a, b, .. } | L::Tint { a, b, .. } if b.is_empty() => a.clone(),
        L::Row { a, b, .. } | L::Tint { a, b, .. } => format!("{a} | {b}"),
      })
      .collect()
  }

  #[test]
  fn nothing_is_asked_when_nothing_is_watched() {
    let (mut app, mut rx) = app();
    let now = Instant::now();
    // the kick tab: no twitch channel, no ask, no wake
    go(&mut app, "xqc");
    assert!(app.pred_due(now).is_none());
    app.pred_tick(now);
    assert_eq!(pending(&mut rx), 0);
    // a twitch tab: asked once, then not again while it is out
    go(&mut app, "forsen");
    assert!(app.pred_due(now).is_some_and(|t| t <= now));
    app.pred_tick(now);
    assert_eq!(pending(&mut rx), 1);
    assert!(app.pred_due(now).is_none());
    app.pred_tick(now);
    assert_eq!(pending(&mut rx), 0, "one in flight per channel");
    // nothing live: the slow look, 45s on
    app.on_pred("forsen".into(), Ok(None));
    assert_eq!(app.pred_due(now), Some(now + pred::IDLE_EVERY));
    // the window hidden: nothing to wait for
    app.occluded = true;
    assert!(app.pred_due(now).is_none());
    app.occluded = false;
    // the banner off: the same
    app.set.ev_pred = false;
    assert!(app.pred_due(now).is_none());
  }

  #[test]
  fn a_live_prediction_shows_a_banner_that_opens_the_view() {
    let (mut app, mut rx) = app();
    go(&mut app, "forsen");
    let now = Instant::now();
    app.pred_tick(now);
    pending(&mut rx);
    app.on_pred("forsen".into(), Ok(answer("ACTIVE", 30_000)));
    assert_eq!(app.pred_due(now), Some(now + pred::LIVE_EVERY), "15s while one is up");
    app.redraw();
    assert_eq!(app.pred_bar.h, app.res.text.line_px as i32, "the banner takes a row");
    let b = app.pred_banner(now_ms()).unwrap();
    assert!(b.body.starts_with("boss down? · yes 75% vs no 25%") && b.tail.ends_with('s') || b.tail.contains(':'));
    assert!(app.pred_bar.sec.is_some(), "the clock is running");
    // locked: no clock, still a banner
    app.on_pred("forsen".into(), Ok(answer("LOCKED", 130_000)));
    app.redraw();
    assert_eq!(app.pred_banner(now_ms()).unwrap().tail, "locked");
    assert!(app.pred_bar.sec.is_none());
    // a click on the banner opens the view
    let a = app.pred_area();
    app.pointer = (a.x + 5, a.y + 2);
    assert!(app.pred_click());
    let s = app.screen.as_ref().unwrap();
    assert_eq!((s.page(), &*s.of), (Page::Pred, "forsen"));
    assert_eq!(pending(&mut rx), 1, "opening asks for a fresh answer");
    let rows = shown(&app);
    assert_eq!(rows[0], "boss down? | locked");
    assert_eq!(rows[1], "4K pts · 5 bettors");
    assert_eq!(rows[2], "yes | 75%");
    assert!(rows[3].starts_with("  ███████████████░░░░░ 3K pts · 4 bettors"), "{}", rows[3]);
    assert_eq!(rows[4], "no | 25%");
    app.redraw();
    assert_eq!(app.pred_bar.h, 0, "the view replaces the banner");
    // r asks again, esc closes
    app.on_pred("forsen".into(), Ok(None));
    assert!(app.pred_key(&Key::Character("r")));
    assert_eq!(pending(&mut rx), 1);
    assert_eq!(shown(&app)[0], "no prediction right now");
    app.close_screen();
    app.redraw();
    assert_eq!(app.pred_bar.h, 0, "nothing live, no banner");
  }

  #[test]
  fn an_ended_prediction_names_its_winner_and_has_no_banner() {
    let (mut app, _rx) = app();
    go(&mut app, "forsen");
    let mut p = answer("RESOLVED", 60_000).unwrap();
    p.outcomes[1].won = true;
    app.on_pred("forsen".into(), Ok(Some(p)));
    app.redraw();
    assert_eq!(app.pred_bar.h, 0);
    app.pred_here();
    let rows = shown(&app);
    assert_eq!(rows[0], "boss down? | ended");
    assert_eq!(rows[4], "no ✓ winner | 25%");
    app.redraw();
  }

  #[test]
  fn a_closed_tab_is_not_kept_and_the_view_needs_twitch() {
    let (mut app, mut rx) = app();
    go(&mut app, "xqc");
    app.pred_here();
    assert!(app.screen.is_none());
    // an answer for a channel no tab holds is dropped
    app.on_pred("gone".into(), Ok(answer("ACTIVE", 0)));
    assert!(app.pred_bar.preds.get("gone").is_none());
    assert_eq!(pending(&mut rx), 0);
  }
}
