// the settings screen (ctrl-, / /settings / the gear) and the key map (? / /keys): list screens over the
// chat built from model::schema and model::keys. j/k move, space or enter changes the row, / searches,
// esc closes. every change goes through `flip`, the same path the slash commands use
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::lists::Kind;
use crate::model::keys::KEYS;
use crate::model::line::Line;
use crate::model::schema::{Ctl, DEFS, Def, Fx};
use crate::model::settings::{Settings, ZOOM_MAX, ZOOM_STEP, clamp_zoom};
use crate::model::vi::Mode;
use crate::ui::canvas::Rect;
use crate::ui::sheet::{self, L, State};
use crate::ui::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
  Settings,
  Keys,
  // a twitch room's blocked terms (app::blocked)
  Blocked,
  // a tab's stream stats (app::stats)
  Summary,
  // a twitch channel's stream and chat modes (app::status)
  Status,
  // a twitch room's mod tools (app::modtools)
  ModTools,
  // a twitch channel's prediction (app::pred)
  Pred,
  // who you are logged in as (app::account)
  Account,
  // who you follow is live (app::following)
  Following,
  // your twitch follows, picked into tabs (app::import)
  Import,
  // your heatsync emotes: slots, removed, saved sets (app::inventory)
  Inventory,
  // a list you keep: muted & blocked people, highlight words, filter rules… (app::lists)
  List,
}

pub struct Screen {
  page: Page,
  pub(super) st: State,
  pub(super) q: Line,
  pub(super) typing: bool,
  // the first g of gg
  chord: bool,
  pub(super) bt: Option<Box<super::blocked::Bt>>,
  pub(super) mt: Option<Box<super::modtools::Mt>>,
  pub(super) inv: Option<Box<super::inventory::Inv>>,
  pub(super) ls: Option<Box<super::lists::Ls>>,
  // the summary: whose (tab name), and its title and lines as last read
  pub(super) of: Box<str>,
  // the settings screen is asking "reset everything to defaults?": y does it, any other key keeps
  pub(super) ask_reset: bool,
  pub(super) title: String,
  pub(super) shown: Vec<L>,
  // the status screen: what its rows stand for, and the stream (None = still asking, Some(None) = no answer)
  pub(super) marks: Vec<Target>,
  pub(super) info: Option<Option<crate::model::status::Info>>,
  // how many characters of a row the screen holds across, as last drawn (the key map wraps to it)
  cols: usize,
}

// what a row stands for
#[derive(Clone, Copy)]
pub(super) enum Target {
  Def(&'static Def),
  // the settings row that puts every look and behaviour back to its default (asks first)
  Reset,
  // blocked terms: a term (its place in the list), ask for the permission, ask again
  Term(usize),
  Allow,
  Retry,
  // the status screen: a chat mode, the stream summary, blocked terms
  Mode(crate::model::modes::Mode),
  Summary,
  BlockedTerms,
  ModTools,
  // mod tools: shield mode, an automod level (its place), an unban request (its place)
  Shield,
  Level(u8),
  Unban(usize),
  // the account screen: log in, link, log out
  Account(super::account::Step),
  // the following screen: a live person (their place in the list)
  Live(usize),
  // the import screen: a follow, a button, a way in
  Imp(super::import::Step),
  // your emotes: a row of the current tab (its place in that tab's full list)
  Inv(usize),
  // a list screen: an entry (its place), the row that adds one
  Item(usize),
  AddItem,
  None,
}

// what the reset row answers to in the search, and what it says
const RESET_WORDS: &str = "reset to defaults restore default settings";
const RESET_LABEL: &str = "reset to defaults";
const RESET_TIP: &str = "puts every look and behaviour setting back to its default. keeps your login, your tabs and which built-in tabs show, filter rules, muted and blocked people, mention rules, notes and everything saved";
const RESET_ASK: &str = "reset every look and behaviour setting to its default? your login, tabs, filter rules, people lists, mention rules and notes stay · y resets · any other key keeps";

// what a key asks of the app, once the screen's own state is updated
enum Do {
  Nothing,
  Close,
  Toggle,
  // l / h: set a switch on / off, or step zoom up / down
  Adjust(bool),
  Keys,
  Settings,
}

impl Screen {
  fn new(page: Page) -> Self {
    Screen {
      page,
      st: State::default(),
      q: Line::default(),
      typing: false,
      chord: false,
      bt: None,
      mt: None,
      inv: None,
      ls: None,
      of: "".into(),
      ask_reset: false,
      title: String::new(),
      shown: Vec::new(),
      marks: Vec::new(),
      info: None,
      cols: 80,
    }
  }

  pub fn page(&self) -> Page {
    self.page
  }

  pub fn typing(&self) -> bool {
    self.typing
  }

  // where typed text goes: the search, or the blocked term being added
  pub fn query_mut(&mut self) -> &mut Line {
    match (self.bt.as_deref_mut(), self.inv.as_deref_mut(), self.ls.as_deref_mut()) {
      (Some(bt), ..) if bt.adding => &mut bt.add,
      (_, Some(iv), _) if iv.ask.is_some() => &mut iv.input,
      (.., Some(ls)) if ls.adding => &mut ls.add,
      _ => &mut self.q,
    }
  }

  pub(super) fn lines(&self, set: &Settings) -> (Vec<L>, Vec<Target>) {
    match self.page {
      Page::Keys => keys_lines(set.vi, self.cols),
      Page::Settings => settings_lines(self.q.text.trim(), set),
      Page::Blocked => self.bt.as_ref().map_or_else(|| (Vec::new(), Vec::new()), |b| b.lines(&self.q.text)),
      Page::Summary => (self.shown.clone(), vec![Target::None; self.shown.len()]),
      Page::Status => (self.shown.clone(), self.marks.clone()),
      Page::Pred => (self.shown.clone(), vec![Target::None; self.shown.len()]),
      Page::ModTools => self.mt.as_ref().map_or_else(|| (Vec::new(), Vec::new()), |m| m.lines(&self.q.text)),
      Page::Account | Page::Following | Page::Import => (self.shown.clone(), self.marks.clone()),
      Page::Inventory => self.inv.as_ref().map_or_else(|| (Vec::new(), Vec::new()), |v| v.lines(&self.q.text)),
      Page::List => self.ls.as_ref().map_or_else(|| (Vec::new(), Vec::new()), |l| l.lines()),
    }
  }

  // what the picked row stands for (settings aside: their rows need the settings to list)
  pub(super) fn target(&self) -> Option<Target> {
    let (_, t) = self.lines(&Settings::default());
    t.get(self.st.at).copied()
  }
}

// the key map: a heading per group, a row per line. a vi row says so (`vi` before its keys) and reads dim until
// vi mode is on; a group made only of them says it once, in its heading. a row that does not fit `cols`
// characters across says what it does, dim, on the lines under its keys
fn keys_lines(vi: bool, cols: usize) -> (Vec<L>, Vec<Target>) {
  let mut lines = Vec::new();
  for g in &KEYS {
    let all = g.all_vi();
    let mixed = !all && g.rows.iter().any(|r| r.vi);
    lines.push(L::Head(if all { format!("{} · vi", g.name) } else { g.name.to_owned() }));
    let lead = if mixed { 4 } else { 0 };
    for r in g.rows {
      let a = match (mixed, r.vi) {
        (true, true) => format!("vi  {}", r.keys),
        (true, false) => format!("    {}", r.keys),
        _ => r.keys.to_owned(),
      };
      let row = |a: String, b: String| {
        if r.vi && !vi { L::Tint { a, b, ink: theme::DIM } } else { L::Row { a, b, pill: None } }
      };
      if a.chars().count() + 2 + r.does.chars().count() <= cols {
        lines.push(row(a, r.does.to_owned()));
        continue;
      }
      lines.push(row(a, String::new()));
      for part in wrap(r.does, cols.saturating_sub(lead + 2)) {
        lines.push(L::Tint { a: format!("{:1$}{part}", "", lead + 2), b: String::new(), ink: theme::DIM });
      }
    }
  }
  let n = lines.len();
  (lines, vec![Target::None; n])
}

// `s` cut at spaces into lines of at most `width` characters (a longer word keeps a line to itself)
fn wrap(s: &str, width: usize) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  for word in s.split_whitespace() {
    match out.last_mut() {
      Some(l) if l.chars().count() + 1 + word.chars().count() <= width => {
        l.push(' ');
        l.push_str(word);
      }
      _ => out.push(word.to_owned()),
    }
  }
  out
}

// under the key map: what the `vi` rows need
fn keys_tip(vi: bool) -> &'static str {
  if vi {
    "vi rows work in normal mode: esc leaves typing first"
  } else {
    "dim vi rows need vi mode: /vi turns it on, then esc leaves typing and they work"
  }
}

fn settings_lines(q: &str, set: &Settings) -> (Vec<L>, Vec<Target>) {
  let mut lines = Vec::new();
  let mut targets = Vec::new();
  if q.split_whitespace().all(|w| RESET_WORDS.contains(&w.to_lowercase())) {
    lines.push(L::Head("reset".into()));
    targets.push(Target::None);
    lines.push(L::Row { a: RESET_LABEL.to_owned(), b: "enter asks first".to_owned(), pill: None });
    targets.push(Target::Reset);
  }
  let mut last = None;
  for d in DEFS.iter().filter(|d| d.matches(q)) {
    if last != Some((d.cat, d.section)) {
      last = Some((d.cat, d.section));
      lines.push(L::Head(format!("{} · {}", d.cat.name(), d.section)));
      targets.push(Target::None);
    }
    let (b, pill) = match d.ctl {
      Ctl::Pill(..) => (String::new(), d.on(set)),
      Ctl::Zoom => (format!("{}%", set.zoom), None),
      Ctl::Side => (set.tab_side.name().to_owned(), None),
      Ctl::Counter => (set.tab_counter.name().to_owned(), None),
      Ctl::Pick(..) => (d.picked(set).unwrap_or_default().to_owned(), None),
      Ctl::Rules => {
        let n = crate::model::filter::with(|f| f.rules().len());
        (if n == 0 { "none".to_owned() } else { format!("{n} · enter opens") }, None)
      }
      Ctl::Alerts => ("enter opens".to_owned(), None),
      Ctl::Commands => ("enter opens".to_owned(), None),
      Ctl::People => {
        let (m, b) = crate::model::filter::with(|f| (f.mutes().len(), f.blocks_sorted().len()));
        (if m + b == 0 { "nobody".to_owned() } else { format!("{m} muted · {b} blocked") }, None)
      }
      Ctl::Buttons(get, _) => {
        let m = get(set);
        (if m == 0 { "none".to_owned() } else { crate::model::modbar::labels(m) }, None)
      }
      Ctl::Reasons => {
        let n = crate::model::filter::with(|f| f.reasons().len());
        (if n == 0 { "none".to_owned() } else { format!("{n} · enter opens") }, None)
      }
    };
    lines.push(L::Row { a: d.label.to_owned(), b, pill });
    targets.push(Target::Def(d));
  }
  (lines, targets)
}

impl App {
  // ---- changing a setting: the one path for slash commands and the screen ----

  // what a flipped switch has to redo
  pub(super) fn applied(&mut self, fx: Fx) {
    match fx {
      Fx::None => {}
      Fx::Vi => self.mode = Mode::Insert,
      Fx::Glow => {
        for v in self.tabs.iter_mut().flat_map(|t| t.views()) {
          v.glow = self.set.first_glow;
          v.invalidate();
        }
      }
      Fx::ReadLine => {
        for v in self.tabs.iter_mut().flat_map(|t| t.views()) {
          v.clear_read();
        }
      }
      Fx::Paints => {
        self.res.cosmetics.on = self.set.paints;
        self.res.cosmetics.hs_on = self.set.hs_paints;
        // the 7tv lookup of your own id follows the switch
        let id = self.me.as_ref().and_then(|m| m.twitch_id);
        self.net.send(crate::net::Cmd::SevenTvMe(super::net_events::seventv_id(id, self.set.paints)));
        self.relayout_where(None);
      }
      Fx::Look => {
        self.res.apply_look(&self.set);
        self.relayout_where(None);
      }
      Fx::Links => {
        self.res.links = self.set.links;
        self.relayout_where(None);
      }
      Fx::Media => {
        self.sync_media();
        self.relayout_where(None);
      }
      Fx::Streamer => self.streamer_follow(),
      Fx::ChatLogs => self.chatlog.set(self.set.chat_logs),
      Fx::Filters => {
        self.push_flags();
        self.refilter_all();
      }
      Fx::Content => {
        crate::heatsync::content::set_shown(self.set.content());
        self.relayout_where(None);
        self.feed_show();
      }
      Fx::Tab(_) => self.fix_active(),
    }
    self.input_dirty = true;
    self.bar_dirty = true;
    self.wake();
  }

  // flip a switch and save: the change shows for itself
  pub(super) fn flip(&mut self, d: &'static Def) {
    let Some(on) = d.on(&self.set).map(|v| !v) else { return };
    d.put(&mut self.set, on);
    self.applied(d.fx);
    self.set.save();
  }

  // a Pick row to value `i`: applied and saved when it moves
  pub(super) fn pick_to(&mut self, d: &'static Def, i: usize) {
    if d.at(&self.set) != Some(i) && d.choose(&mut self.set, i) {
      self.applied(d.fx);
      self.set.save();
    }
  }

  // /modbuttons <names>: the toolbar's buttons, replaced as typed; nothing typed says what is picked
  pub(super) fn buttons_to(&mut self, d: &'static Def, arg: &str) {
    use crate::model::modbar;
    let Ctl::Buttons(get, set) = d.ctl else { return };
    let usage = "usage: /modbuttons x 1m 10m 1h 24h 7d purge ban unban (or off)";
    let cur = get(&self.set);
    if arg.is_empty() {
      let now = if cur == 0 { "none".to_owned() } else { modbar::labels(cur) };
      return self.say(format!("mod toolbar: {now} \u{b7} {usage}"));
    }
    let (mask, bad) = if matches!(arg, "off" | "none") { (0, 0) } else { modbar::parse(arg) };
    if bad > 0 && mask == 0 {
      return self.say(usage);
    }
    if mask != cur {
      set(&mut self.set, mask);
      self.set.save();
      self.wake();
    }
    let now = if mask == 0 { "off".to_owned() } else { modbar::labels(mask) };
    self.say(if bad > 0 { format!("mod toolbar: {now} ({bad} not buttons)") } else { format!("mod toolbar: {now}") });
  }

  pub(super) fn set_switch(&mut self, d: &'static Def, on: bool) {
    if d.on(&self.set).is_some_and(|v| v != on) {
      self.flip(d);
    }
  }

  // every look and behaviour back to its default, each change applied the way flipping it would (the kept
  // ones are in `Settings::reset`)
  pub(super) fn reset_settings(&mut self) {
    let old = self.set;
    self.set = old.reset();
    let mut fx = Vec::new();
    for d in DEFS.iter() {
      let moved = d.on(&old) != d.on(&self.set) || d.picked(&old) != d.picked(&self.set);
      if moved && d.fx != Fx::None && !fx.contains(&d.fx) {
        fx.push(d.fx);
      }
    }
    let (side, counter, zoom) = (self.set.tab_side, self.set.tab_counter, self.set.zoom);
    (self.set.tab_side, self.set.tab_counter, self.set.zoom) = (old.tab_side, old.tab_counter, old.zoom);
    self.side_to(side);
    self.counter_to(counter);
    self.zoom_to(zoom);
    for f in fx {
      self.applied(f);
    }
    self.set.save();
    self.say("settings are back to their defaults");
  }

  // /reset and the settings row: open the settings and ask first
  pub(super) fn ask_reset(&mut self) {
    if !self.screen_is(Page::Settings) {
      self.open_screen(Page::Settings);
    }
    if let Some(s) = self.screen.as_mut() {
      s.ask_reset = true;
    }
  }

  // ---- opening and closing ----

  pub(super) fn open_screen(&mut self, page: Page) {
    if self.picker.is_some() {
      self.close_picker();
    }
    self.close_prompt();
    self.overlay = None;
    let mut s = Screen::new(page);
    let (lines, _) = s.lines(&self.set);
    s.st.settle(&lines);
    self.screen = Some(s);
    self.screen_changed();
  }

  pub(super) fn close_screen(&mut self) {
    if self.screen.take().is_some() {
      self.ime.clear();
      if let Some(t) = self.tabs.get_mut(self.active) {
        t.view.invalidate();
      }
      self.screen_changed();
    }
  }

  pub(super) fn screen_is(&self, page: Page) -> bool {
    self.screen.as_ref().is_some_and(|s| s.page == page)
  }

  pub(super) fn toggle_screen(&mut self, page: Page) {
    if self.screen_is(page) {
      self.close_screen();
    } else {
      self.open_screen(page);
    }
  }

  fn screen_changed(&mut self) {
    self.bar_dirty = true;
    self.input_dirty = true;
    self.wake();
  }

  // ---- keys and the mouse ----

  pub(super) fn screen_key(&mut self, key: Key<&str>, text: Option<&str>) {
    // the reset question takes the next key: y does it, anything else keeps
    if let Some(s) = self.screen.as_mut().filter(|s| s.ask_reset) {
      s.ask_reset = false;
      if matches!(key, Key::Character("y")) {
        self.reset_settings();
      }
      return self.screen_changed();
    }
    let page = self.screen.as_ref().map(|s| s.page);
    let spent = match page {
      Some(Page::Blocked) => self.blocked_key(&key, text),
      Some(Page::ModTools) => self.tools_key(&key),
      Some(Page::Pred) => self.pred_key(&key),
      Some(Page::Following) => self.following_key(&key),
      Some(Page::Import) => self.import_key(&key),
      Some(Page::Inventory) => self.inv_key(&key, text),
      Some(Page::List) => self.list_key(&key, text),
      _ => false,
    };
    if spent {
      if let Some(s) = self.screen.as_mut() {
        let (lines, _) = s.lines(&self.set);
        s.st.settle(&lines);
      }
      return self.screen_changed();
    }
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    let accepts = self.ime.accepts_key_text();
    let Some(s) = self.screen.as_mut() else { return };
    let (lines, _) = s.lines(&self.set);
    let chord = std::mem::take(&mut s.chord);
    let half = (self.canvas.h / self.res.text.line_px.max(1.0) as i32 / 2).max(1);
    let mut act = Do::Nothing;
    if s.typing {
      match key {
        Key::Named(NamedKey::Escape | NamedKey::Enter) => s.typing = false,
        Key::Named(NamedKey::ArrowDown) => s.st.step(&lines, 1),
        Key::Named(NamedKey::ArrowUp) => s.st.step(&lines, -1),
        _ if super::tabs::edit_line(&mut s.q, &key, ctrl) => s.st.at = 0,
        Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
          if let Some(t) = text.filter(|_| accepts) {
            s.q.insert(t);
            s.st.at = 0;
          }
        }
        _ => return,
      }
    } else {
      match key {
        Key::Named(NamedKey::Escape) if !s.q.text.is_empty() => {
          s.q.set_text(String::new());
          s.q.home();
          s.st.at = 0;
        }
        Key::Named(NamedKey::Escape) | Key::Character("q") => act = Do::Close,
        Key::Character("j") | Key::Named(NamedKey::ArrowDown) => s.st.step(&lines, 1),
        Key::Character("k") | Key::Named(NamedKey::ArrowUp) => s.st.step(&lines, -1),
        Key::Character("d") if ctrl => s.st.shift(&lines, half),
        Key::Character("u") if ctrl => s.st.shift(&lines, -half),
        Key::Named(NamedKey::PageDown) => s.st.shift(&lines, half * 2),
        Key::Named(NamedKey::PageUp) => s.st.shift(&lines, -half * 2),
        Key::Character("g") if chord => s.st.first(&lines),
        Key::Character("g") => s.chord = true,
        Key::Named(NamedKey::Home) => s.st.first(&lines),
        Key::Character("G") | Key::Named(NamedKey::End) => s.st.last(&lines),
        Key::Character("/")
          if matches!(
            s.page,
            Page::Settings | Page::Blocked | Page::ModTools | Page::Following | Page::Import | Page::Inventory
          ) =>
        {
          s.typing = true
        }
        Key::Character("?") if s.page == Page::Settings => act = Do::Keys,
        Key::Character("?") => act = Do::Settings,
        Key::Named(NamedKey::Enter | NamedKey::Space) if !ctrl => act = Do::Toggle,
        Key::Character("l") | Key::Named(NamedKey::ArrowRight) => act = Do::Adjust(true),
        Key::Character("h") | Key::Named(NamedKey::ArrowLeft) => act = Do::Adjust(false),
        _ => return,
      }
    }
    s.st.settle(&lines);
    self.screen_do(act);
    self.screen_changed();
  }

  fn screen_do(&mut self, act: Do) {
    match act {
      Do::Nothing => {}
      Do::Close => self.close_screen(),
      Do::Keys => self.open_screen(Page::Keys),
      Do::Settings => self.open_screen(Page::Settings),
      Do::Toggle => self.screen_pick(None),
      Do::Adjust(up) => self.screen_pick(Some(up)),
    }
  }

  // the picked row's change: None = the row's own (flip, apply, step up, add a word), Some(up) = h / l
  fn screen_pick(&mut self, adjust: Option<bool>) {
    let Some(s) = &self.screen else { return };
    let (_, targets) = s.lines(&self.set);
    match (targets.get(s.st.at).copied(), adjust) {
      (Some(Target::Def(d)), _) => match d.ctl {
        Ctl::Pill(..) => match adjust {
          Some(on) => self.set_switch(d, on),
          None => self.flip(d),
        },
        Ctl::Zoom => {
          let z = self.set.zoom;
          let next = match adjust {
            Some(up) => clamp_zoom(z as i32 + if up { ZOOM_STEP as i32 } else { -(ZOOM_STEP as i32) }),
            None if z >= ZOOM_MAX => 100,
            None => clamp_zoom(z as i32 + ZOOM_STEP as i32),
          };
          self.zoom_to(next);
        }
        Ctl::Side => self.side_to(self.set.tab_side.step(adjust.unwrap_or(true))),
        Ctl::Counter => self.counter_to(self.set.tab_counter.step(adjust.unwrap_or(true))),
        Ctl::Pick(c, get, _) => {
          let (n, at) = (c.names.len(), get(&self.set));
          self.pick_to(d, (at + if adjust.unwrap_or(true) { 1 } else { n - 1 }) % n);
        }
        // a list: its own screen
        Ctl::Rules if adjust.is_none() => self.list_open(Kind::Rules),
        Ctl::Reasons if adjust.is_none() => self.list_open(Kind::Reasons),
        Ctl::People if adjust.is_none() => self.list_open(Kind::People),
        Ctl::Alerts if adjust.is_none() => self.list_open(Kind::Alerts),
        Ctl::Commands if adjust.is_none() => self.list_open(Kind::Commands),
        // the toolbar's buttons live in a slash command: leave with it ready to finish
        Ctl::Buttons(..) if adjust.is_none() => {
          self.close_screen();
          if let Some(c) = d.cmd {
            self.type_into_line(&format!("{c} "));
          }
        }
        Ctl::Rules | Ctl::Reasons | Ctl::People | Ctl::Alerts | Ctl::Commands | Ctl::Buttons(..) => {}
      },
      (Some(Target::Reset), None) => self.ask_reset(),
      (Some(Target::Account(step)), None) => self.account_step(step),
      (Some(Target::Live(i)), None) => self.following_pick(i),
      (Some(t @ (Target::Item(_) | Target::AddItem)), None) if s.page == Page::List => self.list_pick(t),
      (Some(Target::Imp(step)), None) => self.import_pick(step),
      (Some(t @ (Target::Allow | Target::Retry | Target::Shield | Target::Level(_) | Target::Unban(_))), _)
        if s.page == Page::ModTools =>
      {
        self.tools_pick(t, adjust)
      }
      (Some(t @ (Target::Inv(_) | Target::Retry)), None) if s.page == Page::Inventory => self.inv_pick(t),
      (Some(t @ (Target::Term(_) | Target::Allow | Target::Retry)), None) => self.blocked_pick(t),
      (Some(t @ (Target::Mode(_) | Target::Summary | Target::BlockedTerms | Target::ModTools)), _) => {
        self.status_pick(t, adjust)
      }
      _ => {}
    }
  }

  pub(super) fn screen_hover(&mut self) {
    let (x, y) = self.pointer;
    let Some(s) = &mut self.screen else { return };
    if let Some(i) = s.st.hit(x, y)
      && i != s.st.at
    {
      s.st.at = i;
      self.wake();
    }
  }

  pub(super) fn screen_click(&mut self) {
    let (x, y) = self.pointer;
    let Some(s) = &mut self.screen else { return };
    if let Some(i) = s.st.hit(x, y) {
      s.st.at = i;
      self.screen_pick(None);
    }
    self.screen_changed();
  }

  // 3 rows a notch, the way the chat scrolls; up (px > 0) goes toward the top
  pub(super) fn screen_wheel(&mut self, px: i32) {
    let Some(s) = &mut self.screen else { return };
    let (lines, _) = s.lines(&self.set);
    s.st.shift(&lines, if px > 0 { -3 } else { 3 });
    self.wake();
  }

  pub(super) fn draw_screen(&mut self) -> Option<Rect> {
    self.account_refresh();
    self.following_refresh();
    self.import_refresh();
    self.summary_refresh();
    self.status_refresh();
    let area = self.chat_area();
    let em = self.res.text.layout("0", &[], &[], f32::MAX).full_width().ceil().max(1.0) as i32;
    let s = self.screen.as_mut()?;
    // the sheet pads each side by 6 and starts its text one cell in
    s.cols = ((area.w - 12) / em - 1).max(20) as usize;
    let (lines, targets) = s.lines(&self.set);
    s.st.settle(&lines);
    let tip = match targets.get(s.st.at) {
      _ if s.ask_reset => RESET_ASK,
      Some(Target::Def(d)) => d.tip,
      Some(Target::Reset) => RESET_TIP,
      Some(Target::Account(step)) => step.tip(),
      _ if s.page == Page::Following => self.following.tip(match targets.get(s.st.at) {
        Some(Target::Live(i)) => Some(*i),
        _ => None,
      }),
      _ => "",
    };
    let shown = s.marks.iter().filter(|t| matches!(t, Target::Live(_))).count();
    let imp_title = self.import.title();
    let fol_title = self.following.title(self.me.is_some(), !s.q.text.trim().is_empty(), shown);
    let bt_title = s.bt.as_ref().map(|b| b.title(&s.q.text)).unwrap_or_default();
    let mt_title = s.mt.as_ref().map(|m| format!("mod tools · {}", m.chan)).unwrap_or_default();
    let inv_title = s.inv.as_ref().map(|v| v.title()).unwrap_or_default();
    let ls_title = s.ls.as_ref().map(|l| l.title()).unwrap_or_default();
    let ls_mark = s.ls.as_ref().map(|l| l.prompt()).unwrap_or_default();
    let keys_title = format!("keys · heatsync-chat {}", env!("CARGO_PKG_VERSION"));
    let (title, search, keys) = match (s.page, s.bt.as_deref()) {
      (Page::Blocked, Some(b)) => (
        bt_title.as_str(),
        Some(if b.adding {
          (b.add.text.as_str(), true, "", "+ ")
        } else {
          (s.q.text.as_str(), s.typing, "/ filter · a add", "/ ")
        }),
        if b.adding {
          "type a term · enter adds · esc stops"
        } else if s.typing {
          "type to filter · enter or esc stops"
        } else if b.confirm.is_some() {
          "y deletes · n keeps"
        } else {
          "j k move · x delete · a add · / filter · esc closes"
        },
      ),
      (Page::Settings, _) => (
        "settings",
        Some((s.q.text.as_str(), s.typing, "/ search", "/ ")),
        if s.typing {
          "type to filter · enter or esc stops · up down moves"
        } else {
          "j k move · space change · / search · ? keys · esc"
        },
      ),
      (Page::Inventory, _) => match s.inv.as_deref() {
        Some(v) => (
          inv_title.as_str(),
          Some(match v.ask {
            Some(a) => (v.input.text.as_str(), true, "", a.mark()),
            None => (s.q.text.as_str(), s.typing, "/ filter", "/ "),
          }),
          v.keys(s.typing),
        ),
        None => ("emotes", None, "esc closes"),
      },
      (Page::List, _) => match s.ls.as_deref() {
        Some(l) => (ls_title.as_str(), l.adding.then_some((l.add.text.as_str(), true, "", ls_mark.as_str())), l.keys()),
        None => ("list", None, "esc closes"),
      },
      (Page::Pred, _) => (s.title.as_str(), None, "read only · j k scroll · r refresh · esc closes"),
      (Page::Account, _) => ("account", None, "j k move · enter picks · esc closes"),
      (Page::Following, _) => (
        fol_title.as_str(),
        Some((s.q.text.as_str(), s.typing, "/ filter", "/ ")),
        if s.typing {
          "type to filter · enter or esc stops"
        } else {
          "j k move · enter opens a tab · w watches · r refreshes · / filter · esc closes"
        },
      ),
      (Page::Import, _) => (
        imp_title.as_str(),
        Some((s.q.text.as_str(), s.typing, "/ filter", "/ ")),
        if s.typing {
          "type to filter · enter or esc stops"
        } else {
          "space picks · enter adds · s picks live · r reloads · / filter · esc"
        },
      ),
      (Page::Summary, _) => (s.title.as_str(), None, "counted live from when this window joined · esc closes"),
      (Page::ModTools, _) => (
        mt_title.as_str(),
        Some((s.q.text.as_str(), s.typing, "/ filter", "/ ")),
        if s.typing {
          "type to filter · enter or esc stops"
        } else if s.mt.as_ref().is_some_and(|m| m.confirm.is_some()) {
          "y yes · n no"
        } else {
          "j k move · enter flips · h l level · y approve · x deny · r reload · esc"
        },
      ),
      (Page::Status, _) if s.marks.iter().any(|t| matches!(t, Target::BlockedTerms)) => {
        (s.title.as_str(), None, "j k move · enter flips · h l wait · esc closes")
      }
      (Page::Status, _) => (s.title.as_str(), None, "j k move · enter opens · esc closes"),
      _ => (keys_title.as_str(), None, "j k scroll · ? settings · esc closes"),
    };
    let tip = match (s.bt.as_deref(), s.mt.as_deref()) {
      (Some(b), _) if s.page == Page::Blocked => b.msg.as_str(),
      (_, Some(m)) if s.page == Page::ModTools => m.msg.as_str(),
      _ if s.page == Page::Keys => keys_tip(self.set.vi),
      _ if s.page == Page::Inventory => s.inv.as_deref().map_or("", |v| v.tip()),
      _ if s.page == Page::Import => self.import.msg.as_str(),
      _ if s.page == Page::List => s.ls.as_deref().map_or("", |l| l.tip()),
      _ => tip,
    };
    let v = sheet::View { title, search, lines: &lines, tip, keys, col: None };
    sheet::draw(&mut self.canvas, &mut self.res.text, area, &v, &mut s.st);
    Some(area)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_setting_has_a_row_under_its_section_heading() {
    let (lines, targets) = settings_lines("", &Settings::default());
    assert_eq!(lines.len(), targets.len());
    let rows = targets.iter().filter(|t| matches!(t, Target::Def(_))).count();
    assert_eq!(rows, DEFS.len());
    assert!(matches!(&lines[0], L::Head(h) if h == "reset"));
    assert!(lines.iter().any(|l| matches!(l, L::Head(h) if h == "filters · messages")));
    // headings never come twice in a row and never end the list
    assert!(matches!(lines.last(), Some(L::Row { .. })));
    assert!(lines.windows(2).all(|w| !(matches!(w[0], L::Head(_)) && matches!(w[1], L::Head(_)))));
  }

  #[test]
  fn search_keeps_matching_rows_and_their_headings_only() {
    let (lines, targets) = settings_lines("hide", &Settings::default());
    let ids: Vec<&str> =
      targets.iter().filter_map(|t| if let Target::Def(d) = t { Some(d.id) } else { None }).collect();
    assert!(ids.contains(&"hide_bots") && ids.contains(&"filter_rules") && !ids.contains(&"vi"));
    assert!(matches!(&lines[0], L::Head(_)));
    let (none, _) = settings_lines("zzzz", &Settings::default());
    assert!(none.is_empty());
    // the reset row answers to its own words
    let (p, t) = settings_lines("reset defaults", &Settings::default());
    assert!(t.iter().any(|t| matches!(t, Target::Reset)) && p.len() == 2);
  }

  // the old "normal" preset left these changed while still reading as active; the reset asks first, then
  // puts back all of them, and leaves the bar's tabs and the saved lists alone
  #[test]
  fn reset_asks_first_then_restores_the_defaults_and_keeps_the_lists() {
    let (net, _asked) = crate::net::Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    crate::model::filter::edit(|f| _ = f.block("troll"));
    app.set = Settings::default();
    app.set.zebra = false;
    app.set.platform_tags = false;
    app.set.hs_paints = false;
    app.set.dim_timeouts = false;
    app.set.tab_feed = false;
    app.open_screen(Page::Settings);
    app.ask_reset();
    assert!(app.screen.as_ref().unwrap().ask_reset);
    // anything but y keeps what you have
    app.screen_key(Key::Character("n"), None);
    assert!(!app.screen.as_ref().unwrap().ask_reset && !app.set.zebra);
    app.ask_reset();
    app.screen_key(Key::Character("y"), None);
    assert!(app.set.zebra && app.set.platform_tags && app.set.hs_paints && app.set.dim_timeouts);
    assert!(!app.set.tab_feed, "the tabs on the bar stay as they were");
    assert!(crate::model::filter::with(|f| f.blocked("troll")));
  }

  #[test]
  fn the_key_map_lists_every_group() {
    let (lines, targets) = keys_lines(false, 80);
    assert_eq!(lines.iter().filter(|l| matches!(l, L::Head(_))).count(), KEYS.len());
    assert!(targets.iter().all(|t| matches!(t, Target::None)));
  }

  #[test]
  fn a_narrow_key_map_wraps_what_the_keys_do_and_loses_none_of_it() {
    let words = |lines: &[L]| -> Vec<String> {
      lines
        .iter()
        .flat_map(|l| match l {
          L::Row { a, b, .. } | L::Tint { a, b, .. } => {
            format!("{a} {b}").split_whitespace().map(str::to_owned).collect()
          }
          L::Head(_) => Vec::new(),
        })
        .collect()
    };
    let (wide, _) = keys_lines(true, 200);
    for cols in [40, 46, 60, 100] {
      let (lines, targets) = keys_lines(true, cols);
      assert_eq!(lines.len(), targets.len());
      for l in &lines {
        if let L::Row { a, b, .. } | L::Tint { a, b, .. } = l {
          let w = a.chars().count() + if b.is_empty() { 0 } else { 2 + b.chars().count() };
          assert!(w <= cols, "{cols} columns: {a:?} {b:?}");
        }
      }
      assert_eq!(words(&lines), words(&wide), "{cols} columns: every word is still there, in order");
    }
    assert_eq!(wrap("a bb ccc dddd", 6), ["a bb", "ccc", "dddd"]);
    assert_eq!(wrap("toolongaword x", 4), ["toolongaword", "x"]);
  }
}
