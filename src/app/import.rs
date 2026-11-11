// import your twitch follows (the ext's "import follows", channel-mgmt.js renderFollowImportPicker): a list
// screen over the chat from /import. the channels you follow, live ones first then a to z, each with a mark
// ([x] picked, [ ] not); the live ones start picked. space picks, enter (or the add selected row) makes one tab
// each through the path /join uses, s picks just the live ones, r reads again, / filters. channels you
// already have a tab for are left out. twitch only: kick follows live in the browser's kick session
use std::collections::HashSet;
use std::time::{Duration, Instant};

use winit::keyboard::{Key, NamedKey};

use super::App;
use super::pop::Act;
use super::screen::{Page, Target};
use crate::i18n::{f, t, tn};
use crate::model::chan::Spec;
use crate::model::import::Follow;
use crate::net::Cmd;
use crate::net::auth::Provider;
use crate::net::import::{Fail, Got};
use crate::ui::sheet::L;
use crate::ui::theme;

// an ask that never came back is forgotten after this
const LOST: Duration = Duration::from_secs(35);

// what picking a row does
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
  // a follow (its place in the list): picks it, or drops the pick
  Row(usize),
  Add,
  Live,
  SignIn,
  Relink,
  Retry,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Phase {
  #[default]
  Reading,
  Failed(Fail),
  Ready,
}

#[derive(Default)]
pub(super) struct Import {
  phase: Phase,
  // the follows to choose from: live first, then a to z, minus the channels already open
  rows: Vec<Follow>,
  // how many follows twitch listed, open ones included
  all: usize,
  truncated: bool,
  picked: HashSet<Box<str>>,
  asked: Option<Instant>,
  // the last thing said, under the list
  pub msg: String,
}

fn say(a: &str) -> L {
  L::Row { a: a.into(), b: String::new(), pill: None }
}

impl Import {
  pub(super) fn title(&self) -> String {
    if self.phase != Phase::Ready || self.rows.is_empty() {
      return t("settings.import.title").into();
    }
    if self.truncated {
      return f("settings.import.title_cut", &[("n", &self.all.to_string())]);
    }
    let live = self.rows.iter().filter(|r| r.live).count();
    f("settings.import.title_ready", &[("n", &self.rows.len().to_string()), ("live", &live.to_string())])
  }

  fn count(&self) -> usize {
    self.rows.iter().filter(|r| self.picked.contains(&r.login)).count()
  }

  // the screen's rows: why there are none, or the two buttons and a row per follow the filter keeps
  fn lines(&self, me: bool, q: &str) -> (Vec<L>, Vec<Target>) {
    let imp = |s| Target::Imp(s);
    let (l, tg): (Vec<L>, Vec<Target>) = match (me, self.phase) {
      (false, _) | (_, Phase::Failed(Fail::Login)) => (
        vec![say(t("settings.import.login")), say(t("settings.import.login_btn"))],
        vec![Target::None, imp(Step::SignIn)],
      ),
      (_, Phase::Reading) => (vec![say(t("settings.import.reading"))], vec![Target::None]),
      (_, Phase::Failed(Fail::Expired)) => (
        vec![say(t("settings.import.expired")), say(t("settings.import.reconnect"))],
        vec![Target::None, imp(Step::Relink)],
      ),
      (_, Phase::Failed(Fail::Scope)) => (
        vec![say(t("settings.import.scope")), say(t("settings.import.reconnect"))],
        vec![Target::None, imp(Step::Relink)],
      ),
      (_, Phase::Failed(Fail::Down)) => {
        (vec![say(t("settings.import.down")), say(t("settings.import.retry"))], vec![Target::None, imp(Step::Retry)])
      }
      (_, Phase::Ready) if self.all == 0 => (vec![say(t("settings.import.none"))], vec![Target::None]),
      (_, Phase::Ready) if self.rows.is_empty() => (vec![say(t("settings.import.all_here"))], vec![Target::None]),
      (_, Phase::Ready) => return self.list(q),
    };
    (l, tg)
  }

  fn list(&self, q: &str) -> (Vec<L>, Vec<Target>) {
    let q = q.trim().to_lowercase();
    let mut lines = vec![
      L::Row {
        a: t("settings.import.add_selected").into(),
        b: f("settings.import.picked", &[("n", &self.count().to_string())]),
        pill: None,
      },
      say(t("settings.import.select_live")),
    ];
    let mut marks = vec![Target::Imp(Step::Add), Target::Imp(Step::Live)];
    for (i, f) in self.rows.iter().enumerate() {
      let label = f.label();
      if !q.split_whitespace().all(|w| label.to_lowercase().contains(w)) {
        continue;
      }
      let a = format!("[{}] {label}", if self.picked.contains(&f.login) { 'x' } else { ' ' });
      lines.push(if f.live {
        L::Tint { a, b: t("settings.import.live").into(), ink: theme::GREEN }
      } else {
        L::Row { a, b: String::new(), pill: None }
      });
      marks.push(Target::Imp(Step::Row(i)));
    }
    if marks.len() == 2 {
      lines.push(say(t("settings.import.no_match")));
      marks.push(Target::None);
    }
    (lines, marks)
  }
}

impl App {
  // /import
  pub(super) fn toggle_import(&mut self) {
    if self.screen_is(Page::Import) {
      return self.close_screen();
    }
    self.import = Import::default();
    self.open_screen(Page::Import);
    self.import_ask();
    self.import_refresh();
  }

  // a different login starts with nothing picked
  pub(super) fn import_reset(&mut self) {
    self.import = Import::default();
  }

  // ask for the follows (not while one ask is out)
  fn import_ask(&mut self) {
    if self.me.is_none() {
      self.import.phase = Phase::Failed(Fail::Login);
      return;
    }
    let now = Instant::now();
    if self.import.asked.is_some_and(|t| now.duration_since(t) < LOST) {
      return;
    }
    self.import.asked = Some(now);
    self.net.send(Cmd::Import);
  }

  // read them again from scratch
  pub(super) fn import_reload(&mut self) {
    if self.import.phase == Phase::Reading && self.import.asked.is_some() {
      return;
    }
    self.import = Import::default();
    self.import_ask();
  }

  pub(super) fn on_import(&mut self, res: Result<Got, Fail>) {
    self.import.asked = None;
    self.import.msg.clear();
    match res {
      Ok(got) => {
        let open: HashSet<Box<str>> = self.other_specs(None).into_iter().map(|s| s.twitch.into()).collect();
        let imp = &mut self.import;
        imp.all = got.rows.len();
        imp.truncated = got.truncated;
        imp.rows = got.rows.into_iter().filter(|r| !open.contains(&r.login)).collect();
        imp.picked = imp.rows.iter().filter(|r| r.live).map(|r| r.login.clone()).collect();
        imp.phase = Phase::Ready;
      }
      Err(f) => self.import.phase = Phase::Failed(f),
    }
    if self.screen_is(Page::Import) {
      self.import_refresh();
      self.wake();
    }
  }

  // the screen's rows, read fresh
  pub(super) fn import_refresh(&mut self) {
    let Some(s) = self.screen.as_ref().filter(|s| s.page() == Page::Import) else { return };
    let (lines, marks) = self.import.lines(self.me.is_some(), &s.q.text);
    if let Some(s) = &mut self.screen {
      (s.shown, s.marks) = (lines, marks);
      let (l, _) = s.lines(&self.set);
      s.st.settle(&l);
    }
  }

  // r reloads, s picks the live ones, space picks, enter adds. false = not mine (the screen's own keys, or a
  // filter being typed)
  pub(super) fn import_key(&mut self, key: &Key<&str>) -> bool {
    if self.mods.control_key() || self.screen.as_ref().is_some_and(|s| s.typing()) {
      return false;
    }
    let step = || self.screen.as_ref().and_then(|s| s.target());
    match key {
      Key::Character("r") => self.import_reload(),
      Key::Character("s") => self.import_live(),
      Key::Named(NamedKey::Space) => {
        if let Some(Target::Imp(step)) = step() {
          self.import_pick(step);
        }
      }
      Key::Named(NamedKey::Enter) => match step() {
        Some(Target::Imp(Step::Row(_))) => self.import_add(),
        Some(Target::Imp(step)) => self.import_pick(step),
        _ => {}
      },
      _ => return false,
    }
    self.import_refresh();
    true
  }

  // a click, or space / enter on a button row
  pub(super) fn import_pick(&mut self, step: Step) {
    self.import.msg.clear();
    match step {
      Step::Row(i) => {
        if let Some(login) = self.import.rows.get(i).map(|r| r.login.clone())
          && !self.import.picked.remove(&login)
        {
          self.import.picked.insert(login);
        }
      }
      Step::Add => self.import_add(),
      Step::Live => self.import_live(),
      Step::SignIn => {
        self.close_screen();
        self.act(Act::Login(Provider::Twitch), None, None);
      }
      // the screen stays: when the browser is done the login lands and the list is read again
      Step::Relink => {
        self.import.msg = t("settings.import.relinking").into();
        self.net.send(Cmd::Relink("base"));
      }
      Step::Retry => self.import_reload(),
    }
  }

  fn import_live(&mut self) {
    self.import.msg.clear();
    self.import.picked = self.import.rows.iter().filter(|r| r.live).map(|r| r.login.clone()).collect();
  }

  // one tab per pick, through the same checks and join as /join; the ones you already have are skipped
  fn import_add(&mut self) {
    let picks: Vec<Box<str>> =
      self.import.rows.iter().filter(|r| self.import.picked.contains(&r.login)).map(|r| r.login.clone()).collect();
    if picks.is_empty() {
      self.import.msg = t("settings.import.pick_one").into();
      return;
    }
    let mut have = self.other_specs(None);
    let mut added = 0;
    for login in &picks {
      if let Ok(spec) = Spec::parse(login, "", "")
        && spec.check(&have).is_ok()
      {
        have.push(spec.clone());
        self.push_tab(&spec);
        super::tabs::ask_links(&self.net, &spec);
        added += 1;
      }
    }
    if added == 0 {
      self.import.msg = t("settings.import.nothing_added").into();
      return;
    }
    self.save_tabs();
    self.close_screen();
    self.activate(self.tabs.len() - added);
    self.say(tn("settings.import.added", added as u64, &[]));
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::NetEvent;
  use winit::keyboard::NamedKey;

  fn me() -> crate::net::auth::Me {
    crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: None,
      twitch_id: None,
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    }
  }

  fn app() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    let (net, asked) = crate::net::Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(520, 420);
    app.me = Some(me());
    (app, asked)
  }

  fn follow(login: &str, name: &str, live: bool) -> Follow {
    Follow { login: login.into(), name: name.into(), live }
  }

  // live first then a to z, as the net side hands them over; forsen is a tab already
  fn got() -> Got {
    Got {
      rows: vec![
        follow("bigfish", "BigFish", true),
        follow("midfish", "", true),
        follow("amy", "Amy", false),
        follow("forsen", "", false),
        follow("zed", "", false),
      ],
      truncated: false,
    }
  }

  fn sent(asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Cmd> {
    std::iter::from_fn(|| asked.try_recv().ok()).collect()
  }

  // how many asks for the follows went out
  fn asks(asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> usize {
    sent(asked).iter().filter(|c| matches!(c, Cmd::Import)).count()
  }

  fn key(app: &mut App, k: Key<&str>) {
    let text = if let Key::Character(c) = k { Some(c) } else { None };
    app.screen_key(k, text);
  }

  fn texts(app: &App) -> Vec<String> {
    app
      .screen
      .as_ref()
      .unwrap()
      .shown
      .iter()
      .map(|l| match l {
        L::Row { a, b, .. } | L::Tint { a, b, .. } => format!("{a} | {b}"),
        L::Head(h) => h.clone(),
      })
      .collect()
  }

  #[test]
  fn opens_with_one_ask_lists_live_first_with_the_live_picked_and_skips_open_tabs() {
    let (mut app, mut asked) = app();
    app.toggle_import();
    assert!(app.screen_is(Page::Import));
    assert_eq!(asks(&mut asked), 1);
    assert_eq!(texts(&app), ["reading your twitch follows… | "]);
    app.import_reload();
    assert_eq!(asks(&mut asked), 0, "an ask already out is not asked twice");
    app.on_net(NetEvent::Import(Ok(got())));
    assert_eq!(
      texts(&app),
      [
        "add selected | 2 picked",
        "select live | ",
        "[x] bigfish (BigFish) | live",
        "[x] midfish | live",
        "[ ] amy (Amy) | ",
        "[ ] zed | ",
      ],
      "forsen has a tab, so it is not offered"
    );
    assert_eq!(app.import.title(), "import follows · 4 to add · 2 live now");
    // the mouse picks a row too; a filter keeps the matches
    app.import_pick(Step::Row(2));
    assert_eq!(app.import.count(), 3);
    app.screen.as_mut().unwrap().q.set_text("fish".into());
    app.import_refresh();
    assert_eq!(texts(&app).len(), 4, "two buttons and the two fish");
    app.screen.as_mut().unwrap().q.set_text("qqq".into());
    app.import_refresh();
    assert_eq!(texts(&app)[2], "nothing matches | ");
  }

  #[test]
  fn space_picks_enter_adds_one_tab_each_through_join_and_s_picks_the_live() {
    let (mut app, mut asked) = app();
    app.toggle_import();
    app.on_net(NetEvent::Import(Ok(got())));
    sent(&mut asked);
    // down to amy (past the two buttons, bigfish, midfish), space picks her
    for _ in 0..4 {
      key(&mut app, Key::Character("j"));
    }
    key(&mut app, Key::Named(NamedKey::Space));
    assert_eq!(app.import.count(), 3);
    key(&mut app, Key::Named(NamedKey::Space));
    assert_eq!(app.import.count(), 2, "space again drops the pick");
    key(&mut app, Key::Named(NamedKey::Space));
    // s: just the live ones again
    key(&mut app, Key::Character("s"));
    assert_eq!(app.import.count(), 2);
    key(&mut app, Key::Named(NamedKey::Space));
    let n = app.tabs.len();
    key(&mut app, Key::Named(NamedKey::Enter));
    assert!(app.screen.is_none(), "the list steps aside");
    let names: Vec<_> = app.tabs[n..].iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names, ["bigfish", "midfish", "amy"]);
    assert_eq!(app.active, n, "and you are in the first new one");
    let joins: Vec<_> = sent(&mut asked)
      .into_iter()
      .filter_map(|c| if let Cmd::Join(k) = c { Some(k.to_string()) } else { None })
      .collect();
    assert_eq!(joins, ["bigfish", "midfish", "amy"], "joined the way /join joins");
  }

  #[test]
  fn nothing_picked_and_already_open_say_so_and_add_nothing() {
    let (mut app, _asked) = app();
    app.toggle_import();
    app.on_net(NetEvent::Import(Ok(got())));
    let n = app.tabs.len();
    app.import_pick(Step::Row(0));
    app.import_pick(Step::Row(1));
    key(&mut app, Key::Named(NamedKey::Enter));
    // the cursor starts on the first button: add selected
    assert_eq!(app.import.msg, "pick at least one channel");
    assert_eq!((app.tabs.len(), app.screen_is(Page::Import)), (n, true));
    // a tab for it appears while the list is open: adding it is skipped, and the list says why
    app.import_pick(Step::Row(0));
    app.add_tab(&Spec::parse("bigfish", "", "").unwrap());
    let n = app.tabs.len();
    app.import_pick(Step::Add);
    assert_eq!(app.import.msg, "nothing added — those are already in your tabs");
    assert_eq!(app.tabs.len(), n);
    // every follow already open
    app.on_net(NetEvent::Import(Ok(Got { rows: vec![follow("forsen", "", true)], truncated: false })));
    assert_eq!(texts(&app), ["every channel you follow is already here | "]);
    app.on_net(NetEvent::Import(Ok(Got { rows: vec![], truncated: false })));
    assert_eq!(texts(&app), ["you don't follow anyone on twitch yet | "]);
  }

  #[test]
  fn each_failure_says_its_own_sentence_and_offers_the_way_out() {
    let (mut app, mut asked) = app();
    app.toggle_import();
    sent(&mut asked);
    let say = |app: &mut App, f| {
      app.on_net(NetEvent::Import(Err(f)));
      texts(app)
    };
    assert_eq!(
      say(&mut app, Fail::Login),
      ["log in with twitch to import your follows, sorted by who's live | ", "log in | "]
    );
    assert_eq!(say(&mut app, Fail::Down), ["couldn't reach twitch — try again in a minute | ", "try again | "]);
    assert_eq!(
      say(&mut app, Fail::Expired),
      ["your twitch link expired — reconnect it to import follows | ", "reconnect twitch | "]
    );
    assert_eq!(
      say(&mut app, Fail::Scope),
      ["your twitch link doesn't include follows — reconnect it to import them | ", "reconnect twitch | "]
    );
    // the way out is the second row: reconnect asks for the base pack, the screen stays for the answer
    key(&mut app, Key::Character("j"));
    key(&mut app, Key::Named(NamedKey::Enter));
    assert!(sent(&mut asked).iter().any(|c| matches!(c, Cmd::Relink("base"))));
    assert!(app.screen_is(Page::Import));
    assert!(app.import.msg.contains("reconnecting twitch"));
    // r reads again from scratch
    app.on_net(NetEvent::Import(Err(Fail::Down)));
    key(&mut app, Key::Character("r"));
    assert_eq!(asks(&mut asked), 1);
    assert_eq!(texts(&app), ["reading your twitch follows… | "]);
  }

  #[test]
  fn logged_out_asks_nobody_and_typing_a_filter_keeps_r_and_s_as_letters() {
    let (mut app, mut asked) = app();
    app.me = None;
    app.toggle_import();
    assert_eq!(asks(&mut asked), 0, "nobody to ask for");
    assert!(texts(&app)[0].starts_with("log in with twitch"));
    app.toggle_import();
    assert!(app.screen.is_none());
    app.me = Some(me());
    app.toggle_import();
    app.on_net(NetEvent::Import(Ok(got())));
    sent(&mut asked);
    key(&mut app, Key::Character("/"));
    key(&mut app, Key::Character("s"));
    key(&mut app, Key::Character("r"));
    assert_eq!(app.screen.as_ref().unwrap().q.text, "sr");
    assert_eq!(asks(&mut asked), 0);
    // a different login starts empty
    app.import_reset();
    assert!(app.import.rows.is_empty());
  }

  #[test]
  fn the_list_draws_with_marks_and_live_in_green() {
    let (mut app, _asked) = app();
    app.toggle_import();
    app.on_net(NetEvent::Import(Ok(got())));
    key(&mut app, Key::Character("j"));
    key(&mut app, Key::Character("j"));
    key(&mut app, Key::Character("j"));
    app.redraw();
    crate::bench::dump(&app.canvas);
    assert!(app.canvas.inked(theme::WHITE) > 400, "the picked row is a white bar");
    assert!(app.canvas.inked(theme::GREEN) > 20, "live rows read green");
  }
}
