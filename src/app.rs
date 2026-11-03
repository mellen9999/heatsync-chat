// window + event loop glue: tabs of chat views fed by the net thread
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime as WinIme, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::platform::x11::WindowAttributesExtX11;
use winit::window::{Window, WindowId};

mod acct;
mod blocked;
mod builtin;
mod card;
mod chips;
mod clip;
mod complete;
mod emote;
mod engage;
mod feed;
mod filters;
mod find;
mod gifs;
mod held;
mod img_view;
mod link;
mod logs;
mod media_peek;
mod mentions;
mod mods;
mod modsuite;
mod modtools;
mod notes;
mod peek;
mod picker;
mod pin;
mod pop;
mod pronouns;
mod quick;
mod rover;
mod screen;
mod stats;
mod status;
mod stream;
mod suggest;
mod tabs;
mod tagcard;
mod tip;
mod unseen;
mod update;
mod upload;
mod whispers;
mod zoom;

use crate::cosmetics::id::PaintId;
use crate::fake::Fake;
use crate::model::chan::{self, Form};
use crate::model::complete::Complete;
use crate::model::feed::Feed;
use crate::model::frecency::Frecency;
use crate::model::ime::Ime;
use crate::model::line::Line;
use crate::model::mention::Mentions;
use crate::model::message::{Message, Platform, flag};
use crate::model::picker::{Focus, Picker, Recents};
use crate::model::pin::Pins;
use crate::model::settings::{Counter, Local, Settings, Side, local_cmd};
use crate::model::stream_events::Moments;
use crate::model::suggest::Suggest;
use crate::model::tabs::Kind;
use crate::model::vi::{Action, Mode, Vi};
use crate::model::whisper::{self, Timeline};
use crate::net::auth::Me;
use crate::net::hs_ws::Cmd as HsCmd;
use crate::net::irc::Chat;
use crate::net::{Cmd, Net, NetEvent, Reply};
use crate::store;
use crate::ui::canvas::{Canvas, Rect};
use crate::ui::chat_view::{ChatView, Paint};
use crate::ui::form::Hit;
use crate::ui::input_bar;
use crate::ui::mode_label;
use crate::ui::res::Res;
use crate::ui::tab_bar::{self, TabLabel};
use crate::ui::union;
use crate::util::time::now_ms;

const FLOOD_EVERY: Duration = Duration::from_millis(50);
const FLOOD_TAB: &str = "flood";
// matches packaging/heatsync-chat.desktop (StartupWMClass) and its icon name
const APP_ID: &str = "heatsync-chat";
const SCROLLBACK: usize = 1000;
// animation repaint ceiling: 30fps — smooth for emotes, ~35% fewer wakeups than native gif timing
const ANIM_MIN_MS: u32 = 33;
const TRIM_AFTER: Duration = Duration::from_secs(2);

struct Gfx {
  window: Rc<Window>,
  surface: Surface<Rc<Window>, Rc<Window>>,
}

// platform tag text + color shown before the input on merged tabs
type Tag = (&'static str, u32);

struct Tab {
  name: Box<str>,
  kind: Kind,
  view: ChatView,
  unread: bool,
  // a stream line (live, offline, game switch) landed while you were elsewhere: the tab reads yellow
  event: bool,
  // every feed merged into this tab: (platform, routing key)
  sources: Vec<(Platform, Box<str>)>,
  // which source the input sends to (ctrl-n cycles on merged tabs)
  target: usize,
  // something took the tab over (a feed thread): `view` shows it, the tab's own lines wait here, still
  // filling, and the label reads `← back` until it closes
  under: Option<ChatView>,
  // the number after its name (channel tabs)
  count: crate::model::tab_count::Count,
  // its stream stats (/summary)
  stats: stats::Count,
}

impl Tab {
  // the tab's own lines, wherever they are right now: chat lands here even while a thread covers it
  fn chat(&mut self) -> &mut ChatView {
    self.under.as_mut().unwrap_or(&mut self.view)
  }

  // both, for what every line must hear (a catalog, a paint, a width)
  fn views(&mut self) -> impl Iterator<Item = &mut ChatView> {
    std::iter::once(&mut self.view).chain(self.under.as_mut())
  }
}

pub struct App {
  gfx: Option<Gfx>,
  canvas: Canvas,
  res: Res,
  tabs: Vec<Tab>,
  active: usize,
  net: Net,
  bar_dirty: bool,
  // the search's count ("3/10", "no match"), at the input bar's right
  find_note: String,
  mods: ModifiersState,
  // vi normal: space was pressed, the next key is a chat command
  chord: Option<char>,
  fake: Fake,
  flood: Option<Instant>,
  prof: bool,
  // twitch room id per channel, once known (roomstate repeats on every mode change)
  rooms: Vec<(Box<str>, Box<str>)>,
  // twitch channels where our badges say we moderate (or own) the room
  mod_in: std::collections::HashSet<Box<str>>,
  // each twitch room's chat modes as its ROOMSTATE says, and a mode change on its way
  modes: std::collections::HashMap<Box<str>, crate::model::modes::Modes>,
  modes_busy: bool,
  // when the soonest timed mute ends
  mute_due: Option<Instant>,
  next_anim: Option<Instant>,
  // the emote hover popup, and when its animated emote flips next
  tip: Option<crate::ui::tip::Tip>,
  tip_next: Option<Instant>,
  // heat counts fall with the clock: the tab bar repaints each second while one shows
  heat_due: Option<Instant>,
  // youtube lines waiting their turn, and when the next one is due
  pacer: crate::model::pace::Pacer,
  pace_due: Option<Instant>,
  // the tab tooltip or a suggestion list sat over the chat last frame (repaint under it)
  over_input: bool,
  // the open `:` `@` `/` list, the text and cursor it was made for, its rows on screen (index, rect)
  suggest: Option<Suggest>,
  suggest_at: (Box<str>, usize),
  suggest_rows: Vec<(usize, Rect)>,
  // where the box's text was last drawn (a click places the cursor)
  input_hit: Option<input_bar::Hit>,
  // an animated emote in the message box changes frame
  input_next: Option<Instant>,
  // the name hover popup, when the pointer rested long enough to ask for it, and what heatsync said
  peek: Option<peek::Peek>,
  media_peek: Option<media_peek::MediaPeek>,
  peek_due: Option<Instant>,
  cards: peek::Cards,
  pronouns: crate::model::pronouns::Pronouns,
  // #tag hover cards: what heatsync said, the open one, and when a resting pointer asks
  tag_cards: crate::model::tagcard::Cards,
  tag_peek: Option<tagcard::TagPeek>,
  tag_due: Option<Instant>,
  // automod holds in rooms we moderate
  held: held::Queue,
  notes: crate::model::notes::Notes,
  occluded: bool,
  // the window was resized or shown again (after a fullscreen app, a hidden tag): the buffer or the
  // compositor's copy of it may hold nothing, so the next present sends the whole frame
  expose: bool,
  // damage of recent presents, for buffers handed back with age > 1
  damage_hist: [Option<Rect>; 2],
  // pending heap trim after bursts of parsing (emote sets, history)
  trim_at: Option<Instant>,
  // the message being typed. mode: with vi keys off it is always Insert (a plain text box);
  // with them on, Chat drives the chat, Insert types, Normal is vim on the box
  line: Line,
  mode: Mode,
  vi: Vi,
  // the vi mode line last drawn: a change repaints the bar
  shown_mode: (Vec<mode_label::Seg>, String, bool),
  set: Settings,
  complete: Option<Complete>,
  // replying to this line, sent with the next one
  reply: Option<Reply>,
  // last pointer position (window px), for clicks
  pointer: (i32, i32),
  input_h: i32,
  input_dirty: bool,
  me: Option<Me>,
  // settings that follow the heatsync account: what it holds, whether the heatsync socket is signed in, and
  // when the settings were last asked for
  acct: crate::model::sync::Acct,
  hs_authed: bool,
  ui_get_at: Option<Instant>,
  // input-method composition, whether the window currently lets one run, and the caret rect
  ime: Ime,
  ime_on: bool,
  ime_area: Option<Rect>,
  ime_sent: Option<Rect>,
  // shift+right-click menu on an emote
  menu: Option<emote::Open>,
  // the add/edit channel form (covers the chat), and the menu on a right-clicked tab
  form: Option<Form>,
  form_hover: Option<Hit>,
  // the pointer is a hand: it sits on a link
  // the pointer's look: a hand over a link, a magnifier over a picture or the open view
  cursor: winit::window::CursorIcon,
  // a picture filling the chat area
  img_view: Option<img_view::ImgView>,
  // one person's archived lines over the active tab (the tab's own lines wait in `Tab::under`)
  logs: Option<crate::model::logs::Logs>,
  // tab mode's cursor on the tab bar (space t)
  rover: Option<rover::Rover>,
  tab_menu: Option<tabs::TabMenu>,
  // the floating menu for login and line actions
  pop: Option<pop::Pop>,
  // the user card over the chat
  card: Option<card::Card>,
  // the settings screen or the key map, covering the chat
  screen: Option<screen::Screen>,
  // last drawn tab rects, then the + button
  bar_hits: Vec<Rect>,
  // the tab index behind each drawn rect (hidden tabs have none)
  bar_map: Vec<usize>,
  plus_hover: bool,
  gear_hover: bool,
  update_hover: bool,
  // a newer release, once found; installing it; when to look again
  update: Option<crate::net::update::Release>,
  updating: bool,
  update_due: Option<Instant>,
  // the title flashing who @'d you while the window is away: their name, and when it next turns
  flash: Option<(Box<str>, bool)>,
  flash_due: Option<Instant>,
  // display scale factor (zoom multiplies it), and smooth-wheel travel toward a zoom step
  dpi: f32,
  zoom_acc: f32,
  picker: Option<Picker>,
  gifs: gifs::State,
  recents: Recents,
  // how often each emote gets picked: tab ranks by it
  frecency: Frecency,
  // pointer is over the logo button
  btn_hover: bool,
  up_hover: bool,
  // the built-in path prompt (no file dialog installed), and an upload or dialog in flight
  prompt: Option<Line>,
  // that prompt is a chat search, not a path; and the search as last kept with enter
  finding: bool,
  find_q: String,
  uploading: bool,
  // the system clipboard, made on first use
  clip: Option<arboard::Clipboard>,
  // pinned lines per room, the bar's height (0 = none), whether it is open, and what it last drew
  pins: Pins,
  pin_h: i32,
  pin_open: bool,
  pin_sig: (Option<Box<str>>, bool, i32, i32),
  moments: Moments,
  // followed streamers' twitch colors (`follow:colors`), by lowercase login
  follow_colors: std::collections::HashMap<Box<str>, u32>,
  // names that ping you, and whether the window has focus (notifications only fire when it does not)
  mentions: Mentions,
  focused: bool,
  // the whispers timeline, and whether this login's history was asked for
  whispers: Timeline,
  whispers_loaded: bool,
  unseen: crate::model::unseen::Unseen,
  // bumped on every login change: a history read from an earlier one is dropped
  whisper_epoch: u32,
  // the feed tab's posts and open thread; the text of a post in flight (put back if it fails)
  feed: Feed,
  pending_post: Option<String>,
  // your own post being edited (its words are in the box), and the post a delete menu is asking about
  edit: Option<engage::Edit>,
  pending_delete: Option<Box<str>>,
}

impl App {
  pub fn new(net: Net, channels: Vec<Box<str>>, flood: bool) -> Self {
    let mut tabs: Vec<Tab> = Vec::new();
    let mut specs = Vec::new();
    for line in channels {
      // a saved line, or an argv word: `forsen`, `kick:slug`, `yt:@handle`, or several merged
      let Ok(spec) = chan::Spec::from_line(&line) else { continue };
      if spec.check(&specs).is_err() {
        continue;
      }
      tabs.push(tabs::make_tab(&net, &spec));
      specs.push(spec);
    }
    if flood {
      tabs.push(Tab {
        name: FLOOD_TAB.into(),
        kind: Kind::Chat,
        view: ChatView::new(SCROLLBACK, FLOOD_TAB.into()),
        unread: false,
        event: false,
        sources: Vec::new(),
        target: 0,
        under: None,
        count: Default::default(),
        stats: Default::default(),
      });
    }
    let set = Settings::load();
    let acct = crate::model::sync::Acct::new(&set);
    crate::heatsync::content::set_shown(set.content());
    crate::model::filter::edit(|f| {
      *f = crate::model::filter::Filters::load();
      (f.bots, f.cmds, f.dupes, f.caps) = (set.hide_bots, set.hide_cmds, set.hide_dupes, set.automod_caps);
    });
    let mute_due = crate::model::filter::with(|f| f.next_expiry(now_ms()))
      .map(|at| Instant::now() + Duration::from_millis((at - now_ms()).max(0) as u64 + 50));
    for t in &mut tabs {
      t.view.glow = set.first_glow;
    }
    // the ext's strip: feed, whispers, mentions, then your channels. you land on the first channel
    let active = if tabs.is_empty() { 0 } else { crate::model::tabs::BUILTIN.len() };
    tabs.splice(0..0, builtin::make_all());
    let mut res = Res::new(1.0);
    res.cosmetics.on = set.paints;
    res.cosmetics.hidden = crate::cosmetics::hidden::Hidden::load();
    for t in &tabs {
      tabs::seat_hosts(&mut res.chatters, &t.sources);
    }
    res.arts.set_motion(set.paint_motion);
    if flood {
      crate::fake::seed_paints(&mut res.cosmetics);
    }
    res.media = set.media;
    res.look = crate::ui::res::Look::of(&set);
    res.partial_links = set.partial_links;
    Self {
      gfx: None,
      canvas: Canvas::new(0, 0),
      res,
      tabs,
      active,
      net,
      bar_dirty: true,
      find_note: String::new(),
      mods: ModifiersState::empty(),
      chord: None,
      fake: Fake::new(0x5eed),
      flood: flood.then(Instant::now),
      prof: std::env::var_os("HSC_PROF").is_some(),
      rooms: Vec::new(),
      mod_in: Default::default(),
      modes: Default::default(),
      modes_busy: false,
      mute_due,
      next_anim: None,
      tip: None,
      tip_next: None,
      heat_due: None,
      pacer: Default::default(),
      pace_due: None,
      input_next: None,
      input_hit: None,
      over_input: false,
      suggest: None,
      suggest_at: ("".into(), 0),
      suggest_rows: Vec::new(),
      peek: None,
      media_peek: None,
      peek_due: None,
      cards: peek::Cards::default(),
      pronouns: Default::default(),
      tag_cards: Default::default(),
      tag_peek: None,
      tag_due: None,
      held: Default::default(),
      notes: crate::model::notes::Notes::load(),
      occluded: false,
      expose: false,
      damage_hist: [None; 2],
      trim_at: None,
      line: Line::default(),
      mode: if set.vi { Mode::Normal } else { Mode::Insert },
      vi: Vi::default(),
      shown_mode: Default::default(),
      set,
      complete: None,
      reply: None,
      pointer: (0, 0),
      input_h: 0,
      input_dirty: true,
      me: None,
      acct,
      hs_authed: false,
      ui_get_at: None,
      ime: Ime::default(),
      ime_on: false,
      ime_area: None,
      ime_sent: None,
      menu: None,
      form: None,
      form_hover: None,
      cursor: winit::window::CursorIcon::Default,
      img_view: None,
      logs: None,
      rover: None,
      tab_menu: None,
      pop: None,
      card: None,
      screen: None,
      bar_hits: Vec::new(),
      bar_map: Vec::new(),
      plus_hover: false,
      gear_hover: false,
      update_hover: false,
      update: None,
      updating: false,
      update_due: Some(Instant::now() + update::FIRST_CHECK),
      flash: None,
      flash_due: None,
      dpi: 1.0,
      zoom_acc: 0.0,
      picker: None,
      gifs: Default::default(),
      recents: Recents::load(),
      frecency: Frecency::load(Recents::load().names(), now_ms()),
      btn_hover: false,
      up_hover: false,
      prompt: None,
      finding: false,
      find_q: String::new(),
      clip: None,
      uploading: false,
      pins: Pins::default(),
      pin_h: 0,
      pin_open: false,
      pin_sig: (None, false, 0, 0),
      moments: Moments::default(),
      follow_colors: Default::default(),
      mentions: Mentions::default(),
      focused: true,
      whispers: Timeline::default(),
      whispers_loaded: false,
      unseen: Default::default(),
      whisper_epoch: 0,
      feed: Feed::default(),
      pending_post: None,
      edit: None,
      pending_delete: None,
    }
  }

  // does the focused thing take typed text (so an input method may run)?
  fn wants_text(&self) -> bool {
    if self.form.is_some() || self.screen.as_ref().is_some_and(|s| s.typing()) {
      return true;
    }
    match &self.picker {
      Some(p) => p.focus == Focus::Search,
      None => self.prompt.is_some() || self.mode == Mode::Insert,
    }
  }

  // keep the window's input-method state in step: allowed only while text is accepted,
  // candidate window parked at the caret
  fn sync_ime(&mut self) {
    let want = self.wants_text();
    if !want && self.ime.composing() {
      self.ime.clear();
      self.input_dirty = true;
      self.wake();
    }
    let Some(g) = &self.gfx else { return };
    if want != self.ime_on {
      self.ime_on = want;
      g.window.set_ime_allowed(want);
      self.ime_sent = None;
    }
    if want && self.ime_area != self.ime_sent {
      self.ime_sent = self.ime_area;
      if let Some(r) = self.ime_area {
        g.window.set_ime_cursor_area(
          PhysicalPosition::new(r.x, r.y),
          PhysicalSize::new(r.w.max(1) as u32, r.h.max(1) as u32),
        );
      }
    }
  }

  fn on_ime(&mut self, ev: WinIme) {
    match ev {
      WinIme::Enabled => self.ime_sent = None,
      WinIme::Preedit(text, cursor) => {
        if self.wants_text() {
          self.ime.set(&text, cursor);
          self.input_dirty = true;
          self.wake();
        }
      }
      WinIme::Commit(text) => {
        let text = self.ime.commit(&text);
        if self.wants_text() && !text.is_empty() {
          if let Some(f) = &mut self.form {
            f.fields[f.focus].insert(&text);
            f.err = None;
          } else if let Some(sc) = self.screen.as_mut().filter(|s| s.typing()) {
            sc.query_mut().insert(&text);
          } else if let Some(p) = &mut self.picker {
            p.type_str(&text);
            self.picker_reveal();
          } else if let Some(p) = &mut self.prompt {
            p.insert(&text);
            if self.finding {
              self.find_live();
            }
          } else {
            self.complete = None;
            self.line.checkpoint(false);
            self.line.insert(&text);
          }
        }
        self.input_dirty = true;
        self.wake();
      }
      WinIme::Disabled => {
        self.ime.clear();
        self.input_dirty = true;
        self.wake();
      }
    }
  }

  // the tab bar: a row on top or at the bottom, a column on the left or right (ext tabPosition)
  fn bar(&self) -> Rect {
    let (w, h) = (self.canvas.w, self.canvas.h);
    let line = self.res.text.line_px as i32;
    // a column is about 13 monospace cells (the ext's 90px), never more than half the window
    let col = ((self.res.text.font_px * 8.0) as i32).min(w / 2);
    match self.set.tab_side {
      Side::Top => Rect::new(0, 0, w, line),
      Side::Bottom => Rect::new(0, (h - line).max(0), w, line),
      Side::Left => Rect::new(0, 0, col, h),
      Side::Right => Rect::new(w - col, 0, col, h),
    }
  }

  // what the tab bar leaves: the pin, the chat and the input bar stack in here
  pub(super) fn body(&self) -> Rect {
    let (w, h, b) = (self.canvas.w, self.canvas.h, self.bar());
    match self.set.tab_side {
      Side::Top => Rect::new(0, b.h, w, h - b.h),
      Side::Bottom => Rect::new(0, 0, w, h - b.h),
      Side::Left => Rect::new(b.w, 0, w - b.w, h),
      Side::Right => Rect::new(0, 0, w - b.w, h),
    }
  }

  fn chat_area(&self) -> Rect {
    let b = self.body();
    let top = b.y + self.pin_h;
    Rect::new(b.x, top, b.w, (b.bottom() - top - self.input_h).max(0))
  }

  fn input_area(&self) -> Rect {
    let b = self.body();
    Rect::new(b.x, (b.bottom() - self.input_h).max(b.y), b.w, self.input_h)
  }

  // the tab bar moved: everything below it re-lays out at the new width
  pub(super) fn side_to(&mut self, side: Side) {
    if side != self.set.tab_side {
      self.set.tab_side = side;
      self.set.save();
      self.relayout_body();
    }
  }

  pub(super) fn counter_to(&mut self, c: Counter) {
    if c != self.set.tab_counter {
      self.set.tab_counter = c;
      self.set.save();
      self.bar_dirty = true;
      self.wake();
    }
  }

  fn pace_next(&mut self) {
    let now = now_ms();
    self.pace_due = self.pacer.next().map(|t| Instant::now() + Duration::from_millis((t - now).max(0) as u64));
  }

  fn pace_tick(&mut self) {
    for (key, msgs) in self.pacer.due(now_ms()) {
      self.relay_show(Platform::Youtube, &key, msgs, false);
    }
    self.pace_next();
  }

  // a chat message landed in tab `i`: its counter moves (the bar repaints when the number shown does)
  fn bump_count(&mut self, i: usize) {
    let active = i == self.active;
    self.tabs[i].count.bump(active, now_ms() / 1000);
    match self.set.tab_counter {
      Counter::Unread if !active => self.bar_dirty = true,
      Counter::Heat if self.heat_due.is_none() => self.bar_dirty = true,
      _ => {}
    }
  }

  // the space under the tabs changed size or place: views take its width, the whole frame repaints
  pub(super) fn relayout_body(&mut self) {
    let w = self.body().w;
    for v in self.tabs.iter_mut().flat_map(Tab::views) {
      v.resize(w);
      v.invalidate();
    }
    self.canvas.fill(self.canvas.bounds(), crate::ui::theme::BLACK);
    self.pin_sig = Default::default();
    self.damage_hist = [None; 2];
    self.expose = true;
    self.bar_dirty = true;
    self.input_dirty = true;
    self.wake();
  }

  fn hint(&self) -> String {
    if !self.tabs.iter().any(|t| t.kind == Kind::Chat) {
      return "no channels — click + or press ctrl-t to add one".into();
    }
    if self.kind() == Kind::Mentions {
      return "enter or a click opens the line in its channel".into();
    }
    if let Some(l) = self.logs_shown() {
      return l.hint();
    }
    if self.feedish() {
      return self.feed_hint();
    }
    if self.kind() == Kind::Whispers {
      return match self.whisper_label() {
        Some(to) => format!("type to whisper {to}· /w user msg"),
        None => "/w user msg to whisper someone".into(),
      };
    }
    let lead = if self.set.vi { "i type" } else { "type to chat" };
    let Some(me) = &self.me else { return format!("ctrl-l log in · {lead}") };
    let Some((p, ..)) = self.target() else { return lead.into() };
    match me.name_on(p) {
      Some(name) => format!("{lead} as {name}"),
      None => format!("{} isn't linked · ctrl-l to link it", p.name()),
    }
  }

  // auto-hide: an empty bar takes no room (mode and reply stay as they are)
  fn hidden(&self) -> bool {
    self.set.auto_hide && self.line.text.is_empty() && !self.ime.composing() && self.prompt.is_none()
  }

  // where the input sends: the tab's target source, tagged when the tab merges platforms
  fn target(&self) -> Option<(Platform, &str, Option<Tag>)> {
    let t = self.tabs.get(self.active)?;
    let (p, key) = t.sources.get(t.target)?;
    let tag = match p {
      Platform::Twitch => ("[T] ", crate::ui::theme::TWITCH),
      Platform::Kick => ("[K] ", crate::ui::theme::GREEN),
      Platform::Youtube => ("[Y] ", crate::ui::theme::RED),
    };
    Some((*p, key, (t.sources.len() > 1).then_some(tag)))
  }

  fn set_mode(&mut self, m: Mode) {
    self.mode = m;
    self.input_dirty = true;
    self.wake();
  }

  // vi: back to normal, the cursor stepping back onto a char
  fn leave_insert(&mut self) {
    self.sync_overlays();
    self.vi.end_insert(&mut self.line);
    self.set_mode(Mode::Normal);
  }

  fn enter_insert(&mut self) {
    if self.mode != Mode::Insert {
      self.vi.begin_insert(&self.line);
    }
    self.set_mode(Mode::Insert);
  }

  // after a send or local command: fresh undo history
  fn after_send(&mut self) {
    self.vi.reset();
    if self.set.vi && self.mode == Mode::Insert {
      self.vi.begin_insert(&self.line);
    }
    self.input_dirty = true;
    self.wake();
  }

  // slash commands that are ours: switches from the schema table, zoom, tabs, the two screens. never sent anywhere
  fn local(&mut self, cmd: Local) {
    match cmd {
      Local::Toggle(id) => {
        if let Some(d) = crate::model::schema::def(id) {
          self.flip(d);
        }
      }
      Local::Tab(Some(k), shown) => self.set_shown(k, shown),
      Local::Tab(None, shown) => {
        self.say(format!("usage: /{} feed, whispers or mentions", if shown { "show" } else { "hide" }))
      }
      Local::Zoom(Some(pct)) => self.zoom_to(pct),
      Local::Zoom(None) => self.say("usage: /zoom 1.5 (anything from 0.5 to 3)"),
      Local::Side(Some(side)) => self.side_to(side),
      Local::Side(None) => self.say("usage: /tabs top, bottom, left or right"),
      Local::Pick(id, Some(i)) => {
        if let Some(d) = crate::model::schema::def(id) {
          self.pick_to(d, i);
        }
      }
      Local::Pick(id, None) => {
        if let Some(d) = crate::model::schema::def(id)
          && let (Some(cmd), crate::model::schema::Ctl::Pick(c, ..)) = (d.cmd, d.ctl)
        {
          self.say(format!("usage: {cmd} {}", c.usage));
        }
      }
      Local::Counter(Some(c)) => self.counter_to(c),
      Local::Counter(None) => self.say("usage: /tabcounter unread, heat or off"),
      Local::Settings => self.open_screen(screen::Page::Settings),
      Local::Keys => self.open_screen(screen::Page::Keys),
      Local::Blocked => self.blocked_here(),
      Local::Summary => self.summary_here(),
      Local::Status => self.status_here(),
      Local::ModTools => self.tools_here(),
      Local::Lclear => self.lclear(),
      Local::User(name) => self.user_cmd(&name),
      Local::Goto(q) => self.goto_tab(&q),
      Local::Set(k, v) => self.set_cmd(&k, &v),
    }
  }

  fn send(&mut self) {
    if self.logs_shown().is_some() {
      return self.say("you're reading chat logs: esc goes back");
    }
    if self.edit.is_some() {
      return self.edit_send();
    }
    self.faces();
    if let Some(cmd) = local_cmd(&self.line.text) {
      self.line.take();
      self.local(cmd);
      return self.after_send();
    }
    if let Some(cmd) = chan::command(&self.line.text) {
      self.line.take();
      self.chan_cmd(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::filtercmd::command(&self.line.text) {
      self.line.take();
      self.filter_cmd(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::notes::command(&self.line.text) {
      self.line.take();
      self.note_cmd(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::modes::command(&self.line.text) {
      self.line.take();
      self.modes_slash(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::modsuite::command(&self.line.text) {
      self.line.take();
      self.suite_slash(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::modcmd::command(&self.line.text) {
      self.line.take();
      self.mod_slash(cmd);
      return self.after_send();
    }
    if let Some(cmd) = crate::model::postcmd::command(&self.line.text) {
      self.line.take();
      self.post_slash(cmd);
      return self.after_send();
    }
    if self.feedish() && !self.line.text.trim().is_empty() {
      self.feed_post();
      return self.after_send();
    }
    if let Some(cmd) = whisper::command(&self.line.text) {
      self.line.take();
      self.whisper_slash(cmd);
      return self.after_send();
    }
    if self.kind() == Kind::Whispers {
      if let Some(text) = self.line.take() {
        self.reply_whisper(text);
      }
      return self.after_send();
    }
    if self.kind() == Kind::Mentions {
      return self.say("mentions is read-only. open a line to answer it");
    }
    let Some((platform, chan)) = self.target().map(|(p, k, _)| (p, Box::<str>::from(k))) else { return };
    if let Some(text) = self.line.take() {
      let reply = self.reply.take();
      if let Some(v) = self.view() {
        v.deselect();
      }
      let text = crate::model::emoji::convert(&text).into_owned();
      self.net.send(Cmd::Send { platform, chan, text, reply });
      if let Some(v) = self.view() {
        v.scroll_to_bottom();
      }
      self.after_send();
    }
  }

  fn wake(&self) {
    if let Some(g) = &self.gfx {
      g.window.request_redraw();
    }
  }

  fn kind(&self) -> Kind {
    self.tabs.get(self.active).map_or(Kind::Chat, |t| t.kind)
  }

  fn view(&mut self) -> Option<&mut ChatView> {
    self.tabs.get_mut(self.active).map(|t| &mut t.view)
  }

  fn scroll(&mut self, px: i32) {
    let res = &mut self.res;
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.scroll_by(px, res);
    }
    self.logs_edge();
    self.wake();
  }

  fn select(&mut self, i: usize) {
    if i < self.tabs.len() && i != self.active {
      self.img_close();
      self.find_clear();
      self.hide_tip();
      self.rover = None;
      // an edit belongs to the tab it was started in
      self.edit_end();
      self.active = i;
      // a card or menu belongs to the tab it was opened in
      self.card = None;
      self.pop = None;
      self.tabs[i].unread = false;
      self.tabs[i].event = false;
      self.tabs[i].count.read();
      if let Some(s) = unseen::surface(self.tabs[i].kind) {
        self.unseen_clear(s);
      }
      self.tabs[i].view.invalidate();
      self.bar_dirty = true;
      self.input_dirty = true;
      self.entered();
      self.wake();
    }
  }

  // ctrl-w: leave a channel; a built-in tab just hides (/show brings it back)
  fn close_tab(&mut self) {
    match self.tabs.get(self.active).map(|t| t.kind) {
      Some(Kind::Chat) => self.remove_tab(self.active),
      Some(k) => self.set_shown(k, false),
      None => {}
    }
  }

  // a catalog changed: rows in that channel (None = all) need new layouts
  fn relayout_where(&mut self, ch: Option<&str>) {
    for t in &mut self.tabs {
      if ch.is_none_or(|c| *c == *t.name) {
        t.views().for_each(ChatView::relayout);
      }
    }
    self.wake();
  }

  fn touch_sender(&mut self, id: u64) {
    let uid = id.to_string();
    for v in self.tabs.iter_mut().flat_map(Tab::views) {
      v.touch_sender(&uid);
    }
    self.wake();
  }

  // a paint or badge landed: redo only the lines whose name it dresses
  fn touch_paint(&mut self, id: &PaintId) {
    for v in self.tabs.iter_mut().flat_map(Tab::views) {
      v.touch_paint(id);
    }
    self.wake();
  }

  fn trim_soon(&mut self) {
    self.trim_at = Some(Instant::now() + TRIM_AFTER);
  }

  fn tabs_with(&self, p: Platform, key: &str) -> Vec<usize> {
    (0..self.tabs.len()).filter(|&i| self.tabs[i].sources.iter().any(|s| s.0 == p && *s.1 == *key)).collect()
  }

  fn touched(&mut self, i: usize) {
    if i == self.active {
      self.wake();
    } else if !self.tabs[i].unread {
      self.tabs[i].unread = true;
      self.bar_dirty = true;
      self.wake();
    }
  }

  fn on_relay(&mut self, p: Platform, key: &str, mut msgs: Vec<Message>, history: bool) {
    for m in &mut msgs {
      if history {
        m.flags |= flag::HISTORY;
      }
      self.res.chatters.heard(m);
      self.scan_mention(p, key, m);
    }
    // youtube's batches drip out at their real pace (model::pace); the rest waits for pace_due
    if p == Platform::Youtube && !history {
      msgs = self.pacer.push(key, msgs, now_ms());
      self.pace_next();
    }
    self.relay_show(p, key, msgs, history);
  }

  // lines for every tab carrying that channel: history at the top, live at the bottom
  fn relay_show(&mut self, p: Platform, key: &str, msgs: Vec<Message>, history: bool) {
    if msgs.is_empty() {
      return;
    }
    if !history {
      for m in &msgs {
        self.logs_live(p, key, m);
      }
    }
    for i in self.tabs_with(p, key) {
      let v = self.tabs[i].chat();
      if history {
        v.prepend(msgs.clone());
        if i == self.active {
          self.wake();
        }
      } else {
        for m in &msgs {
          v.push(m.clone());
        }
        for m in &msgs {
          self.tally(i, key, m);
          self.bump_count(i);
        }
        self.touched(i);
      }
    }
  }

  fn on_links(&mut self, tab: &str, kick: Option<Box<str>>, yt: Option<(Box<str>, Option<Box<str>>)>) {
    let Some(i) = self.tabs_with(Platform::Twitch, tab).first().copied() else { return };
    let mut add = Vec::new();
    if let Some(slug) = kick {
      self.net.send(Cmd::Hs(HsCmd::Join(Platform::Kick, slug.clone())));
      self.net.send(Cmd::Backfill(Platform::Kick, slug.clone(), slug.clone()));
      add.push((Platform::Kick, slug));
    }
    if let Some((handle, ucid)) = yt {
      self.net.send(Cmd::Hs(HsCmd::YtSub(handle.clone())));
      if let Some(uc) = ucid {
        self.net.send(Cmd::Backfill(Platform::Youtube, handle.clone(), uc));
      }
      add.push((Platform::Youtube, handle));
    }
    let t = &mut self.tabs[i];
    for src in add {
      if !t.sources.contains(&src) {
        t.sources.push(src);
      }
    }
    let tags = t.sources.len() > 1;
    let v = t.chat();
    v.tags = tags;
    v.relayout();
    self.input_dirty = true;
    if i == self.active {
      self.wake();
    }
  }

  fn on_chat(&mut self, ch: &str, ev: Chat) {
    let ev = match ev {
      Chat::Msg(mut m) if ch != FLOOD_TAB => {
        if self.res.chatters.heard(&m) {
          for v in self.tabs.iter_mut().flat_map(Tab::views) {
            v.adopt_id(&m.login, &m.user_id);
          }
        }
        self.scan_mention(Platform::Twitch, ch, &mut m);
        self.logs_live(Platform::Twitch, ch, &m);
        Chat::Msg(m)
      }
      ev => ev,
    };
    let i = if ch == FLOOD_TAB {
      self.tabs.iter().position(|t| &*t.name == FLOOD_TAB)
    } else {
      self.tabs_with(Platform::Twitch, ch).first().copied()
    };
    let Some(i) = i else { return };
    if let Chat::Msg(m) = &ev {
      self.tally(i, ch, m);
    }
    let tab = self.tabs[i].chat();
    match ev {
      Chat::Msg(m) => {
        tab.push(m);
        self.bump_count(i);
      }
      Chat::ClearMsg(id, login) => {
        tab.clear_msg(&id);
        let who = if login.is_empty() { "a message".to_owned() } else { format!("{login}'s message") };
        self.mod_log(Platform::Twitch, ch, format!("deleted {who}"));
      }
      Chat::ClearUser { login, secs } => {
        tab.clear_user(&login);
        let what = secs.map_or_else(|| "banned".to_owned(), |s| format!("timed out for {s}s"));
        tab.push(Message::system(now_ms(), Platform::Twitch, format!("{login} was {what}")));
        self.mod_log(Platform::Twitch, ch, format!("{login} was {what}"));
      }
      Chat::ClearAll => tab.push(Message::system(now_ms(), Platform::Twitch, "chat was cleared by a moderator")),
      Chat::Room { id, modes } => {
        self.modes_seen(ch, modes);
        if !self.rooms.iter().any(|r| *r.0 == *ch) {
          self.rooms.push((ch.into(), id.clone()));
          self.net.send(Cmd::Room(ch.into(), id));
          self.held_sweep();
        }
        return;
      }
      Chat::Mod(on) => return self.on_mod_state(ch, on),
    }
    if i == self.active {
      self.wake();
    } else if !self.tabs[i].unread {
      self.tabs[i].unread = true;
      self.bar_dirty = true;
      self.wake();
    }
  }

  // the vi mode line for what the box is doing right now, and the half-typed command
  fn mode_line(&self) -> (Vec<mode_label::Seg>, String) {
    if self.prompt.is_some() {
      return (Vec::new(), if self.finding { self.find_note.clone() } else { String::new() });
    }
    if let Some(note) = self.rover_note() {
      return (Vec::new(), note);
    }
    let short = !self.line.text.is_empty() && !self.hidden();
    let segs = mode_label::segs(self.set.vi, self.mode, self.selecting(), short);
    let cmd = match self.chord {
      _ if segs.is_empty() || self.mode != Mode::Normal => String::new(),
      Some(' ') => "space".into(),
      _ => Some(self.vi.showcmd()).filter(|c| !c.is_empty()).unwrap_or_else(|| self.find_note.clone()),
    };
    (segs, cmd)
  }

  fn redraw(&mut self) {
    let mut dirty: Option<Rect> = None;
    let (label, showcmd) = self.mode_line();
    let hidden_now = self.hidden();
    if (&label, &showcmd, hidden_now) != (&self.shown_mode.0, &self.shown_mode.1, self.shown_mode.2) {
      self.shown_mode = (label.clone(), showcmd.clone(), hidden_now);
      self.input_dirty = true;
      self.bar_dirty = true;
    }
    if self.input_next.is_some_and(|t| t <= Instant::now()) {
      self.input_next = None;
      self.input_dirty = true;
    }
    if std::mem::take(&mut self.input_dirty) {
      self.sync_suggest();
      let chips = if self.prompt.is_some() { Vec::new() } else { self.input_chips() };
      let show_hint = self.line.text.is_empty() && (!self.set.vi || self.mode == Mode::Normal);
      let hint = match &self.prompt {
        Some(_) if self.finding && self.logs_shown().is_some() => {
          "type a search · enter finds it · esc cancels".to_owned()
        }
        Some(_) if self.finding => "type to filter · @name for one person · enter keeps it · esc clears".to_owned(),
        Some(_) => "type or drop a file path · tab completes · enter uploads · esc cancels".to_owned(),
        None if show_hint => self.hint(),
        None => String::new(),
      };
      // the picture view is no place to type (ext): the box steps aside
      let hidden = self.hidden() || self.img_view.is_some();
      let tag = match &self.prompt {
        Some(_) if self.finding => Some(("find: ", crate::ui::theme::YELLOW)),
        Some(_) => Some(("path: ", crate::ui::theme::YELLOW)),
        None => self.target().and_then(|t| t.2),
      };
      let reply = self.reply.as_ref().filter(|_| self.prompt.is_none()).map(|r| &*r.nick);
      let bar = input_bar::Bar {
        line: self.prompt.as_ref().unwrap_or(&self.line),
        mode: if self.prompt.is_some() { Mode::Insert } else { self.mode },
        target: tag,
        hint: &hint,
        reply,
        edit: self.edit.is_some() && self.prompt.is_none(),
        preedit: self.ime.preedit(),
        preedit_caret: self.ime.caret(),
        picker_on: self.picker.is_some() || self.btn_hover,
        upload_on: self.uploading || (self.prompt.is_some() && !self.finding) || self.up_hover,
        logo: &self.res.logo,
        label: if hidden { &[] } else { &label },
        showcmd: &showcmd,
        chips: &chips,
      };
      let w = self.body().w;
      let h = if hidden { 0 } else { input_bar::height(&mut self.res.text, &bar, w) };
      if h != self.input_h {
        // the chat area changed size: repaint it whole
        self.input_h = h;
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.invalidate();
        }
      }
      if h > 0 {
        let r = self.input_area();
        let (caret, next, hit) = input_bar::draw(&mut self.canvas, &mut self.res.text, &mut self.res.images, r, &bar);
        self.ime_area = Some(caret);
        self.input_hit = (self.prompt.is_none() && !self.line.text.is_empty()).then_some(hit);
        self.input_next = next.map(|ms| Instant::now() + Duration::from_millis(ms.max(ANIM_MIN_MS) as u64));
        dirty = Some(r);
      }
    }
    if let Some(r) = self.sync_pin() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    let area = self.chat_area();
    if (self.menu.is_some()
      || self.tab_menu.is_some()
      || self.pop.is_some()
      || self.card.is_some()
      || self.tip.is_some()
      || self.media_peek.is_some()
      || self.over_input)
      && let Some(t) = self.tabs.get_mut(self.active)
    {
      // the menu floats over the chat: repaint under it so blits and scrolls leave no ghosts
      t.view.invalidate();
    }
    if std::mem::take(&mut self.bar_dirty) {
      let shown: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.visible(i)).collect();
      let (mode, now) = (self.set.tab_counter, now_ms() / 1000);
      let counts: Vec<String> = shown
        .iter()
        .map(|&i| {
          let active = i == self.active;
          let t = &mut self.tabs[i];
          if t.kind == Kind::Chat { t.count.text(mode, active, now) } else { String::new() }
        })
        .collect();
      self.heat_due = (mode == Counter::Heat && counts.iter().any(|c| !c.is_empty()))
        .then(|| Instant::now() + Duration::from_secs(1));
      let labels: Vec<TabLabel> = shown
        .iter()
        .zip(&counts)
        .map(|(&i, count)| {
          let t = &self.tabs[i];
          let hot = match t.kind {
            Kind::Mentions => crate::ui::theme::RED,
            Kind::Whispers => crate::ui::theme::CYAN,
            _ if t.event => crate::ui::theme::YELLOW,
            _ => crate::ui::theme::WHITE,
          };
          // a tab something took over reads `← back` while you look at it (ext takeover.js)
          let back = (t.under.is_some() || self.img_view.is_some()) && i == self.active;
          let name = if back { "← back" } else { &t.name };
          let count = if back { "" } else { count.as_str() };
          let cursor = self.rover.as_ref().is_some_and(|r| r.at == i);
          TabLabel { name, count, active: i == self.active, unread: !back && (t.unread || t.event), hot, cursor }
        })
        .collect();
      let bar = self.bar();
      let up = self.update_label();
      self.bar_hits = tab_bar::draw(
        &mut self.canvas,
        &mut self.res.text,
        bar,
        &labels,
        if hidden_now { (&label, &showcmd) } else { (&[], "") },
        tab_bar::Hot {
          plus: self.plus_hover,
          gear: self.gear_hover || self.screen.is_some(),
          update: up.as_deref().map(|l| (l, self.update_hover)),
        },
      );
      self.bar_map = shown;
      dirty = Some(dirty.map_or(bar, |d| union(d, bar)));
    }
    if self.res.images.next_tick() {
      self.trim_soon();
    }
    if self.form.is_some() {
      self.next_anim = None;
      if let Some(r) = self.draw_form() {
        dirty = Some(dirty.map_or(r, |d| union(d, r)));
      }
    } else if self.screen.is_some() {
      self.next_anim = None;
      if let Some(r) = self.draw_screen() {
        dirty = Some(dirty.map_or(r, |d| union(d, r)));
      }
    } else if self.img_view.is_some() {
      if let Some(r) = self.draw_img_view() {
        dirty = Some(dirty.map_or(r, |d| union(d, r)));
      }
    } else if self.picker.is_some() {
      // the picker covers the chat: paint only it
      self.next_anim = None;
      if let Some(r) = self.draw_picker() {
        dirty = Some(dirty.map_or(r, |d| union(d, r)));
      }
    } else {
      let (canvas, res) = (&mut self.canvas, &mut self.res);
      let add = |d: Option<Rect>, r: Rect| Some(d.map_or(r, |d| union(d, r)));
      if let Some(t) = self.tabs.get_mut(self.active) {
        match t.view.take_paint(res, area.h) {
          Paint::Nothing => {}
          Paint::Full => {
            t.view.draw(canvas, res, area, area);
            dirty = add(dirty, area);
          }
          Paint::Append { shift } => {
            canvas.scroll_up(area, shift);
            t.view.scrolled(area, shift);
            t.view.draw(canvas, res, area, Rect::new(area.x, area.bottom() - shift, area.w, shift));
            dirty = add(dirty, area);
          }
        }
        let due = self.next_anim.is_some_and(|t| t <= Instant::now());
        if due && let Some(r) = t.view.animate(canvas, res, area) {
          dirty = add(dirty, r);
        }
        self.next_anim = t.view.next_anim.map(|ms| Instant::now() + Duration::from_millis(ms.max(ANIM_MIN_MS) as u64));
      } else if dirty.is_some() {
        canvas.fill(area, crate::ui::theme::BLACK);
        dirty = add(dirty, area);
      }
    }
    if let Some(r) = self.draw_over_input() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_card() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_menu() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_tab_menu() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_pop() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_peek() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_tag_peek() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_media_peek() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(r) = self.draw_tip() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    for t in [self.tip_next, self.input_next].into_iter().flatten() {
      self.next_anim = Some(self.next_anim.map_or(t, |n| n.min(t)));
    }
    for (id, url, h) in self.res.images.take_wanted() {
      self.net.send(Cmd::Image(id, url, h));
    }
    for url in self.res.previews.take_wanted() {
      self.net.send(Cmd::Preview(url));
    }
    for id in self.res.senders.take_wanted() {
      self.net.send(Cmd::Sender(id, None));
    }
    self.ask_people();
    for id in self.res.cosmetics.take_wanted() {
      self.net.send(Cmd::Cosmetics(id));
    }
    self.present(dirty);
  }

  fn present(&mut self, dirty: Option<Rect>) {
    let whole = self.canvas.bounds();
    let dirty = if std::mem::take(&mut self.expose) { Some(whole) } else { dirty };
    let (Some(g), Some(d)) = (&mut self.gfx, dirty) else {
      return;
    };
    let Ok(mut buf) = g.surface.buffer_mut() else { return };
    if buf.len() != self.canvas.px.len() {
      return;
    }
    // a reused buffer is `age` presents old: copy this frame's damage plus what it missed since
    let copy = match buf.age() {
      1 => Some(d),
      2 => self.damage_hist[0].map(|h| union(h, d)),
      _ => None,
    };
    self.damage_hist = [Some(d), self.damage_hist[0]];
    match copy {
      Some(r) => {
        let r = r.intersect(self.canvas.bounds());
        let w = self.canvas.w as usize;
        for y in r.y..r.bottom() {
          let a = y as usize * w + r.x as usize;
          buf[a..a + r.w as usize].copy_from_slice(&self.canvas.px[a..a + r.w as usize]);
        }
      }
      None => buf.copy_from_slice(&self.canvas.px),
    }
    // a fresh buffer carries the whole frame: say so, or the compositor keeps only the damage of it
    let d = if copy.is_none() { whole } else { d };
    let r = softbuffer::Rect {
      x: d.x as u32,
      y: d.y as u32,
      width: NonZeroU32::new(d.w as u32).unwrap_or(NonZeroU32::MIN),
      height: NonZeroU32::new(d.h as u32).unwrap_or(NonZeroU32::MIN),
    };
    if let Err(e) = buf.present_with_damage(&[r]) {
      eprintln!("present: {e}");
    }
  }

  fn resize(&mut self, w: u32, h: u32) {
    let (Some(g), Some(nw), Some(nh)) = (&mut self.gfx, NonZeroU32::new(w), NonZeroU32::new(h)) else {
      return;
    };
    if let Err(e) = g.surface.resize(nw, nh) {
      eprintln!("resize: {e}");
      return;
    }
    self.resized(w as i32, h as i32);
  }

  // the canvas at its new size, and everything on it due to paint again
  fn resized(&mut self, w: i32, h: i32) {
    self.canvas.resize(w, h);
    self.damage_hist = [None; 2];
    // softbuffer keeps a resized buffer's age though its pixels are new (zeroed): send it all
    self.expose = true;
    self.input_dirty = true;
    // the canvas came back zeroed: everything paints again, not only what changed width (a height-only
    // resize, e.g. a new window tiling beside this one, left the chat black but for its animations)
    let bw = self.body().w;
    for v in self.tabs.iter_mut().flat_map(Tab::views) {
      v.resize(bw);
      v.invalidate();
    }
    self.pin_sig = Default::default();
    self.bar_dirty = true;
    self.wake();
  }

  fn key_insert(&mut self, key: Key<&str>, text: Option<&str>) {
    let ctrl = self.mods.control_key();
    if self.global_key(key.clone()) {
      return;
    }
    self.sync_suggest();
    if self.suggest_key(&key) {
      return;
    }
    if key == Key::Named(NamedKey::Tab) {
      return self.tab_complete(self.mods.shift_key());
    }
    // any other key commits the current candidate; an emote pick joins the recents. esc only ends
    // the cycle (and a reply), keeping the pick
    let cycling = self.complete.is_some();
    if let Some(name) = self.complete.take().and_then(|c| c.emote().map(str::to_owned)) {
      self.used_emote(&name);
    }
    if cycling && key == Key::Named(NamedKey::Escape) {
      self.reply = None;
      self.input_dirty = true;
      return self.wake();
    }
    let recall = matches!(key, Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown));
    let space = key == Key::Named(NamedKey::Space) || text == Some(" ");
    // an edit leaves a ctrl-z step (a run of typed letters is one); anything else ends the run
    let alt = self.mods.alt_key();
    match &key {
      Key::Named(NamedKey::Backspace | NamedKey::Delete) => self.line.checkpoint(false),
      Key::Character("w" | "u" | "k") if ctrl => self.line.checkpoint(false),
      Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => self.line.checkpoint(!space),
      _ => self.line.seal(),
    }
    if self.set.vi && key == Key::Named(NamedKey::Escape) {
      self.sync_overlays();
    }
    let l = &mut self.line;
    match key {
      Key::Named(NamedKey::Escape) if self.set.vi => {
        let empty = l.text.is_empty();
        if empty {
          self.reply = None;
          self.edit_end();
        }
        return self.leave_insert();
      }
      Key::Named(NamedKey::Escape) => {
        self.reply = None;
        self.edit_end();
        self.find_clear();
      }
      Key::Named(NamedKey::Enter) => self.send(),
      Key::Named(NamedKey::Backspace) if ctrl => l.delete_word_back(),
      Key::Named(NamedKey::Backspace) => l.backspace(),
      Key::Named(NamedKey::Delete) => l.delete(),
      Key::Named(NamedKey::ArrowLeft) if ctrl => l.word_left(),
      Key::Named(NamedKey::ArrowRight) if ctrl => l.word_right(),
      Key::Named(NamedKey::ArrowLeft) => l.left(),
      Key::Named(NamedKey::ArrowRight) => l.right(),
      Key::Named(NamedKey::Home) => l.home(),
      Key::Named(NamedKey::End) => l.end(),
      Key::Named(NamedKey::ArrowUp) => l.older(),
      Key::Named(NamedKey::ArrowDown) => l.newer(),
      Key::Character("a") if ctrl => l.home(),
      Key::Character("e") if ctrl => l.end(),
      Key::Character("w") if ctrl => l.delete_word_back(),
      Key::Character("u") if ctrl => l.clear_to_start(),
      Key::Character("k") if ctrl => l.clear_to_end(),
      Key::Character("z") if ctrl => l.undo(),
      Key::Character("Z" | "y") if ctrl => l.redo(),
      Key::Character("n") if ctrl => {
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.target = (t.target + 1) % t.sources.len().max(1);
        }
      }
      Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
        // mid-composition the input method owns the text; its commit arrives as an ime event
        if let Some(t) = text.filter(|_| self.ime.accepts_key_text()) {
          l.insert(t);
        }
      }
      _ => return,
    }
    if recall {
      self.chip_recalled();
    } else if space {
      self.chip_spaced();
    }
    self.input_dirty = true;
    self.wake();
  }

  // keys that work in every mode: login, tab switching, page scroll
  fn global_key(&mut self, key: Key<&str>) -> bool {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    let half = self.chat_area().h / 2;
    match key {
      Key::Character("o") if ctrl => self.pick_file(),
      Key::Character("f") if ctrl => self.find_open(),
      Key::Character("g") if ctrl && self.find_on() => self.find_step(1),
      Key::Character("G") if ctrl && self.find_on() => self.find_step(-1),
      Key::Character("t") if ctrl => self.open_form(None),
      Key::Character("l") if ctrl => self.login(),
      Key::Character("L") if ctrl => self.net.send(Cmd::Logout),
      Key::Named(NamedKey::Tab) if ctrl => self.select(self.step_tab(self.active, self.mods.shift_key())),
      Key::Character(d) if alt && d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
        let n = (d.as_bytes()[0] - b'0') as usize;
        if let Some(i) = self.nth_visible(if n == 0 { 9 } else { n - 1 }) {
          self.select(i);
        }
      }
      Key::Named(NamedKey::PageDown) => self.scroll(-half * 2),
      Key::Named(NamedKey::PageUp) => self.scroll(half * 2),
      _ => return false,
    }
    true
  }

  // vi normal mode: a picked line's keys first, then the space leader's chat commands, then vim on the box
  // a key went down: the open layer first (form, screen, picture, prompt, picker, menus, the chat logs),
  // then the box's mode
  fn on_key(&mut self, k: Key<&str>, t: Option<&str>) {
    if self.zoom_key(&k) {
      return;
    }
    if self.form.is_some() {
      if self.paste_key(&k) {
        return;
      }
      return self.form_key(k, t);
    }
    if self.mods.control_key() && k == Key::Character(",") {
      return self.toggle_screen(screen::Page::Settings);
    }
    if k == Key::Named(NamedKey::F1) {
      return self.toggle_screen(screen::Page::Keys);
    }
    if self.screen.is_some() {
      return self.screen_key(k, t);
    }
    if self.img_view.is_some() {
      return self.img_key(k);
    }
    if self.mods.control_key() && k == Key::Character(";") {
      return self.toggle_picker();
    }
    if self.paste_key(&k) {
      return;
    }
    if self.prompt.is_some() {
      return self.prompt_key(k, t);
    }
    if self.picker.is_some() {
      return self.picker_key(k, t);
    }
    if self.pop.is_some() {
      return self.pop_key(k);
    }
    if self.card.is_some() {
      return self.card_key(k);
    }
    if self.tab_menu.is_some() {
      return self.tab_menu_key(k);
    }
    if self.rover.is_some() {
      return self.rover_key(k);
    }
    if self.menu.is_some() {
      return self.menu_key(k);
    }
    if self.logs_key(k.clone()) {
      return;
    }
    match self.mode {
      Mode::Insert => self.key_insert(k, t),
      Mode::Normal => self.key_normal(k),
    }
  }

  fn key_normal(&mut self, key: Key<&str>) {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    if self.global_key(key.clone()) {
      return;
    }
    if self.chord.take() == Some(' ') {
      self.leader_key(key);
      return self.wake();
    }
    if self.selecting() && self.pick_key(key.clone()) {
      return;
    }
    let idle = self.vi.idle();
    let half = self.chat_area().h / 2;
    match key {
      Key::Named(NamedKey::Space) if idle && !ctrl && !alt => {
        self.chord = Some(' ');
        return self.wake();
      }
      Key::Character("t" | "T") if self.vi.g_pending() => {
        self.vi.abort();
        return self.select(self.step_tab(self.active, key == Key::Character("T")));
      }
      Key::Character("v") if idle && !ctrl => return self.select_start(),
      Key::Character("?") if idle => return self.open_screen(screen::Page::Keys),
      // vim's own / n N search the box, which is one line: here they search the chat
      Key::Character("/") if idle && !ctrl => return self.find_open(),
      Key::Character("n") if idle && !ctrl && self.find_on() => return self.find_step(1),
      Key::Character("N") if idle && !ctrl && self.find_on() => return self.find_step(-1),
      Key::Character("d") if ctrl => return self.scroll(-half),
      Key::Character("u") if ctrl => return self.scroll(half),
      Key::Character("w") if ctrl => return self.close_tab(),
      Key::Named(NamedKey::Escape) if idle && self.feed_back() => return,
      _ => {}
    }
    let name = match key {
      Key::Character("r") if ctrl => "<c-r>",
      _ if ctrl || alt => return,
      Key::Character(c) => c,
      Key::Named(NamedKey::Space) => " ",
      Key::Named(NamedKey::Escape) => "Esc",
      Key::Named(NamedKey::Enter) => "Enter",
      Key::Named(NamedKey::Backspace) => "Backspace",
      Key::Named(NamedKey::Delete) => "Delete",
      Key::Named(NamedKey::ArrowUp) => "Up",
      Key::Named(NamedKey::ArrowDown) => "Down",
      Key::Named(NamedKey::ArrowLeft) => "Left",
      Key::Named(NamedKey::ArrowRight) => "Right",
      Key::Named(NamedKey::Home) => "Home",
      Key::Named(NamedKey::End) => "End",
      _ => return,
    };
    self.sync_overlays();
    let act = self.vi.key(&mut self.line, name);
    if let Some(y) = self.vi.take_fresh() {
      self.copy(&y);
    }
    match act {
      Action::None => {}
      Action::Insert => self.mode = Mode::Insert,
      Action::Send => {
        self.mode = Mode::Insert;
        self.send();
      }
      Action::HistoryOlder => {
        self.line.older();
        Vi::settle(&mut self.line);
        self.chip_recalled();
      }
      Action::HistoryNewer => {
        self.line.newer();
        Vi::settle(&mut self.line);
        self.chip_recalled();
      }
      Action::Leave => {
        if !self.find_clear() {
          self.reply = None;
          self.edit_end();
        }
      }
    }
    self.complete = None;
    self.input_dirty = true;
    self.wake();
  }

  // `r` on the selected line: reply to it on its own platform
  fn reply_selected(&mut self) {
    let Some(t) = self.tabs.get_mut(self.active).filter(|t| t.kind == Kind::Chat) else { return };
    let Some(m) = t.view.selected() else { return };
    if m.id.is_empty() {
      return self.say("that line can't be replied to");
    }
    let reply = Reply { id: m.id.clone(), nick: m.nick.clone(), text: m.body.clone() };
    let p = m.platform;
    if let Some(i) = t.sources.iter().position(|s| s.0 == p) {
      t.target = i;
    }
    self.reply = Some(reply);
    self.enter_insert();
  }

  fn select_start(&mut self) {
    let h = self.chat_area().h;
    let res = &mut self.res;
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.select_start(res, h);
    }
    self.wake();
  }

  fn select_move(&mut self, by: i32) {
    let h = self.chat_area().h;
    let res = &mut self.res;
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.select_move(by, res, h);
    }
    self.wake();
  }

  // click a line to select it; click the input to type
  fn click(&mut self) {
    self.hide_tip();
    if self.form.is_some() {
      return self.form_click();
    }
    if self.screen.is_some() {
      // the tab bar stays live: a tab or the gear closes the screen first
      if !self.bar().contains(self.pointer.0, self.pointer.1) {
        return self.screen_click();
      }
      if self.gear_at() {
        return self.close_screen();
      }
      self.close_screen();
    }
    if self.pop.is_some() {
      return self.pop_click();
    }
    if self.card.is_some() {
      return self.card_click();
    }
    if self.tab_menu.is_some() {
      return self.tab_menu_click();
    }
    if self.menu.is_some() {
      return self.menu_click();
    }
    let (x, y) = self.pointer;
    if self.bar().contains(x, y) {
      return self.bar_click();
    }
    if self.img_view.is_some() {
      // the picture is its own way out
      return self.img_close();
    }
    if self.suggest_click() {
      return;
    }
    if self.pin_click() {
      return;
    }
    let (chat, input) = (self.chat_area(), self.input_area());
    if self.over_button() {
      return self.toggle_picker();
    }
    if self.over_upload() {
      return self.pick_file();
    }
    if self.picker.is_some() {
      return self.picker_click();
    }
    if input.contains(x, y) {
      // the cursor goes where you click, like any text box
      if let Some(i) = self.input_hit.as_ref().map(|h| h.byte_at(x, y)) {
        self.complete = None;
        self.line.seal();
        self.line.cursor = i.min(self.line.text.len());
        self.input_dirty = true;
      }
      return self.enter_insert();
    }
    if !chat.contains(x, y) {
      return;
    }
    if self.vote_press()
      || self.held_press()
      || self.img_press()
      || self.reveal_press()
      || self.thread_press()
      || self.link_press()
      || self.tag_press()
      || self.emote_press(false)
      || self.name_press()
    {
      return;
    }
    let res = &mut self.res;
    let picked = self.tabs.get_mut(self.active).is_some_and(|t| t.view.select_at(chat, y, res));
    if picked && self.tabs[self.active].kind == Kind::Mentions {
      self.jump_to_mention();
    } else if picked && self.set.vi && self.mode == Mode::Insert {
      // a picked line's keys (r, m, c) are normal mode's
      self.leave_insert();
    }
    self.wake();
  }

  fn over_button(&self) -> bool {
    let (x, y) = self.pointer;
    self.input_h > 0 && input_bar::button(self.input_area(), self.res.text.line_px as i32).contains(x, y)
  }

  fn right_click(&mut self) {
    if self.picker.is_some() || self.form.is_some() {
      return;
    }
    if self.screen.is_some() {
      return self.close_screen();
    }
    if self.pop.is_some() {
      self.pop = None;
      return self.pop_changed();
    }
    if self.card.is_some() {
      self.card = None;
      return self.pop_changed();
    }
    if self.tab_menu.is_some() {
      self.tab_menu = None;
      return self.tab_menu_changed();
    }
    if self.bar().contains(self.pointer.0, self.pointer.1) {
      return self.bar_right_click();
    }
    if self.menu.is_some() {
      self.menu = None;
      return self.menu_changed();
    }
    if !self.emote_press(true) {
      self.line_right_click();
    }
  }

  fn selecting(&self) -> bool {
    self.tabs.get(self.active).is_some_and(|t| t.view.selected().is_some())
  }

  // a picked line's keys: j/k move it, r replies, esc lets go. false = not one of them (vi gets it)
  fn pick_key(&mut self, key: Key<&str>) -> bool {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) if !ctrl => self.select_move(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) if !ctrl => self.select_move(-1),
      Key::Named(NamedKey::Enter) if self.kind() == Kind::Mentions => self.jump_to_mention(),
      Key::Named(NamedKey::Enter) if self.feedish() => self.feed_activate(),
      Key::Character("u") if !ctrl && self.feedish() => self.feed_vote(),
      Key::Character("r") if !ctrl && self.feedish() => self.feed_reply(),
      Key::Character("h") | Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Backspace)
        if !ctrl && self.feed_back() => {}
      Key::Named(NamedKey::Enter) if self.kind() == Kind::Whispers => self.whisper_activate(),
      Key::Character("r") if !ctrl && self.kind() == Kind::Whispers => self.whisper_reply_selected(),
      Key::Character("r") if !ctrl => self.reply_selected(),
      Key::Character("m") if !ctrl => self.line_menu_key(),
      Key::Character("c") if !ctrl => self.card_selected(),
      Key::Character("o") if !ctrl && !alt => self.open_selected_link(),
      Key::Character("c") if ctrl => self.copy_selected(),
      Key::Named(NamedKey::Escape) | Key::Character("v") if !ctrl => {
        if let Some(v) = self.view() {
          v.deselect();
        }
        self.wake();
      }
      _ => return false,
    }
    true
  }

  // space then a key (the ext's leader): chat commands that vim's keys would otherwise take
  fn leader_key(&mut self, key: Key<&str>) {
    let Key::Character(c) = key else { return };
    match c {
      "/" => self.find_open(),
      "n" if self.find_on() => self.find_step(1),
      "N" if self.find_on() => self.find_step(-1),
      "]" => self.select(self.step_tab(self.active, false)),
      "[" => self.select(self.step_tab(self.active, true)),
      "x" => self.pin_hide(),
      "t" => self.rover_open(),
      "w" => self.watch_here(),
      "g" => {
        let res = &mut self.res;
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.scroll_to_top(res);
        }
      }
      "G" => {
        if let Some(v) = self.view() {
          v.scroll_to_bottom();
        }
      }
      d if d.len() == 1 && (b'1'..=b'9').contains(&d.as_bytes()[0]) => {
        if let Some(i) = self.nth_visible((d.as_bytes()[0] - b'1') as usize) {
          self.select(i);
        }
      }
      _ => {}
    }
  }
}

impl ApplicationHandler<NetEvent> for App {
  fn resumed(&mut self, el: &ActiveEventLoop) {
    if self.gfx.is_some() {
      return;
    }
    // x11 shows it; wayland compositors take their icon from the .desktop file instead
    let icon = crate::ui::logo::window_rgba().and_then(|(px, w)| winit::window::Icon::from_rgba(px, w, w).ok());
    let attrs = Window::default_attributes()
      .with_title(mentions::TITLE)
      .with_window_icon(icon)
      .with_inner_size(winit::dpi::LogicalSize::new(420, 800));
    // the app id (wayland) and WM_CLASS (x11): what the .desktop file's icon is matched on
    let attrs = WindowAttributesExtWayland::with_name(attrs, APP_ID, APP_ID);
    let attrs = WindowAttributesExtX11::with_name(attrs, APP_ID, APP_ID);
    let window = match el.create_window(attrs) {
      Ok(w) => Rc::new(w),
      Err(e) => {
        eprintln!("window: {e}");
        return el.exit();
      }
    };
    let surface = Context::new(window.clone()).and_then(|ctx| Surface::new(&ctx, window.clone()));
    let surface = match surface {
      Ok(s) => s,
      Err(e) => {
        eprintln!("surface: {e}");
        return el.exit();
      }
    };
    self.dpi = window.scale_factor() as f32;
    self.res.set_scale(self.scale());
    let size = window.inner_size();
    self.gfx = Some(Gfx { window, surface });
    self.resize(size.width, size.height);
  }

  fn user_event(&mut self, _: &ActiveEventLoop, ev: NetEvent) {
    self.on_net(ev);
  }

  fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
    match ev {
      WindowEvent::CloseRequested => el.exit(),
      WindowEvent::Resized(s) => self.resize(s.width, s.height),
      WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
        self.dpi = scale_factor as f32;
        self.apply_scale();
      }
      WindowEvent::RedrawRequested => {
        let t = Instant::now();
        self.redraw();
        if self.prof {
          eprintln!("frame {:.3}ms img {}KB", t.elapsed().as_secs_f64() * 1e3, self.res.images.bytes() >> 10);
        }
      }
      WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
      WindowEvent::Focused(f) => {
        self.focused = f;
        if f {
          self.flash_stop();
        } else {
          self.hide_tip();
        }
      }
      WindowEvent::Occluded(o) => {
        self.occluded = o;
        if !o {
          self.expose = true;
          self.wake();
        }
      }
      WindowEvent::CursorMoved { position, .. } => {
        self.pointer = (position.x as i32, position.y as i32);
        self.menu_hover();
        self.tab_menu_hover();
        self.pop_hover();
        self.card_hover();
        self.screen_hover();
        self.form_hover_move();
        self.bar_hover();
        self.picker_hover();
        self.cursor_sync();
        self.tip_hover();
        let (over, up) = (self.over_button(), self.over_upload());
        if over != self.btn_hover || up != self.up_hover {
          self.btn_hover = over;
          self.up_hover = up;
          self.input_dirty = true;
          self.wake();
        }
      }
      WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.click(),
      WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => self.right_click(),
      WindowEvent::Ime(ev) => self.on_ime(ev),
      WindowEvent::DroppedFile(p) => self.upload_file(p),
      WindowEvent::CursorLeft { .. } => self.hide_tip(),
      WindowEvent::MouseWheel { delta, .. } => {
        self.hide_tip();
        let px = match delta {
          MouseScrollDelta::LineDelta(_, y) => (y * self.res.text.line_px * 3.0) as i32,
          MouseScrollDelta::PixelDelta(p) => p.y as i32,
        };
        let lines = match delta {
          MouseScrollDelta::LineDelta(_, y) => Some(y),
          MouseScrollDelta::PixelDelta(_) => None,
        };
        if self.zoom_wheel(lines, px as f32) || self.form.is_some() {
          return;
        }
        if self.screen.is_some() {
          self.screen_wheel(px);
        } else if self.img_view.is_some() {
          self.img_wheel(px);
        } else if self.picker.is_some() {
          self.picker_wheel(px);
        } else {
          self.scroll(px);
        }
      }
      WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
        self.hide_tip();
        let (k, t) = (event.logical_key.as_ref(), event.text.as_deref());
        self.on_key(k, t);
      }
      _ => {}
    }
  }

  fn about_to_wait(&mut self, el: &ActiveEventLoop) {
    self.sync_ime();
    let now = Instant::now();
    if let Some(next) = self.flood
      && now >= next
    {
      let m = self.fake.message(now_ms());
      self.on_chat(FLOOD_TAB, Chat::Msg(m));
      self.flood = Some(now + FLOOD_EVERY);
    }
    if self.trim_at.is_some_and(|t| t <= now) {
      self.trim_at = None;
      crate::util::mem::trim();
    }
    // animations sleep while the window is hidden
    let anim = self.next_anim.filter(|_| !self.occluded);
    if anim.is_some_and(|t| t <= now) {
      self.wake();
    }
    if self.mute_due.is_some_and(|t| t <= now) {
      self.mute_tick();
    }
    if self.tag_due().is_some_and(|t| t <= now) {
      self.tag_fire();
    }
    if self.peek_due.is_some_and(|t| t <= now) {
      self.peek_fire();
    }
    if self.gifs.due().is_some_and(|t| t <= now) {
      self.gifs_tick(now);
    }
    if self.update_due.is_some_and(|t| t <= now) {
      self.update_check();
    }
    if self.pace_due.is_some_and(|t| t <= now) {
      self.pace_tick();
    }
    if self.flash_due.is_some_and(|t| t <= now) {
      self.flash_tick();
    }
    if self.held.due().is_some_and(|t| t <= now) {
      self.held_tick();
    }
    self.ui_state_tick();
    if self.heat_due.is_some_and(|t| t <= now) {
      self.heat_due = None;
      self.bar_dirty = true;
      self.wake();
    }
    let wake = [
      self.flood,
      anim,
      self.trim_at,
      self.mute_due,
      self.peek_due,
      self.tag_due(),
      self.gifs.due(),
      self.update_due,
      self.heat_due,
      self.pace_due,
      self.flash_due,
      self.held.due(),
      self.ui_state_due(),
    ]
    .into_iter()
    .flatten()
    .min();
    el.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
  }
}

impl App {
  // every net event lands here; the window handler and the headless live probe share it
  fn on_net(&mut self, ev: NetEvent) {
    match ev {
      NetEvent::Chat(ch, c) => self.on_chat(&ch, c),
      NetEvent::Backfill(ch, events) => {
        let Some(i) = self.tabs.iter().position(|t| t.name == ch) else {
          return;
        };
        let mut history: Vec<Message> = events
          .into_iter()
          .filter_map(|e| match e {
            Chat::Msg(m) => Some(m),
            _ => None,
          })
          .collect();
        // every line first, so an older line without an id (heatsync's archived part) can take
        // the one a newer line from the same person carries
        for m in &history {
          self.res.chatters.heard(m);
        }
        for m in &mut history {
          m.flags |= flag::HISTORY;
          if m.user_id.is_empty()
            && let Some(id) = self.res.chatters.twitch_id(&m.login)
          {
            m.user_id = id.to_string().into();
          }
          self.scan_mention(Platform::Twitch, &ch, m);
        }
        self.tabs[i].chat().prepend(history);
        self.trim_soon();
        if i == self.active {
          self.wake();
        }
      }
      NetEvent::ModDone(d) => self.on_mod_done(*d),
      NetEvent::Card { platform, login, res } => self.on_card(platform, &login, res),
      NetEvent::Pronouns(uid, res) => self.on_pronouns(uid, res),
      NetEvent::Held(h) => self.on_held(*h),
      NetEvent::HeldEnd { chan, id, end } => self.on_held_end(chan, id, end),
      NetEvent::HeldWatched { chan, res } => self.on_held_watched(chan, res),
      NetEvent::HeldActed { chan, id, allow, res } => self.on_held_acted(chan, id, allow, res),
      NetEvent::Blocked { room, res } => self.on_blocked(room, res),
      NetEvent::ModesDone { chan, set, res } => self.on_modes_done(chan, set, res),
      NetEvent::StreamInfo { chan, info } => self.on_stream_info(chan, info),
      NetEvent::Suite { chan, cmd, res } => self.on_suite(chan, cmd, res),
      NetEvent::Tools { chan, res } => self.on_tools(chan, res),
      NetEvent::TagCard(tag, res) => self.on_tag_card(tag, res),
      NetEvent::NoteSaved { platform, login, text, ok } => self.note_saved(platform, &login, &text, ok),
      // an answer that outlived its login (logout raced the fetch) must not seed the next state
      NetEvent::Lists(_) | NetEvent::BlockedHashes(_) if self.me.is_none() => {}
      NetEvent::Lists(l) => self.on_lists(l),
      NetEvent::MuteSync(s) => self.on_mute_sync(s),
      NetEvent::Whispers { items, live, epoch } => {
        if whisper::current(epoch, self.whisper_epoch) {
          self.on_whispers(items, live)
        }
      }
      NetEvent::WhisperSent { key, res } => self.on_whisper_sent(key, res),
      NetEvent::Feed(p) => self.on_feed(p),
      NetEvent::FeedFailed { status } => self.on_feed_failed(status),
      NetEvent::Thread { id, res } => self.on_thread(id, res),
      NetEvent::Update(r) => self.on_update(r),
      NetEvent::Updated(res) => self.on_updated(res),
      NetEvent::Media { id, res } => {
        self.feed.media_loaded(&id, res);
        self.feed_show();
      }
      NetEvent::Seen { surface, at } => self.on_seen(surface, at),
      // an answer that outlived its login must not seed the next one
      NetEvent::UiState(_) if self.me.is_none() => {}
      NetEvent::UiState(st) => self.on_ui_state(st),
      NetEvent::HsAuth(on) => {
        self.hs_authed = on;
        if on {
          self.ui_state_fetch();
          self.held_reconnected();
        }
      }
      NetEvent::Voted { id, res } => self.on_voted(id, res),
      NetEvent::Posted { res } => self.on_posted(res),
      NetEvent::Bookmarked { id, on, res } => self.on_bookmarked(id, on, res),
      NetEvent::Marks { ids, res } => self.on_marks(ids, res),
      NetEvent::Edited { id, res } => self.on_edited(id, res),
      NetEvent::Deleted { id, res } => self.on_deleted(id, res),
      NetEvent::Logs { epoch, res } => self.on_logs(epoch, res),
      NetEvent::Gifs { epoch, q, res } => self.on_gifs(epoch, q, res),
      NetEvent::FeedLive(p) => self.on_feed_live(p),
      NetEvent::Pin { platform, key, pin } => self.on_pin(platform, &key, pin),
      NetEvent::Stream(l) => self.on_stream(l),
      NetEvent::FollowColors(c) => self.on_follow_colors(c),
      NetEvent::Moment { platform, key, text } => self.on_moment(platform, &key, &text),
      NetEvent::Emotes(ch, p, set) => {
        self.res.emotes.set(ch.as_deref(), p, set);
        self.trim_soon();
        self.relayout_where(ch.as_deref());
      }
      NetEvent::Preview(url, p) => {
        if self.res.previews.answered(&url, p) {
          for t in &mut self.tabs {
            t.views().for_each(ChatView::relayout);
          }
          self.wake();
        }
      }
      NetEvent::Image(id, h, res) => {
        // a chip's picture may be the one that arrived
        self.input_dirty |= !self.line.chips.is_empty();
        if self.res.images.loaded(id, h, res) {
          if let Some(t) = self.tabs.get_mut(self.active) {
            t.view.relayout();
          }
        } else if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.invalidate();
        }
        self.wake();
      }
      NetEvent::TwitchBadges(ch, set) => {
        self.res.badges.set_twitch(ch.as_deref(), set);
        self.relayout_where(ch.as_deref());
      }
      NetEvent::FfzRoomBadges(ch, set) => {
        let had = !set.is_empty();
        self.res.badges.set_ffz_room(&ch, set);
        if had {
          self.relayout_where(Some(&ch));
        }
      }
      NetEvent::UserBadges(src, set) => {
        self.res.badges.set_users(src, set);
        self.relayout_where(None);
        self.trim_soon();
      }
      NetEvent::Senders(sets) => {
        if self.prof {
          eprintln!("senders {} owning {}", sets.len(), sets.iter().filter(|(_, s)| !s.is_empty()).count());
        }
        for (id, set) in sets {
          if self.res.senders.put(id, set) {
            self.touch_sender(id);
          }
        }
        self.trim_soon();
      }
      NetEvent::Me(me) => {
        // our own inventory feeds tab-complete: fetch it now, not on first sight of our own line
        self.net.send(Cmd::SevenTvMe(me.as_ref().and_then(|m| m.twitch_id)));
        if let Some(id) = me.as_ref().and_then(|m| m.twitch_id) {
          self.res.senders.lookup(id, "", now_ms());
          self.wake();
        }
        if let Some(m) = &me {
          self.say(format!("logged in as {}", m.username));
        }
        // a different login starts with a clean whispers tab
        if self.me.as_ref().map(|m| &m.username) != me.as_ref().map(|m| &m.username) {
          self.whispers_reset();
          // the new account's own seen times arrive with SeenGet below
          self.unseen = Default::default();
          // nothing pending belongs to the new account
          self.acct.reset(&self.set);
          self.ui_get_at = None;
          self.filters_login_changed();
          self.feed_reset();
          self.res.blocked.clear_server();
          self.mod_in.clear();
          self.held = Default::default();
          self.relayout_where(None);
        }
        self.me = me;
        // a relink may have brought the automod permission
        self.held.reset();
        self.held_sweep();
        if self.me.is_some() {
          self.net.send(Cmd::Social(crate::net::social::Cmd::SeenGet));
          self.ui_state_fetch();
        }
        self.whisper_empty();
        self.set_mention_names();
        self.filter_names();
        self.entered();
        self.bar_dirty = true;
        self.input_dirty = true;
        self.wake();
      }
      NetEvent::SendersStale { ids, ver, removed } => {
        if self.prof {
          eprintln!("senders stale {}", ids.len());
        }
        for id in ids {
          let Some(scrubbed) = self.res.senders.stale(id, removed.as_deref()) else { continue };
          self.net.send(Cmd::Sender(id, ver.clone()));
          if scrubbed {
            self.touch_sender(id);
          }
        }
      }
      NetEvent::Cosmetics(got) => {
        for (id, c) in got {
          if self.res.cosmetics.put(&id, c) {
            self.touch_paint(&id);
          }
        }
        self.trim_soon();
      }
      NetEvent::CosmeticsChanged(ids) => {
        ids.iter().for_each(|id| self.res.cosmetics.stale(id));
        self.wake();
      }
      NetEvent::CosmeticsStale(id) => {
        self.res.cosmetics.stale(&id);
        self.wake();
      }
      NetEvent::EmoteDelta(ch, d) => {
        if self.res.emotes.patch(&ch, &d) {
          self.relayout_where(Some(&ch));
        }
      }
      NetEvent::BlockedHashes(h) => {
        self.res.blocked.set_server(h);
        self.relayout_where(None);
      }
      NetEvent::Inventory { removed } => {
        if let Some(id) = self.me.as_ref().and_then(|m| m.twitch_id) {
          let scrubbed = self.res.senders.stale(id, removed.as_deref());
          self.net.send(Cmd::Sender(id, Some(now_ms().to_string().into())));
          if scrubbed == Some(true) {
            self.touch_sender(id);
          }
        }
        self.bar_dirty = true;
        self.wake();
      }
      NetEvent::Relay { platform, key, msgs, history } => self.on_relay(platform, &key, msgs, history),
      NetEvent::RelayClear { platform, key, login } => {
        // a removal must find every line: what is still waiting shows first
        let waiting = self.pacer.flush(&key);
        self.relay_show(platform, &key, waiting, false);
        for i in self.tabs_with(platform, &key) {
          self.tabs[i].chat().clear_user(&login);
          self.touched(i);
        }
        self.mod_log(platform, &key, format!("{login} was removed"));
      }
      NetEvent::YtStatus { key, status } => {
        let line = match &*status {
          "not-live" => "youtube: not live right now",
          "ended" => "youtube: stream ended",
          "chat-off" => "youtube: chat is off",
          "no-slot" => "youtube: relay is full, retrying later",
          _ => return,
        };
        for i in self.tabs_with(Platform::Youtube, &key) {
          self.tabs[i].chat().push(Message::system(now_ms(), Platform::Youtube, format!("{line} ({key})")));
          self.touched(i);
        }
      }
      NetEvent::Links { tab, kick, yt } => self.on_links(&tab, kick, yt),
      NetEvent::Upload(u) => self.on_upload(u),
      NetEvent::Status(s) => self.say(s),
    }
  }
}

pub fn persist(channels: &[Box<str>]) {
  store::save_tabs(channels);
}

// the whole app, no window and no event loop: real network in over a channel, real layout + redraw
// into the canvas. run with HOME/XDG_CONFIG_HOME/XDG_CACHE_HOME pointed at an empty dir
#[cfg(test)]
mod live {
  use super::*;
  use crate::cosmetics::id::PaintId;
  use crate::net::Out;

  // ennortix's solid #ffb000 base (the [H] brand orange is #ff8700, so it never counts)
  fn amber(c: &crate::ui::canvas::Canvas) -> usize {
    c.px.iter().filter(|p| (*p >> 16 & 255) > 200 && (165..=190).contains(&(*p >> 8 & 255)) && (*p & 255) < 60).count()
  }

  // twitch line from ennortix -> paint fetched -> stored -> looked up by the renderer -> amber pixels
  #[test]
  #[ignore]
  fn live_app_paints_ennortix() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (tx, rx) = std::sync::mpsc::channel();
    let net = Net::spawn_out(Out::Pipe(tx));
    let mut app = App::new(net, vec!["nl_kripp yt:@kripparrian".into()], false);
    app.dpi = std::env::var("HSC_PROBE_DPI").ok().and_then(|v| v.parse().ok()).unwrap_or(1.5);
    app.res.set_scale(app.scale());
    app.canvas.resize(420, 800);
    for t in &mut app.tabs {
      t.view.resize(420);
    }
    let id = PaintId::Twitch(37005319);
    let deadline = Instant::now() + Duration::from_secs(25);
    let (mut stored, mut line, mut sent) = (false, None::<(Box<str>, Message)>, false);
    let mut px = 0;
    while Instant::now() < deadline && !(sent && px > 30) {
      if let Ok(ev) = rx.recv_timeout(Duration::from_millis(250)) {
        if let NetEvent::Backfill(ch, evs) = &ev {
          let hit = evs.iter().rev().find_map(|e| match e {
            Chat::Msg(m) if PaintId::of_message(m).as_ref() == Some(&id) => Some(m.clone()),
            _ => None,
          });
          line = line.or(hit.map(|m| (ch.clone(), m)));
        }
        app.on_net(ev);
      }
      app.redraw();
      stored = stored || app.res.cosmetics.get(&id).is_some_and(|c| c.paint.is_some());
      // their paint is in: a fresh live line from them lands at the bottom of the tab
      if let (true, Some((ch, m)), false) = (stored, &line, sent) {
        let mut m = m.clone();
        m.id = "live-probe".into();
        m.flags &= !flag::HISTORY;
        app.on_net(NetEvent::Chat(ch.clone(), Chat::Msg(m)));
        sent = true;
        app.tabs[app.active].view.scroll_to_bottom();
      }
      app.redraw();
      px = amber(&app.canvas);
    }
    // heatsync says their paint changed: the refetch must not lose the look
    let frame = serde_json::json!({"type": "cosmetic:changed", "ids": [id.wire()]});
    app.on_net(NetEvent::CosmeticsChanged(crate::net::hs_ws::cosmetic_ids(&frame)));
    let again = Instant::now() + Duration::from_secs(4);
    while Instant::now() < again {
      if let Ok(ev) = rx.recv_timeout(Duration::from_millis(250)) {
        app.on_net(ev);
      }
      app.redraw();
    }
    app.tabs[app.active].view.scroll_to_bottom();
    app.redraw();
    let after = amber(&app.canvas);
    assert!(after > 30, "paint lost after cosmetic:changed (amber px {after})");
    // the old lines too: scroll the whole history past the viewport, a painted name shows somewhere
    let mut seen = 0;
    app.tabs[app.active].view.scroll_to_top(&mut app.res);
    for _ in 0..400 {
      app.redraw();
      seen = seen.max(app.tabs[app.active].view.animated_names());
      app.tabs[app.active].view.scroll_by(-400, &mut app.res);
    }
    assert!(seen > 0, "no painted ennortix line anywhere in the history");
    let n = app.tabs[app.active].view.len();
    eprintln!("rows {n} amber px {px} has paint: {:?}", app.res.cosmetics.get(&id).map(|c| c.paint.is_some()));
    if let Some(path) = std::env::var_os("HSC_DUMP") {
      let c = &app.canvas;
      let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
      c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
      let _ = std::fs::write(path, out);
    }
    assert!(stored, "ennortix paint never stored under {id:?}");
    assert!(sent, "no ennortix line came through the tab");
    let painted = app.tabs[app.active].view.animated_names();
    assert!(
      painted > 0 && px > 30,
      "paint stored but the renderer shows none (painted names {painted}, amber px {px})"
    );
  }

  // hovering a stacked emote opens the whole stack, offline (scratch HOME as below):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d HSC_DUMP=$d/x.ppm cargo test -j2 --release headless_stack -- --ignored
  #[test]
  #[ignore]
  fn headless_stack_hover() {
    use crate::emotes::{Emote, Provider};
    use crate::images::decode::{Decoded, Frame};
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, Vec::new(), true);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let (base, ice) = ("https://cdn.7tv.app/emote/a/1x.webp", "https://cdn.betterttv.net/emote/b/1x.webp");
    let e = |url: &str, zero_width| Emote { url: url.into(), w: 28, h: 28, zero_width };
    app.res.emotes.set(
      None,
      Provider::SevenTv,
      [("Kappa".into(), e(base, false)), ("frozen0".into(), e(ice, true))].into(),
    );
    let mut m = Message::system(now_ms(), Platform::Twitch, "x");
    m.kind = crate::model::message::Kind::Chat;
    m.id = "1".into();
    m.login = "alice".into();
    m.nick = "alice".into();
    m.color = 0x00ff00;
    m.body = "so cold Kappa frozen0 brr".into();
    app.on_net(NetEvent::Chat(FLOOD_TAB.into(), Chat::Msg(m)));
    app.redraw();
    // a ring and a dot: the picture, not a flat square, so alignment shows
    let art = |w: u32, rgb: [u8; 3]| {
      let px: Vec<u8> = (0..w * w)
        .flat_map(|i| {
          let (x, y) = ((i % w) as i32 - w as i32 / 2, (i / w) as i32 - w as i32 / 2);
          let r2 = x * x + y * y;
          let on = r2 < (w as i32 / 2).pow(2) && (w as i32 > 14 || r2 > 4);
          if on { [rgb[0], rgb[1], rgb[2], 255] } else { [0, 0, 0, 0] }
        })
        .collect();
      Decoded { w, h: w, frames: vec![Frame { rgba: px.into(), delay_ms: 0 }] }
    };
    let area = app.chat_area();
    let spot = (area.x..area.right()).step_by(2).find_map(|x| {
      (area.y..area.bottom()).find_map(|y| {
        let res = &mut app.res;
        let h = app.tabs[app.active].view.emote_at(area, x, y, res)?;
        (&*h.name == "Kappa").then_some((x, y))
      })
    });
    app.pointer = spot.expect("the stack answers the pointer");
    app.tip_hover();
    let tip = app.tip.as_ref().expect("hovering the stack opens the popup");
    assert!(tip.is(
      "Kappa",
      base,
      &[crate::ui::chat_view::Layer {
        name: "frozen0".into(),
        url: ice.into(),
        scope: crate::emotes::block::Scope::Global
      }]
    ));
    // the chat's own pictures first, then the popup's big rungs
    // (the wanted height packs the max width above bit 16)
    let mut drawn = None;
    for _ in 0..4 {
      app.redraw();
      while let Ok(c) = asked.try_recv() {
        let Cmd::Image(id, url, h) = c else { continue };
        let n = ((h & 0xffff) / 28).max(1);
        let img = if url.contains("betterttv") { art(14 * n, [0, 255, 255]) } else { art(28 * n, [255, 0, 0]) };
        app.on_net(NetEvent::Image(id, h, Ok(img)));
      }
      drawn = app.draw_tip();
    }
    let r = drawn.expect("the popup paints once every piece is here");
    // both pieces show in the popup: the base's ring and the overlay's dot
    let has = |rgb| (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| app.canvas.px[(y * 720 + x) as usize] == rgb));
    assert!(has(0xff0000) && has(0x00ffff), "base and overlay both in the popup");
    if let Some(path) = std::env::var_os("HSC_DUMP") {
      let c = &app.canvas;
      let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
      c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
      let _ = std::fs::write(path, out);
    }
  }

  // tab-completed emotes draw as pictures in the box, an overlay typed after one stacks on it, a hand-typed
  // channel emote stays a word, and backspace takes a picture whole (scratch HOME as below):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d cargo test -j2 --release headless_input_chips -- --ignored
  #[test]
  #[ignore]
  fn headless_input_chips() {
    use crate::emotes::{Emote, Provider};
    use crate::images::decode::{Decoded, Frame};
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, Vec::new(), true);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    let (base, ice) = ("https://cdn.7tv.app/emote/a/1x.webp", "https://cdn.betterttv.net/emote/b/1x.webp");
    let e = |url: &str, zero_width| Emote { url: url.into(), w: 28, h: 28, zero_width };
    app.res.emotes.set(
      None,
      Provider::SevenTv,
      [("Kappa".into(), e(base, false)), ("frozen0".into(), e(ice, true))].into(),
    );
    let space = |app: &mut App| app.key_insert(Key::Named(NamedKey::Space), Some(" "));
    // typed by hand, a channel or global name stays a word
    app.line.insert("Kappa");
    space(&mut app);
    assert!(app.line.chip_ranges().is_empty());
    app.line.take();
    app.line.insert("hi Kap");
    app.tab_complete(false);
    assert_eq!(app.line.text, "hi Kappa ", "a pick comes with a space to keep typing");
    let tip = app.complete.as_ref().and_then(Complete::tip);
    assert_eq!(tip.as_deref(), Some("1/1 Kappa · global · 7tv users"));
    app.redraw();
    assert!(app.over_input, "the cycle's tooltip shows over the box");
    app.line.insert("frozen0");
    space(&mut app);
    assert_eq!(app.line.text, "hi Kappa frozen0 ", "the words sent are unchanged");
    assert_eq!(app.line.chip_ranges(), vec![3..8, 9..16]);
    let solid = |w: u32, rgb: [u8; 3]| {
      let px: Vec<u8> = (0..w * w).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
      Decoded { w, h: w, frames: vec![Frame { rgba: px.into(), delay_ms: 0 }] }
    };
    for _ in 0..3 {
      app.input_dirty = true;
      app.redraw();
      while let Ok(c) = asked.try_recv() {
        let Cmd::Image(id, url, h) = c else { continue };
        let img = if url.contains("betterttv") { solid(10, [0, 255, 255]) } else { solid(28, [255, 0, 0]) };
        app.on_net(NetEvent::Image(id, h & 0xffff, Ok(img)));
      }
    }
    let r = app.input_area();
    let count = |app: &App, rgb| {
      (r.y..r.bottom())
        .map(|y| (r.x..r.right()).filter(|&x| app.canvas.px[(y * 720 + x) as usize] == rgb).count())
        .sum::<usize>()
    };
    // the base's square with the overlay's smaller square on it: one stack, not two pictures
    assert!(count(&app, 0xff0000) > 28 * 20, "the base is drawn in the box");
    assert_eq!(count(&app, 0x00ffff), 100, "the overlay sits on the base");
    if let Some(path) = std::env::var_os("HSC_DUMP") {
      let c = &app.canvas;
      let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
      c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
      let _ = std::fs::write(path, out);
    }
    // a click puts the cursor there: before the first letter, past the end, on a picture's near side
    let hit = app.input_hit.as_ref().expect("the box remembers where its text is");
    let mid = r.y + r.h / 2;
    assert_eq!(hit.byte_at(r.x, mid), 0);
    assert_eq!(hit.byte_at(r.x + 400, mid), app.line.text.len());
    let red = (r.x..r.right()).find(|&x| app.canvas.px[(mid * 720 + x) as usize] == 0xff0000).expect("the base");
    assert_eq!(hit.byte_at(red + 1, mid), 3, "left half of the picture: before it");
    assert_eq!(hit.byte_at(red + 26, mid), 16, "right half: after the whole stack");
    // the trailing space, then the overlay whole, then the base whole
    for want in ["hi Kappa frozen0", "hi Kappa ", "hi Kappa", "hi "] {
      app.key_insert(Key::Named(NamedKey::Backspace), None);
      assert_eq!(app.line.text, want);
    }
    // vi: esc, hh lands on the whole stack (drawn as one reversed block), x takes it whole
    app.set.vi = true;
    app.mode = Mode::Insert;
    app.line.insert("Kappa frozen0 yo");
    app.key_insert(Key::Named(NamedKey::Escape), None);
    assert_eq!(app.mode, Mode::Normal);
    for _ in 0..3 {
      app.key_normal(Key::Character("h"));
    }
    assert_eq!(app.line.cursor, 3, "on the stack, not inside it");
    app.key_normal(Key::Character("x"));
    assert_eq!(app.line.text, "hi  yo");
  }

  // the line menu like the ext's, and private notes end to end (own HOME: it writes the notes file)
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d cargo test -j2 --release headless_line_menu -- --ignored
  #[test]
  #[ignore]
  fn headless_line_menu() {
    use crate::app::pop::Act;
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    let mut m = Message::system(now_ms(), Platform::Twitch, "x");
    m.kind = crate::model::message::Kind::Chat;
    m.id = "1".into();
    m.login = "bob".into();
    m.nick = "Bob".into();
    m.color = 0x00ff00;
    m.body = "hello\tthere".into();
    app.on_chat("forsen", Chat::Msg(m));
    app.redraw();
    let labels = |app: &App| app.pop.as_ref().map(|p| p.menu.items.iter().map(|r| r.1.clone()).collect::<Vec<_>>());
    let open = |app: &mut App| {
      app.tabs[app.active].view.select_start(&mut app.res, 300);
      app.open_line_menu((10, 10));
      labels(app).expect("a menu")
    };
    let rows = open(&mut app);
    for want in ["copy name", "copy message", "copy \u{2192} input", "hide in tab", "add note", "filter to Bob"] {
      assert!(rows.contains(&want.to_owned()), "{want} in {rows:?}");
    }
    // copy → input: the words, flat, a space after
    app.act(Act::Quote, None, None);
    assert_eq!(app.line.text, "hello there ");
    app.line.take();
    // /note: set, show, edit text comes back into the box, delete
    app.note_cmd(crate::model::notes::command("/note BOB likes cats").unwrap());
    assert_eq!(app.notes.text(Platform::Twitch, "bob"), "likes cats");
    assert!(open(&mut app).contains(&"edit note".to_owned()));
    app.act(Act::Note, app.pop.as_ref().and_then(|p| p.subject.clone()), None);
    assert_eq!(app.line.text, "/note bob likes cats");
    app.line.take();
    app.note_cmd(crate::model::notes::command("/note bob").unwrap());
    assert_eq!(app.notes.text(Platform::Twitch, "bob"), "");
    // hide in tab flips the row
    let who = app.pop.as_ref().and_then(|p| p.subject.clone());
    app.act(Act::HideInTab(true), who, None);
    assert!(app.tabs[app.active].view.hidden_user("bob"));
    // filter to opens the search on @bob, applied
    app.find_for("Bob");
    assert!(app.finding && app.prompt.as_ref().is_some_and(|l| l.text == "@bob"));
  }

  // vi has two modes: normal edits the box, and once a line is picked r/m/c act on it; space leads chat
  // commands (scratch HOME as below):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d cargo test -j2 --release headless_vi_normal -- --ignored
  #[test]
  #[ignore]
  fn headless_vi_normal() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, Vec::new(), true);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    app.set.vi = true;
    app.mode = Mode::Normal;
    let mut m = Message::system(now_ms(), Platform::Twitch, "x");
    m.kind = crate::model::message::Kind::Chat;
    (m.id, m.login, m.nick, m.color, m.body) = ("1".into(), "alice".into(), "alice".into(), 0x00ff00, "yo".into());
    app.on_net(NetEvent::Chat(FLOOD_TAB.into(), Chat::Msg(m)));
    app.redraw();
    let k = |app: &mut App, c: &'static str| app.key_normal(Key::Character(c));
    // nothing picked: r is vim's replace
    app.line.insert("hey");
    Vi::settle(&mut app.line);
    k(&mut app, "r");
    k(&mut app, "a");
    assert_eq!(app.line.text, "hea");
    assert!(app.reply.is_none());
    // a picked line takes r: reply to it, typing
    k(&mut app, "v");
    assert!(app.selecting());
    assert_eq!(app.mode_line().0.first().map(|s| s.0), Some("-- select --"), "short: the box holds text");
    k(&mut app, "r");
    assert_eq!(app.reply.as_ref().map(|r| &*r.nick), Some("alice"));
    assert_eq!(app.mode, Mode::Insert);
    // esc: back to normal (never a third mode), again: let go of the line, again: the reply goes
    app.key_insert(Key::Named(NamedKey::Escape), None);
    assert_eq!(app.mode, Mode::Normal);
    app.key_normal(Key::Named(NamedKey::Escape));
    assert!(!app.selecting() && app.reply.is_some());
    app.key_normal(Key::Named(NamedKey::Escape));
    assert!(app.reply.is_none());
    assert_eq!(app.mode, Mode::Normal);
    // space then x hides the pin, not a char; the leader shows while armed
    app.key_normal(Key::Named(NamedKey::Space));
    assert_eq!(app.mode_line().1, "space");
    k(&mut app, "x");
    assert_eq!(app.line.text, "hea");
    assert_eq!(app.mode_line().1, "");
    k(&mut app, "x");
    assert_eq!(app.line.text.len(), 2, "without space, x deletes a char");
  }

  // `:ka` lists emotes, `@` people, a lone `/b` commands; arrows move, enter picks into the tab cycle,
  // esc shuts the list until the text moves (scratch HOME as below):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d cargo test -j2 --release headless_suggest -- --ignored
  #[test]
  #[ignore]
  fn headless_suggest() {
    use crate::emotes::{Emote, Provider};
    use crate::model::suggest::Kind as S;
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, Vec::new(), true);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    let e = |url: &str| Emote { url: url.into(), w: 28, h: 28, zero_width: false };
    let set = [
      ("Kappa".into(), e("https://cdn.7tv.app/emote/a/1x.webp")),
      ("KappaPride".into(), e("https://cdn.7tv.app/emote/b/1x.webp")),
    ];
    app.res.emotes.set(None, Provider::SevenTv, set.into());
    let mut m = Message::system(now_ms(), Platform::Twitch, "x");
    m.kind = crate::model::message::Kind::Chat;
    (m.id, m.login, m.nick, m.color, m.body) = ("1".into(), "alice".into(), "alice".into(), 0x00ff00, "yo".into());
    app.on_net(NetEvent::Chat(FLOOD_TAB.into(), Chat::Msg(m)));
    let key = |app: &mut App, k: NamedKey| app.key_insert(Key::Named(k), None);
    let open = |app: &mut App| {
      app.input_dirty = true;
      app.redraw();
      app
        .suggest
        .as_ref()
        .map(|s| (s.kind, s.items.iter().filter(|p| p.code.is_none()).map(|p| p.name.to_string()).collect::<Vec<_>>()))
    };
    app.line.insert("so :ka");
    assert_eq!(open(&mut app), Some((S::Emote, vec!["Kappa".into(), "KappaPride".into()])));
    assert!(app.over_input && !app.suggest_rows.is_empty(), "the list is on screen");
    key(&mut app, NamedKey::ArrowDown);
    key(&mut app, NamedKey::Enter);
    assert_eq!(app.line.text, "so KappaPride ");
    assert_eq!(app.line.chip_ranges(), vec![3..13]);
    // the list carries on as the tab cycle
    key(&mut app, NamedKey::Tab);
    assert!(
      app.line.chip_ranges().is_empty() && app.line.text.starts_with("so "),
      "next comes the emoji: text, no chip"
    );
    app.tab_complete(true);
    app.tab_complete(true);
    assert_eq!(app.line.text, "so Kappa ");
    assert_eq!(app.line.chip_ranges(), vec![3..8]);
    app.line.take();
    app.complete = None;
    app.line.insert("hi @al");
    assert_eq!(open(&mut app).map(|o| o.0), Some(S::People));
    key(&mut app, NamedKey::Tab);
    assert_eq!(app.line.text, "hi @alice ");
    app.line.take();
    app.complete = None;
    app.line.insert("/ba");
    let cmds = open(&mut app).expect("a lone /word lists commands");
    assert!(cmds.0 == S::Command && cmds.1.contains(&"/ban".to_string()));
    if let Some(path) = std::env::var_os("HSC_DUMP") {
      let c = &app.canvas;
      let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
      c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
      let _ = std::fs::write(path, out);
    }
    key(&mut app, NamedKey::Escape);
    assert_eq!(open(&mut app), None, "esc shuts it");
    assert_eq!(app.line.text, "/ba");
    app.line.insert("n");
    assert!(open(&mut app).is_some(), "typing opens it again");
  }

  fn dump_frame(app: &App) {
    if let Some(path) = std::env::var_os("HSC_DUMP") {
      let c = &app.canvas;
      let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
      c.px.iter().for_each(|p| out.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
      let _ = std::fs::write(path, out);
    }
  }

  // `:smi` lists emotes first, then emoji (glyph and :shortcode:); picking puts the glyph in the box;
  // a typed :shortcode: becomes the glyph on send. HSC_DUMP=$d/x.ppm (the list) ... cargo test -j2
  // --release headless_picker_emoji -- --ignored  (scratch HOME as above)
  // a stream line made before the streamer is known gets their color and twitch id when a lookup or the
  // followed colors land; the frame's own color is never overruled; followed colors beat chat's
  #[test]
  fn stream_lines_resolve_late() {
    use crate::model::card::Info;
    use crate::model::color;
    use crate::model::stream_events::{Line, Tone};
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let line = |ch: &str, color| Line {
      platform: Platform::Twitch,
      channel: ch.into(),
      text: "x".into(),
      tone: Tone::Switch,
      name: ch.into(),
      color,
      verb: "◆ switched to ".into(),
      game: "Chess".into(),
    };
    let ft = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    let last = |app: &App| app.tabs[ft].view.last_msg().map(|m| (m.color, m.user_id.to_string())).unwrap();
    app.on_stream(line("knut", None));
    app.on_stream(line("framed", Some(0x00ff00)));
    let framed = last(&app);
    assert_eq!(framed.0, color::readable(0x00ff00));
    app.tabs[ft].view.restyle_late(|_, _| (1, None));
    assert_eq!(last(&app), framed, "a frame's own color is not late");
    // before: palette color, no id
    app.on_stream(line("knut", None));
    let before = last(&app);
    assert_eq!(before, (color::readable(color::default_for("knut")), String::new()));
    // a lookup lands: chat's color + the id for badges and paints
    let info = Info { color: Some(0xff0000), twitch_id: Some(4242), ..Info::default() };
    app.learn_card(Platform::Twitch, "knut", Ok(info));
    assert_eq!(last(&app), (color::readable(0xff0000), "4242".to_string()));
    // followed colors outrank it
    app.on_follow_colors(vec![("knut".into(), 0x00ffff)]);
    assert_eq!(last(&app), (color::readable(0x00ffff), "4242".to_string()));
    // and a brand-new line follows the same chain
    app.on_stream(line("knut", None));
    assert_eq!(last(&app), (color::readable(0x00ffff), "4242".to_string()));
  }

  #[test]
  fn a_height_only_resize_paints_the_chat_again() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(420, 300);
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    for n in 0..40 {
      app.tabs[chat].chat().push(Message::system(now_ms(), Platform::Twitch, format!("line {n} with some words")));
    }
    app.redraw();
    let lit = |app: &App| {
      let a = app.chat_area();
      (a.y..a.bottom())
        .flat_map(|y| (a.x..a.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| app.canvas.px[(y * app.canvas.w + x) as usize] != 0)
        .count()
    };
    let before = lit(&app);
    assert!(before > 500, "the chat drew ({before} px)");
    // a new window tiles beside this one: same width, shorter
    app.resized(420, 240);
    app.redraw();
    assert!(lit(&app) > 300, "the chat paints again after a height-only resize, not only its animations");
  }

  #[test]
  fn the_card_asks_pronoundb_once_and_shows_the_answer_beside_the_name() {
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.set.pronouns = true;
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    let person = |login: &str, uid: &str| {
      let mut m = Message::system(now_ms(), Platform::Twitch, "hi");
      m.kind = crate::model::message::Kind::Chat;
      (m.login, m.nick, m.user_id) = (login.into(), login.into(), uid.into());
      m
    };
    let alice = person("alice", "42");
    let who = app.subject_of(&alice).expect("a chatter");
    let drain = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
      std::iter::from_fn(|| asked.try_recv().ok())
        .filter_map(|c| if let Cmd::Pronouns(u) = c { Some(u) } else { None })
        .collect::<Vec<_>>()
    };
    app.open_card(who.clone());
    assert_eq!(drain(&mut asked), vec![42], "her line's twitch id is asked about, at once");
    assert_eq!(app.pronouns_of(app.card_twitch_id()), None, "nothing shows until it lands");
    app.on_net(NetEvent::Pronouns(42, Ok(Some("she/they".into()))));
    assert_eq!(app.pronouns_of(app.card_twitch_id()), Some("she/they"));
    app.redraw();
    // closed and opened again: the answer is kept, nothing is asked
    app.card = None;
    app.open_card(who);
    assert!(drain(&mut asked).is_empty());
    // switched off: nothing shows, nobody new is asked about
    app.set.pronouns = false;
    assert_eq!(app.pronouns_of(app.card_twitch_id()), None);
    app.open_card(app.subject_of(&person("bob", "43")).unwrap());
    assert!(drain(&mut asked).is_empty(), "pronoundb never hears of bob");
  }

  #[test]
  fn chat_logs_left_open_on_one_tab_keep_their_keys_to_that_tab() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
    (app.set.vi, app.mode) = (true, Mode::Normal);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    let tab = |app: &App, n: &str| app.tabs.iter().position(|t| &*t.name == n).unwrap();
    let (a, b) = (tab(&app, "forsen"), tab(&app, "xqc"));
    let line = |id: &str| {
      let mut m = Message::system(now_ms(), Platform::Twitch, "hello");
      m.kind = crate::model::message::Kind::Chat;
      (m.id, m.login, m.nick) = (id.into(), "bob".into(), "bob".into());
      m
    };
    app.select(a);
    app.tabs[a].chat().push(line("1"));
    let who = app.subject_of(&line("1")).unwrap();
    app.logs_open(&who);
    assert!(app.logs_shown().is_some());
    // off to another tab without closing them: its lines answer r and m as always
    app.select(b);
    app.tabs[b].chat().push(line("2"));
    app.redraw();
    assert!(app.logs_shown().is_none());
    app.on_key(Key::Character("v"), Some("v"));
    app.on_key(Key::Character("r"), Some("r"));
    assert_eq!(app.reply.as_ref().map(|r| &*r.id), Some("2"), "r replies here, it doesn't refresh the hidden logs");
    app.mode = Mode::Normal;
    app.on_key(Key::Character("m"), Some("m"));
    assert!(app.pop.is_some(), "the line menu opens");
    app.pop = None;
    assert!(!app.hint().contains("y copy link"), "the box speaks for this tab");
    // back on their tab they are as left, and their keys work
    app.select(a);
    assert!(app.logs_shown().is_some());
    app.on_key(Key::Character("q"), Some("q"));
    assert!(app.logs.is_none() && app.tabs[a].under.is_none(), "q closes them");
  }

  #[test]
  #[ignore]
  fn headless_picker_emoji() {
    use crate::emotes::{Emote, Provider};
    use crate::model::suggest::Kind as S;
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    let e = |url: &str| Emote { url: url.into(), w: 28, h: 28, zero_width: false };
    app.res.emotes.set(None, Provider::SevenTv, [("smirkCat".into(), e("https://cdn.7tv.app/emote/a/1x.webp"))].into());
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    app.enter_insert();
    let key = |app: &mut App, k: NamedKey| app.key_insert(Key::Named(k), None);
    app.line.insert("so :smi");
    app.input_dirty = true;
    app.redraw();
    let s = app.suggest.as_ref().expect("the list opens");
    assert_eq!(s.kind, S::Emote);
    assert_eq!(&*s.items[0].name, "smirkCat", "emotes first, like the ext");
    let first_emoji = s.items.iter().position(|p| p.code.is_some()).expect("emoji follow");
    assert!(first_emoji >= 1);
    assert_eq!(s.items[first_emoji].code, Some("smile"), "shortest prefix first");
    assert!(app.over_input && !app.suggest_rows.is_empty());
    dump_frame(&app);
    // the glyph is what goes in the box
    app.suggest.as_mut().unwrap().sel = first_emoji;
    key(&mut app, NamedKey::Enter);
    assert_eq!(app.line.text, "so 😄 ");
    assert!(app.line.chips.is_empty(), "an emoji is text, not a picture chip");
    assert!(app.complete.as_ref().and_then(|c| c.tip()).unwrap().contains(":smile:"), "the tip names the code");
    // tab with a bare :word cycles emoji too
    app.line.take();
    app.complete = None;
    app.line.insert(":joy");
    app.tab_complete(false);
    assert!(app.line.text.starts_with('😂'), "{}", app.line.text);
    // send turns shortcodes into glyphs
    app.complete = None;
    app.line.take();
    app.line.insert("gg :fire: :100: :nope:");
    while asked.try_recv().is_ok() {}
    app.send();
    let sent = std::iter::from_fn(|| asked.try_recv().ok()).find_map(|c| match c {
      Cmd::Send { text, .. } => Some(text),
      _ => None,
    });
    assert_eq!(sent.as_deref(), Some("gg 🔥 💯 :nope:"));
  }

  // the picker's gifs tab: ctrl-tab / [ ] switch, typing asks heatsync after a rest (old answers drop),
  // an answer fills a grid of stills, the rested pick plays, enter inserts the link and keeps recents.
  // HSC_DUMP=$d/x.ppm (the grid) ... cargo test -j2 --release headless_picker_gifs -- --ignored
  #[test]
  #[ignore]
  fn headless_picker_gifs() {
    use crate::images::decode::{Decoded, Frame};
    use crate::model::gifs;
    use crate::model::picker::Tab;
    use crate::net::social;
    use serde_json::json;
    crate::store::remove("recent-gifs");
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 400);
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    app.enter_insert();
    let gif_asks = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
      std::iter::from_fn(|| asked.try_recv().ok())
        .filter_map(|c| match c {
          Cmd::Social(social::Cmd::Gifs { q, epoch }) => Some((q, epoch)),
          _ => None,
        })
        .collect::<Vec<_>>()
    };
    let page = |tag: &str, n: u32| {
      let rows: Vec<_> = (0..n)
        .map(|i| {
          json!({"id": i, "provider": "giphy", "label": format!("{tag} {i}"),
          "url": format!("https://media.giphy.com/media/{tag}{i}/giphy.gif"),
          "preview": format!("https://media.giphy.com/media/{tag}{i}/200_s.gif"),
          "animated": format!("https://media.giphy.com/media/{tag}{i}/200.gif")})
        })
        .collect();
      gifs::parse(&json!({"gifs": rows, "library": {"state": "ready", "indexed": 9}}))
    };
    // every picture asked for lands as a flat tile, one color per url
    let land = |app: &mut App, asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| -> Vec<String> {
      let mut urls = Vec::new();
      for _ in 0..3 {
        app.redraw();
        let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
          .filter_map(|c| if let Cmd::Image(id, url, target) = c { Some((id, url, target)) } else { None })
          .collect();
        if wanted.is_empty() {
          break;
        }
        for (id, url, target) in wanted {
          let tone = url.bytes().fold(60u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
          let px = [(tone & 0xff) as u8 | 0x40, (tone >> 8 & 0xff) as u8 | 0x40, (tone >> 16 & 0xff) as u8 | 0x40, 255];
          let d = Decoded { w: 80, h: 45, frames: vec![Frame { rgba: px.repeat(80 * 45).into(), delay_ms: 0 }] };
          urls.push(url.to_string());
          app.res.images.loaded(id, target, Ok(d));
        }
      }
      urls
    };
    app.toggle_picker();
    assert_eq!(app.picker.as_ref().unwrap().tab, Tab::Emotes);
    // grid focus: ] goes to gifs; a cold box with no recents asks for the top list at once
    app.picker.as_mut().unwrap().focus = Focus::Grid;
    app.picker_key(Key::Character("]"), Some("]"));
    assert_eq!(app.picker.as_ref().unwrap().tab, Tab::Gifs);
    let top = gif_asks(&mut asked);
    assert_eq!(top.len(), 1, "{top:?}");
    assert_eq!(&*top[0].0, "");
    app.redraw();
    assert_eq!(app.picker.as_ref().unwrap().note, "searching…");
    // a stale answer (older epoch) is dropped; the right one fills the grid
    app.on_net(NetEvent::Gifs { epoch: top[0].1 + 7, q: "".into(), res: Ok(page("old", 3)) });
    assert!(app.picker.as_ref().unwrap().selected_gif().is_none());
    app.on_net(NetEvent::Gifs { epoch: top[0].1, q: "".into(), res: Ok(page("top", 24)) });
    assert_eq!(app.picker.as_ref().unwrap().selected_gif().map(|g| &*g.label), Some("top 0"));
    assert!(app.picker.as_ref().unwrap().note.is_empty());
    // typing asks once after a rest, not per key; a bad answer says why
    app.picker.as_mut().unwrap().focus = Focus::Search;
    for c in ["c", "a", "t"] {
      app.picker_key(Key::Character(c), Some(c));
    }
    assert!(gif_asks(&mut asked).is_empty(), "nothing before the rest is over");
    app.gifs_tick(Instant::now() + Duration::from_secs(1));
    let ask = gif_asks(&mut asked);
    assert_eq!(ask.len(), 1);
    assert_eq!(&*ask[0].0, "cat");
    app.on_net(NetEvent::Gifs {
      epoch: ask[0].1,
      q: "cat".into(),
      res: Err(gifs::Fail::of(429, &json!({"resetAt": now_ms() + 7_500}), now_ms())),
    });
    assert!(app.picker.as_ref().unwrap().note.contains("wait 8s"), "{}", app.picker.as_ref().unwrap().note);
    // the same text tries again on the next change
    app.picker_key(Key::Named(NamedKey::Backspace), None);
    app.picker_key(Key::Character("t"), Some("t"));
    app.gifs_tick(Instant::now() + Duration::from_secs(1));
    let ask = gif_asks(&mut asked);
    assert_eq!(ask.len(), 1);
    app.on_net(NetEvent::Gifs { epoch: ask[0].1, q: "cat".into(), res: Ok(page("cat", 24)) });
    // the grid paints stills for what is in view; the selected one has not started to play
    let urls = land(&mut app, &mut asked);
    assert!(!urls.is_empty() && urls.len() <= 24, "{}", urls.len());
    assert!(
      urls.iter().all(|u| u.starts_with("https://heatsync.org/api/img?url=") && u.contains("200_s.gif")),
      "{urls:?}"
    );
    dump_frame(&app);
    // a rested pick asks for its moving picture, once
    app.picker_key(Key::Named(NamedKey::ArrowRight), None);
    app.gifs_tick(Instant::now() + Duration::from_secs(1));
    assert!(app.picker.as_ref().unwrap().hot);
    let moving = land(&mut app, &mut asked);
    assert_eq!(moving.len(), 1, "{moving:?}");
    assert!(moving[0].contains("%2F200.gif"), "{moving:?}");
    // the same search again comes from the cache, no request
    app.picker.as_mut().unwrap().clear_query();
    app.picker_changed();
    app.gifs_tick(Instant::now() + Duration::from_secs(1));
    assert_eq!(app.picker.as_ref().unwrap().selected_gif().map(|g| &*g.label), Some("top 0"), "the top list, cached");
    app.picker.as_mut().unwrap().type_str("cat");
    app.picker_changed();
    app.gifs_tick(Instant::now() + Duration::from_secs(1));
    assert!(gif_asks(&mut asked).is_empty());
    assert_eq!(app.picker.as_ref().unwrap().selected_gif().map(|g| &*g.label), Some("cat 0"));
    // enter puts the link in the box, closes, and remembers it
    app.picker_key(Key::Named(NamedKey::ArrowRight), None);
    app.picker_key(Key::Named(NamedKey::Enter), None);
    assert!(app.picker.is_none());
    assert_eq!(app.line.text, "https://media.giphy.com/media/cat1/giphy.gif ");
    assert_eq!(app.gifs.recent.as_ref().map(|r| r.len()), Some(1));
    // reopening: the box is on the gifs you sent, no request; ctrl-tab goes back to emotes
    app.line.take();
    app.toggle_picker();
    app.picker_tab(Some(Tab::Gifs));
    assert!(gif_asks(&mut asked).is_empty());
    let g = app.picker.as_ref().unwrap().selected_gif().cloned().unwrap();
    assert_eq!(&*g.url, "https://media.giphy.com/media/cat1/giphy.gif");
    app.redraw();
    app.picker_tab(None);
    assert_eq!(app.picker.as_ref().unwrap().tab, Tab::Emotes);
    app.close_picker();
    assert!(app.gifs.due().is_none());
  }

  // a channel's went-live lands in every chat tab; only its own tab turns yellow (scratch HOME as below):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d cargo test -j2 --release headless_stream_everywhere -- --ignored
  #[test]
  #[ignore]
  fn headless_stream_everywhere() {
    use crate::model::stream_events::{Line, Tone};
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
    let tab = |app: &App, n: &str| app.tabs.iter().position(|t| &*t.name == n).unwrap();
    let (forsen, xqc) = (tab(&app, "forsen"), tab(&app, "xqc"));
    app.select(forsen);
    let line = |ch: &str| Line {
      platform: Platform::Twitch,
      channel: ch.into(),
      text: format!("[{ch}] ◆ went live").into(),
      tone: Tone::Online,
      name: ch.into(),
      color: None,
      verb: "◆ went live".into(),
      game: "".into(),
    };
    let n = |app: &App, i: usize| app.tabs[i].view.len();
    let before = (n(&app, forsen), n(&app, xqc));
    // followed, no tab: both tabs get it, neither turns yellow
    app.on_stream(line("ennortix"));
    assert_eq!((n(&app, forsen), n(&app, xqc)), (before.0 + 1, before.1 + 1));
    assert!(!app.tabs[xqc].event && !app.tabs[xqc].unread, "someone else's news lights no tab");
    // a tabbed channel: everywhere too, and its own tab turns yellow
    app.on_stream(line("xqc"));
    assert_eq!(n(&app, forsen), before.0 + 2);
    assert!(app.tabs[xqc].event && !app.tabs[forsen].event);
    // drawn like the ext: name in its color, `◆ verb` in the tone, the game white, a stripe at the left
    app.on_stream(Line {
      color: Some(0x00ff00),
      tone: Tone::Switch,
      verb: "◆ switched to ".into(),
      game: "Chess".into(),
      ..line("forsen")
    });
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 200);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    app.redraw();
    let has = |rgb| app.canvas.px.contains(&rgb);
    assert!(has(0x00ff00) && has(crate::ui::theme::YELLOW), "the name in its color, the verb in yellow");
    crate::bench::dump(&app.canvas);
  }

  // an @mention while the window is away flashes the title with who it was, the latest name winning,
  // until the window comes back; focused, nothing flashes (scratch HOME as below):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d cargo test -j2 --release headless_mention_flash -- --ignored
  #[test]
  #[ignore]
  fn headless_mention_flash() {
    use crate::model::message::Kind;
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    // silent: a test never plays the ping
    app.set.mention_volume = crate::model::settings::Volume::V0;
    app.set.title_flash = true;
    app.mentions.set(["mellen"]);
    let line = |app: &mut App, who: &str| {
      let mut m = Message::system(1, Platform::Twitch, "x");
      (m.kind, m.login, m.nick, m.body) = (Kind::Chat, who.into(), who.into(), "hi @mellen".into());
      app.scan_mention(Platform::Twitch, "forsen", &mut m);
    };
    line(&mut app, "bob");
    assert!(app.flash.is_none(), "focused: you saw it");
    app.focused = false;
    line(&mut app, "bob");
    line(&mut app, "eve\u{7}");
    assert_eq!(app.flash.as_ref().map(|f| &*f.0), Some("eve"), "the latest, control bytes gone");
    app.flash_tick();
    assert!(app.flash.as_ref().is_some_and(|f| f.1) && app.flash_due.is_some(), "showing the name");
    app.flash_tick();
    assert!(app.flash.as_ref().is_some_and(|f| !f.1), "and back");
    app.focused = true;
    app.flash_stop();
    assert!(app.flash.is_none() && app.flash_due.is_none());
    // switched off mid-flash: the next turn stops it
    app.focused = false;
    line(&mut app, "bob");
    app.set.title_flash = false;
    app.flash_tick();
    assert!(app.flash.is_none() && app.flash_due.is_none());
  }

  // a stream line turns an unwatched tab yellow, and right-click clears a red, cyan or yellow tab
  // without opening it or its menu, offline (scratch HOME as below):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d cargo test -j2 --release headless_tab_ack -- --ignored
  #[test]
  #[ignore]
  fn headless_tab_ack() {
    use crate::model::stream_events::Tone;
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, Vec::new(), true);
    app.canvas.resize(720, 360);
    let at = |app: &App, k: Kind| app.tabs.iter().position(|t| t.kind == k).unwrap();
    let (feed, flood) = (at(&app, Kind::Feed), app.tabs.iter().position(|t| &*t.name == FLOOD_TAB).unwrap());
    let (mentions, whispers) = (at(&app, Kind::Mentions), at(&app, Kind::Whispers));
    app.select(feed);
    app.say_line(flood, Message::system(0, Platform::Twitch, "switched to chess"), Tone::Switch);
    assert!(app.tabs[flood].event && app.tabs[flood].unread, "the news turns the tab yellow");
    // a mention lights the tab; looking on another client before it changes nothing, after it puts it out
    use crate::model::unseen::Surface;
    app.unseen_note(Surface::Mentions);
    assert!(app.tabs[mentions].unread);
    app.on_net(NetEvent::Seen { surface: Surface::Mentions, at: now_ms() - 60_000 });
    assert!(app.tabs[mentions].unread, "an older look elsewhere leaves it lit");
    app.on_net(NetEvent::Seen { surface: Surface::Mentions, at: now_ms() + 1 });
    assert!(!app.tabs[mentions].unread, "a look on another client puts it out");
    app.say_line(flood, Message::system(0, Platform::Twitch, "[🔥] chat is popping"), Tone::Moment);
    app.tabs[mentions].unread = true;
    app.tabs[whispers].unread = true;
    let right_click = |app: &mut App, i: usize| {
      app.bar_dirty = true;
      app.redraw();
      let p = app.bar_map.iter().position(|&t| t == i).expect("the tab is in the bar");
      let r = app.bar_hits[p];
      app.pointer = (r.x + r.w / 2, r.y + r.h / 2);
      app.right_click();
    };
    right_click(&mut app, flood);
    assert!(!app.tabs[flood].event && app.tabs[flood].unread, "yellow clears, the plain unread stays");
    for i in [mentions, whispers] {
      right_click(&mut app, i);
      assert!(!app.tabs[i].unread && app.tab_menu.is_none(), "cleared, no menu");
    }
    assert_eq!(app.active, feed, "nothing was opened");
    right_click(&mut app, mentions);
    assert!(app.tab_menu.is_some(), "a quiet tab still gets its menu");
    // opening a yellow tab clears it too
    app.tab_menu = None;
    app.say_line(flood, Message::system(0, Platform::Twitch, "went offline"), Tone::Offline);
    app.select(flood);
    assert!(!app.tabs[flood].event);
  }

  // a click on a reply's >>id opens its parent's thread with the reply picked out, and a click on
  // the open thread's own >>id goes back to the list, offline (scratch HOME as below):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d cargo test -j2 --release headless_feed_thread -- --ignored
  #[test]
  #[ignore]
  fn headless_feed_thread_click() {
    use crate::model::post::parse;
    use crate::net::social;
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, Vec::new(), true);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let post = |id: &str, reply_to: Option<&str>| {
      let mut v = serde_json::json!({"base36_id": id, "username": "alice", "content": "hi"});
      if let Some(r) = reply_to {
        v["reply_to"] = r.into();
      }
      parse(&v).unwrap()
    };
    let (op, re) = (post("00004c", None), post("00004d", Some("00004c")));
    app.feed.posts = vec![re.clone(), op.clone()];
    app.feed.loaded = true;
    let feed = app.tabs.iter().position(|t| t.kind == Kind::Feed).expect("a feed tab");
    app.select(feed);
    app.feed_show();
    app.redraw();
    while asked.try_recv().is_ok() {}
    // the >>id of the line holding `id`
    let id_spot = |app: &mut App, id: &str| {
      let area = app.chat_area();
      (area.x..area.right()).find_map(|x| {
        (area.y..area.bottom()).find_map(|y| {
          let res = &mut app.res;
          (app.tabs[feed].view.thread_at(area, x, y, res)?.as_ref() == id).then_some((x, y))
        })
      })
    };
    app.pointer = id_spot(&mut app, "00004d").expect("the reply's >>4d is on screen");
    app.click();
    let t = app.feed.thread.as_ref().expect("the click opens a thread");
    assert_eq!(&*t.id, "00004c", "a reply opens its parent's thread");
    let fetched = std::iter::from_fn(|| asked.try_recv().ok())
      .any(|c| matches!(c, Cmd::Social(social::Cmd::Thread { ref id }) if &**id == "00004c"));
    assert!(fetched, "the thread is fetched");
    app.on_net(NetEvent::Thread { id: "00004c".into(), res: Ok((op, vec![re])) });
    app.redraw();
    let picked = app.tabs[feed].view.selected().map(|m| m.id.clone());
    assert_eq!(picked.as_deref(), Some("00004d"), "the clicked reply is picked out");
    app.pointer = id_spot(&mut app, "00004c").expect("the op's >>4c heads the thread");
    app.click();
    assert!(app.feed.thread.is_none(), "the open thread's own >>id goes back");
    assert!(app.tabs[feed].under.is_none(), "and the list is back in its tab");
  }

  // right-click on a chat line opens its menu; r on a picked line starts a reply (scratch HOME as above)
  #[test]
  #[ignore]
  fn headless_line_menu_and_r() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    let mut m = Message::system(now_ms(), Platform::Twitch, "hello there");
    m.kind = crate::model::message::Kind::Chat;
    m.id = "abc".into();
    m.nick = "bob".into();
    m.login = "bob".into();
    app.on_chat("forsen", Chat::Msg(m));
    app.redraw();
    let area = app.chat_area();
    // the newest line sits at the bottom of the chat
    app.pointer = (area.x + 200, area.bottom() - 6);
    app.right_click();
    assert!(app.pop.is_some(), "right-click opens the line menu");
    app.pop = None;
    app.mode = Mode::Normal;
    app.key_normal(Key::Character("r"));
    assert!(app.reply.is_some(), "r on the picked line starts a reply");
  }

  // a feed post lands as a line in a chat tab; its >>id opens the thread right there, the tab reads
  // `← back`, chat keeps landing underneath, and back puts the chat where it was (scratch HOME as above):
  //   ... cargo test -j2 --release headless_thread_takeover -- --ignored
  #[test]
  #[ignore]
  fn headless_thread_takeover() {
    use crate::model::post::parse;
    use crate::net::social;
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    let line = |t: &str| {
      let mut m = Message::system(now_ms(), Platform::Twitch, t);
      m.kind = crate::model::message::Kind::Chat;
      m.nick = "bob".into();
      m.login = "bob".into();
      m
    };
    app.on_chat("forsen", Chat::Msg(line("before")));
    let op =
      parse(&serde_json::json!({"base36_id": "00003t", "username": "alice", "user_id": "5", "content": "a thread"}))
        .unwrap();
    app.on_feed_live(op.clone());
    app.on_feed_live(op.clone());
    let rows = app.tabs[chat].view.len();
    assert_eq!(rows, 2, "the post is one line in the chat tab, heard once");
    app.on_chat("forsen", Chat::Msg(line("see >>3t")));
    app.redraw();
    while asked.try_recv().is_ok() {}
    let area = app.chat_area();
    let spot = |app: &mut App, want: &str| {
      (area.y..area.bottom()).find_map(|y| {
        (area.x..area.right()).find_map(|x| {
          let res = &mut app.res;
          (app.tabs[chat].view.thread_at(area, x, y, res)?.as_ref() == want).then_some((x, y))
        })
      })
    };
    // both the post's own >>3t and the one written in chat name it; click the written one
    let hits: Vec<_> = (area.y..area.bottom())
      .filter_map(|y| {
        let res = &mut app.res;
        (area.x..area.right()).find(|&x| app.tabs[chat].view.thread_at(area, x, y, res).is_some()).map(|x| (x, y))
      })
      .collect();
    assert!(hits.len() >= 2, "a post's >>id and one typed in chat are both clickable");
    app.pointer = *hits.last().unwrap();
    app.click();
    assert!(app.tabs[chat].under.is_some(), "the thread took the chat tab over");
    assert!(app.feedish(), "keys and typing go to the thread");
    let fetched = std::iter::from_fn(|| asked.try_recv().ok())
      .any(|c| matches!(c, Cmd::Social(social::Cmd::Thread { ref id }) if &**id == "00003t"));
    assert!(fetched);
    app.on_net(NetEvent::Thread { id: "00003t".into(), res: Ok((op, vec![])) });
    app.on_chat("forsen", Chat::Msg(line("during")));
    assert_eq!(app.tabs[chat].view.len(), 2, "the thread view: the op and 'no replies yet'");
    app.redraw();
    assert!(spot(&mut app, "00003t").is_some());
    crate::bench::dump(&app.canvas);
    // the tab reads `← back`; a click on it goes back
    let p = app.bar_map.iter().position(|&i| i == chat).unwrap();
    let r = app.bar_hits[p];
    app.pointer = (r.x + 2, r.y + 2);
    app.bar_click();
    assert!(app.tabs[chat].under.is_none() && app.feed.thread.is_none());
    assert_eq!(app.tabs[chat].view.len(), rows + 2, "chat that landed meanwhile is there");
  }

  // right-click a line → chat logs: one person's archive over the tab, drawn as chat, older pages at
  // the top, a to swap scope, / to search, esc back (scratch HOME as above):
  //   HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_chat_logs -- --ignored
  #[test]
  #[ignore]
  fn headless_chat_logs() {
    use crate::model::logs::{self, Who};
    use crate::net::social;
    use serde_json::json;
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    app.res.look.timestamps = false;
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    let line = |who: &str, t: &str| {
      let mut m = Message::system(now_ms(), Platform::Twitch, t);
      m.kind = crate::model::message::Kind::Chat;
      m.nick = who.into();
      m.login = who.into();
      m.id = format!("live-{t}").into();
      m
    };
    app.on_chat("forsen", Chat::Msg(line("bob", "hello chat")));
    let before = app.tabs[chat].view.len();
    // the line menu has the row; picking it opens the view
    app.tabs[chat].view.select_start(&mut app.res, 300);
    app.open_line_menu((20, 20));
    let rows = app.pop.as_ref().map(|p| p.menu.items.iter().map(|r| r.1.clone()).collect::<Vec<_>>()).unwrap();
    assert_eq!(rows.last().map(String::as_str), Some("chat logs"), "after filter to, like the ext");
    app.pop_key(Key::Named(NamedKey::Escape));
    let m = app.tabs[chat].view.selected().cloned().unwrap();
    let who = app.subject_of(&m).unwrap();
    app.act(pop::Act::Logs, Some(who), None);
    assert!(app.logs.is_some() && app.tabs[chat].under.is_some(), "the chat tab is taken over");
    assert!(!app.feedish(), "not a feed thread: no votes, no posting");
    let asks = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
      std::iter::from_fn(|| asked.try_recv().ok())
        .filter_map(|c| match c {
          Cmd::Social(social::Cmd::Logs(r)) => Some(r),
          _ => None,
        })
        .collect::<Vec<_>>()
    };
    let first = asks(&mut asked);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].path(), "/api/archive/user/twitch/bob/messages?limit=100&channel=forsen");
    let w = Who { platform: Platform::Twitch, login: "bob".into(), nick: "bob".into(), user_id: "".into() };
    let row = |n: i64, ch: &str, html: &str| {
      json!({"message": format!("archived {n}"), "message_id": format!("m{n}"), "channel": ch,
        "timestamp": format!("2026-10-3{}T23:59:5{}.000Z", n % 2, n % 10), "color": "#00FF00", "display_name": "Bob",
        "message_html": html})
    };
    let page = |from: i64, n: i64, cursor: Option<&str>| {
      let rows: Vec<_> = (from..from + n).rev().map(|i| row(i, "forsen", "")).collect();
      logs::parse_page(&json!({"results": rows, "next_cursor": cursor}), &w)
    };
    // a stale answer (from before something else) is ignored
    app.on_net(NetEvent::Logs { epoch: first[0].epoch + 1000, res: Ok(page(0, 3, None)) });
    assert!(app.tabs[chat].view.len() == 0, "nothing from a stale answer");
    app.on_net(NetEvent::Logs { epoch: first[0].epoch, res: Ok(page(10, 40, Some("2026-10-30|1"))) });
    assert_eq!(app.tabs[chat].view.len(), 41, "forty lines and the dim line at the top");
    app.redraw();
    crate::bench::dump(&app.canvas);
    // keys: j picks the newest, y finds its public link, g goes to the top and asks for older
    assert!(app.logs_key(Key::Character("j")));
    let picked = app.tabs[chat].view.selected().cloned().unwrap();
    assert_eq!(&*picked.id, "m49");
    assert_eq!(
      app.logs.as_ref().unwrap().link_of(&picked).as_deref(),
      Some("https://heatsync.org/search/logs/twitch/forsen/2026-10-31?m=m49")
    );
    assert!(app.logs_key(Key::Character("g")));
    let older = asks(&mut asked);
    assert_eq!(older.len(), 1, "the top asks for the next page");
    assert_eq!(older[0].cursor.as_deref(), Some("2026-10-30|1"));
    app.on_net(NetEvent::Logs { epoch: older[0].epoch, res: Ok(page(0, 10, None)) });
    assert_eq!(app.tabs[chat].view.len(), 51, "ten older lines in front");
    assert!(app.logs.as_ref().unwrap().done, "no cursor left");
    // a: every channel. the old answer is dropped, a fresh ask has no channel filter
    assert!(app.logs_key(Key::Character("a")));
    let all = asks(&mut asked);
    assert_eq!(all.len(), 1);
    assert!(all[0].channel.is_none());
    let html = "hi <img class=\"hs-archive-emote\" src=\"/e/7tv/01ABC\" alt=\"LUL\">";
    let mut rows: Vec<_> = (0..6).rev().map(|i| row(i, if i % 2 == 0 { "forsen" } else { "xqc" }, "")).collect();
    rows.push(json!({"message": "hi LUL", "message_html": html, "channel": "xqc", "message_id": "e1",
      "timestamp": "2026-10-31T00:00:00Z"}));
    rows.rotate_right(1);
    let p = logs::parse_page(&json!({"results": rows}), &w);
    app.on_net(NetEvent::Logs { epoch: all[0].epoch, res: Ok(p) });
    assert_eq!(app.tabs[chat].view.len(), 7 + 1);
    // live: bob in another room joins the bottom of the every-channel view, alice never does
    app.on_chat("forsen", Chat::Msg(line("alice", "not bob")));
    app.on_chat("forsen", Chat::Msg(line("bob", "live line")));
    assert_eq!(app.tabs[chat].view.len(), 9);
    assert_eq!(app.tabs[chat].under.as_ref().unwrap().len(), before + 2, "chat keeps landing underneath");
    app.redraw();
    crate::bench::dump(&app.canvas);
    // / searches the archive on enter; esc clears it, then esc leaves
    assert!(app.logs_key(Key::Character("/")));
    assert!(app.finding && app.prompt.is_some());
    for c in "lul".chars() {
      let t = c.to_string();
      app.find_key(Key::Character(&t), Some(&t));
    }
    app.find_key(Key::Named(NamedKey::Enter), None);
    let s = asks(&mut asked);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].query.as_deref(), Some("lul"));
    app.on_net(NetEvent::Logs { epoch: s[0].epoch, res: Ok(logs::parse_page(&json!({"results": []}), &w)) });
    assert_eq!(app.tabs[chat].view.len(), 0, "an empty search says so");
    assert!(app.logs_key(Key::Named(NamedKey::Escape)));
    assert!(app.logs.as_ref().unwrap().query.is_none(), "esc clears the search first");
    assert_eq!(asks(&mut asked).len(), 1, "and the timeline is asked for again");
    assert!(app.logs_key(Key::Named(NamedKey::Escape)));
    assert!(app.logs.is_none() && app.tabs[chat].under.is_none(), "esc leaves");
    assert_eq!(app.tabs[chat].view.len(), before + 2, "the chat is back with what landed meanwhile");
    // a failure never crashes and a late answer for a closed view is dropped
    app.on_net(NetEvent::Logs { epoch: s[0].epoch, res: Err(503) });
    // the ← back tab closes it too
    let who = app.subject_of(&m).unwrap();
    app.act(pop::Act::Logs, Some(who), None);
    assert!(app.logs.is_some());
    let first = asks(&mut asked).pop().unwrap();
    app.on_net(NetEvent::Logs { epoch: first.epoch, res: Err(503) });
    assert!(app.logs.as_ref().unwrap().failed && !app.logs.as_ref().unwrap().done);
    app.redraw();
    let p = app.bar_map.iter().position(|&i| i == chat).unwrap();
    let r = app.bar_hits[p];
    app.pointer = (r.x + 2, r.y + 2);
    app.bar_click();
    assert!(app.logs.is_none() && app.tabs[chat].under.is_none());
  }

  // the account's settings reach the window the way a flip does (look, tabs, vi, side), are saved, and
  // nothing goes back out; a flip here goes out once the socket is signed in (scratch HOME as above)
  #[test]
  #[ignore]
  fn headless_account_settings() {
    use crate::model::tabs::Kind;
    use serde_json::json;
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, Vec::new(), false);
    app.canvas.resize(640, 240);
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
    app.hs_authed = true;
    let state =
      json!({"zebra": false, "viMode": true, "tabPosition": "left", "hiddenTabs": ["pinned", "feed"], "junk": [1]});
    let (inb, _) = crate::model::sync::pluck(&state);
    app.on_net(NetEvent::UiState(inb));
    assert!(!app.set.zebra && app.set.vi && !app.set.tab_feed && app.set.tab_whispers);
    assert_eq!(app.set.tab_side, Side::Left);
    assert!(app.tabs.iter().position(|t| t.kind == Kind::Feed).is_some_and(|i| !app.visible(i)), "feed went away");
    assert!(Settings::load().vi && !Settings::load().zebra, "saved");
    app.ui_state_tick();
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.ui_state_tick();
    let out = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
      let mut n = 0;
      while let Ok(c) = asked.try_recv() {
        n += usize::from(matches!(c, Cmd::Hs(crate::net::hs_ws::Cmd::UiSync(_))));
      }
      n
    };
    assert_eq!(out(&mut asked), 0, "applying the account's state sends nothing back");
    app.flip(crate::model::schema::def("timestamps").unwrap());
    app.ui_state_tick();
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.ui_state_tick();
    assert_eq!(out(&mut asked), 1, "your flip goes out once");
  }

  // tabs on each side: the bar sits there, the chat and input take the rest, a click on a tab picks it.
  // HSC_DUMP=$d/x.ppm (the last side, left) ... cargo test -j2 --release headless_tab_sides -- --ignored
  #[test]
  #[ignore]
  fn headless_tab_sides() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(640, 240);
    app.relayout_body();
    let (forsen, xqc) = (0..app.tabs.len()).fold((0, 0), |(f, x), i| match &*app.tabs[i].name {
      "forsen" => (i, x),
      "xqc" => (f, i),
      _ => (f, x),
    });
    for n in 0..30 {
      app.on_chat("forsen", Chat::Msg(Message::system(0, Platform::Twitch, format!("line {n}"))));
    }
    for side in [Side::Bottom, Side::Right, Side::Top, Side::Left] {
      app.select(forsen);
      app.side_to(side);
      app.redraw();
      let (bar, body) = (app.bar(), app.body());
      assert_eq!(bar.w * bar.h + body.w * body.h, 640 * 240, "{side:?}: bar + body tile the window");
      assert!(app.chat_area().intersect(bar).is_empty() && app.input_area().intersect(bar).is_empty());
      assert_eq!(app.tabs[forsen].view.width, body.w, "{side:?}: the chat is as wide as the body");
      assert!(app.bar_hits.iter().all(|r| bar.intersect(*r) == *r || r.is_empty()), "{side:?}: hits in the bar");
      let p = app.bar_map.iter().position(|&i| i == xqc).unwrap();
      let r = app.bar_hits[p];
      app.pointer = (r.x + 2, r.y + 2);
      app.click();
      assert_eq!(app.active, xqc, "{side:?}: a click picks the tab");
    }
    app.select(forsen);
    app.redraw();
    crate::bench::dump(&app.canvas);
  }

  // a picture in chat: the magnifier over it, a click fills the pane, + zooms, esc goes back.
  // HSC_DUMP=$d/x.ppm (the open view) ... cargo test -j2 --release headless_img_view -- --ignored
  #[test]
  #[ignore]
  fn headless_img_view() {
    use crate::images::decode::{Decoded, Frame};
    use winit::window::CursorIcon;
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(640, 400);
    app.relayout_body();
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    let mut m = Message::system(0, Platform::Twitch, "look https://heatsync.org/uploads/cat.png");
    m.kind = crate::model::message::Kind::Chat;
    m.nick = "bob".into();
    m.login = "bob".into();
    app.on_chat("forsen", Chat::Msg(m));
    // every picture asked for lands as a 400x300 blue block
    let land = |app: &mut App, asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
      for _ in 0..3 {
        app.redraw();
        let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
          .filter_map(|c| if let Cmd::Image(id, _, target) = c { Some((id, target)) } else { None })
          .collect();
        if wanted.is_empty() {
          break;
        }
        for (id, target) in wanted {
          let rgba = [0, 0, 255, 255].repeat(400 * 300).into();
          app.res.images.loaded(id, target, Ok(Decoded { w: 400, h: 300, frames: vec![Frame { rgba, delay_ms: 0 }] }));
        }
        for t in &mut app.tabs {
          t.view.relayout();
        }
      }
    };
    land(&mut app, &mut asked);
    let find = |app: &mut App| {
      let area = app.chat_area();
      (area.y..area.bottom())
        .step_by(4)
        .find_map(|y| {
          (area.x..area.right())
            .step_by(4)
            .find(|&x| {
              let res = &mut app.res;
              app.tabs[chat].view.pic_at(area, x, y, res).is_some()
            })
            .map(|x| (x, y))
        })
        .expect("the picture is under the pointer somewhere")
    };
    let at = find(&mut app);
    app.pointer = at;
    app.cursor_sync();
    assert_eq!(app.cursor, CursorIcon::ZoomIn, "a magnifier over a picture");
    app.click();
    assert!(app.img_view.is_some(), "a click opens it");
    land(&mut app, &mut asked);
    app.bar_dirty = true;
    app.redraw();
    assert_eq!(app.input_h, 0, "no input bar under the picture");
    let blue = |app: &App| app.canvas.px.iter().filter(|&&p| p == 0x0000ff).count();
    assert!(blue(&app) >= 390 * 290, "the picture at its own size");
    app.cursor_sync();
    assert_eq!(app.cursor, CursorIcon::ZoomOut);
    let p = app.bar_map.iter().position(|&i| i == chat).unwrap();
    let r = app.bar_hits[p];
    crate::bench::dump(&app.canvas);
    app.img_key(Key::Character("+"));
    app.redraw();
    assert!(blue(&app) > 400 * 300, "+ makes it bigger");
    app.img_key(Key::Character("0"));
    app.img_key(Key::Named(NamedKey::Escape));
    assert!(app.img_view.is_none(), "esc closes it");
    // ← back closes it too
    app.redraw();
    app.pointer = find(&mut app);
    app.click();
    assert!(app.img_view.is_some());
    app.pointer = (r.x + 2, r.y + 2);
    app.click();
    assert!(app.img_view.is_none() && app.active == chat, "← back closes it, the tab stays");
  }

  // the chat look switches take effect on lines already shown, and persist.
  // cargo test -j2 --release headless_chat_look -- --ignored
  #[test]
  #[ignore]
  fn headless_chat_look() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["aaa".into()], false);
    // the scratch HOME keeps what the last run switched: start from the defaults
    app.set = crate::model::settings::Settings::default();
    app.res.look = crate::ui::res::Look::of(&app.set);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(640, 300);
    app.relayout_body();
    let pos = app.tabs.iter().position(|t| &*t.name == "aaa").unwrap();
    app.select(pos);
    let mut m = Message::system(1_700_000_000_000, Platform::Twitch, "hi");
    m.kind = crate::model::message::Kind::Chat;
    m.nick = "bob".into();
    m.login = "bob".into();
    app.on_chat("aaa", Chat::Msg(m));
    app.redraw();
    // the ext's defaults: no clock, stripes on
    assert!(!app.res.look.timestamps && app.res.look.zebra);
    app.local(Local::Toggle("timestamps"));
    app.local(Local::Pick("time_fmt", Some(1)));
    app.local(Local::Pick("emote_size", Some(2)));
    app.local(Local::Pick("line_height", Some(2)));
    app.redraw();
    assert!(app.res.look.timestamps && app.res.look.twelve && app.res.look.emote_mul == 4);
    assert!((app.res.look.spacing - 1.3).abs() < 1e-6);
    let saved = Settings::parse(&app.set.render());
    assert!(saved.timestamps && saved.time_fmt == crate::model::settings::TimeFmt::H12);
    assert_eq!(saved.emote_size.mul(), 4);
    // a pick answers to its slash command both ways
    app.local(Local::Pick("animate_emotes", Some(1)));
    assert!(!app.res.look.animate_emotes);
    app.local(Local::Pick("animate_emotes", Some(0)));
    assert!(app.res.look.animate_emotes);
  }

  // the unread number after a channel's name, and tab mode: the cursor moves without switching, K
  // carries a channel up, enter opens. HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_tab_mode -- --ignored
  #[test]
  #[ignore]
  fn headless_tab_mode() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["aaa".into(), "bbb".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(640, 300);
    app.side_to(Side::Top);
    app.relayout_body();
    let pos = |app: &App, n: &str| app.tabs.iter().position(|t| &*t.name == n).unwrap();
    app.select(pos(&app, "aaa"));
    for _ in 0..3 {
      let mut m = Message::system(0, Platform::Twitch, "hi");
      m.kind = crate::model::message::Kind::Chat;
      m.nick = "bob".into();
      m.login = "bob".into();
      app.on_chat("bbb", Chat::Msg(m));
    }
    let b = pos(&app, "bbb");
    assert_eq!(app.tabs[b].count.text(Counter::Unread, false, now_ms() / 1000), "3");
    app.bar_dirty = true;
    app.redraw();
    crate::bench::dump(&app.canvas);
    // tab mode: the cursor starts on the open tab and moves without switching
    app.leader_key(Key::Character("t"));
    assert_eq!(app.rover.as_ref().map(|r| r.at), Some(pos(&app, "aaa")));
    app.rover_key(Key::Character("j"));
    assert_eq!(app.rover.as_ref().map(|r| r.at), Some(pos(&app, "bbb")));
    assert_eq!(&*app.tabs[app.active].name, "aaa", "moving never switches");
    assert!(app.mode_line().1.contains("bbb"), "the input bar says where the cursor is");
    // K carries bbb above aaa, and the cursor follows it
    let before = pos(&app, "aaa");
    app.rover_key(Key::Character("K"));
    assert_eq!(pos(&app, "bbb"), before);
    assert_eq!(app.rover.as_ref().map(|r| r.at), Some(pos(&app, "bbb")));
    assert_eq!(&*app.tabs[app.active].name, "aaa", "the open tab stays open through the swap");
    app.rover_key(Key::Named(NamedKey::Enter));
    assert!(app.rover.is_none());
    assert_eq!(&*app.tabs[app.active].name, "bbb");
    assert_eq!(app.tabs[app.active].count.text(Counter::Unread, false, now_ms() / 1000), "", "opening reads it");
  }

  // a reddit link: the resolver is asked once, its answer draws as a card under the line, and the card
  // is a link to the post. HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_preview_card -- --ignored
  #[test]
  #[ignore]
  fn headless_preview_card() {
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(640, 400);
    app.relayout_body();
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    let link = "https://www.reddit.com/r/rust/comments/1c0rgn3/x/";
    for _ in 0..2 {
      let mut m = Message::system(0, Platform::Twitch, format!("lol {link}"));
      m.kind = crate::model::message::Kind::Chat;
      m.nick = "bob".into();
      m.login = "bob".into();
      app.on_chat("forsen", Chat::Msg(m));
    }
    app.redraw();
    let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| if let Cmd::Preview(u) = c { Some(u) } else { None })
      .collect();
    assert_eq!(wanted, vec![Box::<str>::from(link)], "asked once for two lines");
    let v = serde_json::json!({"type": "rich", "title": "a post title", "author": "r/rust"});
    app.on_net(NetEvent::Preview(link.into(), crate::model::preview::parse(link, &v)));
    app.redraw();
    crate::bench::dump(&app.canvas);
    let cyan = app.canvas.px.iter().filter(|&&p| p == 0x00ffff).count();
    assert!(cyan > 40, "the [reddit] tag is drawn ({cyan})");
    let area = app.chat_area();
    let hit = (area.y..area.bottom()).rev().step_by(2).find_map(|y| {
      let res = &mut app.res;
      let x =
        (area.x..area.right()).step_by(3).find(|&x| app.canvas.px[(y * app.canvas.w + x) as usize] == 0x00ffff)?;
      app.tabs[chat].view.link_at(area, x, y, res)
    });
    assert_eq!(hit.as_deref(), Some(link), "the card opens the post");
    // a chat-log permalink quotes the line: the speaker in their color
    let log = "https://heatsync.org/search/logs/twitch/zubatlel/2026-11-03?m=f6dd6ca7-86c2-487e-9324-d39b7109281a";
    let mut m = Message::system(0, Platform::Twitch, log);
    m.kind = crate::model::message::Kind::Chat;
    m.nick = "bob".into();
    m.login = "bob".into();
    app.on_chat("forsen", Chat::Msg(m));
    app.redraw();
    let v = serde_json::json!({"message": {"display_name": "Nightbot", "message": "quoted words",
      "timestamp": "2026-11-03T01:00:34.985Z", "color": "#00ff00"}});
    app.on_net(NetEvent::Preview(log.into(), crate::model::preview::parse(log, &v)));
    app.redraw();
    crate::bench::dump(&app.canvas);
    assert!(app.canvas.px.iter().filter(|&&p| p == 0x00ff00).count() > 40, "the quoted speaker in green");
  }

  // a youtube link: its still under the line with a ▶, a hand over it, a click plays it (never the
  // picture view). HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_video_still -- --ignored
  #[test]
  #[ignore]
  fn headless_video_still() {
    use crate::images::decode::{Decoded, Frame};
    use winit::window::CursorIcon;
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(640, 400);
    app.relayout_body();
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    let link = "https://youtu.be/dQw4w9WgXcQ";
    let mut m = Message::system(0, Platform::Twitch, format!("watch {link}"));
    m.kind = crate::model::message::Kind::Chat;
    m.nick = "bob".into();
    m.login = "bob".into();
    app.on_chat("forsen", Chat::Msg(m));
    for _ in 0..3 {
      app.redraw();
      let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
        .filter_map(|c| if let Cmd::Image(id, url, target) = c { Some((id, url, target)) } else { None })
        .collect();
      for (id, url, target) in wanted {
        assert!(url.contains("i.ytimg.com%2Fvi%2FdQw4w9WgXcQ"), "the still, through the proxy: {url}");
        let rgba = [0, 0, 255, 255].repeat(320 * 180).into();
        app.res.images.loaded(id, target, Ok(Decoded { w: 320, h: 180, frames: vec![Frame { rgba, delay_ms: 0 }] }));
      }
      for t in &mut app.tabs {
        t.view.relayout();
      }
    }
    app.redraw();
    crate::bench::dump(&app.canvas);
    let area = app.chat_area();
    let blue = app.canvas.px.iter().filter(|&&p| p == 0x0000ff).count();
    assert!(blue > 20_000, "the still is drawn ({blue})");
    let at = (area.y..area.bottom())
      .step_by(4)
      .find_map(|y| {
        (area.x..area.right())
          .step_by(4)
          .find(|&x| app.canvas.px[(y * app.canvas.w + x) as usize] == 0x0000ff)
          .map(|x| (x, y))
      })
      .expect("the still is on screen");
    app.pointer = (at.0 + 8, at.1 + 8);
    app.cursor_sync();
    assert_eq!(app.cursor, CursorIcon::Pointer, "a hand over a video, not the magnifier");
    while asked.try_recv().is_ok() {}
    app.click();
    assert!(app.img_view.is_none(), "a video never opens the picture view");
    let played: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| if let Cmd::Play(u) = c { Some(u) } else { None })
      .collect();
    assert_eq!(played, vec![Box::<str>::from(link)]);
  }

  // feed media, offline: a held post's box and show, the withheld files fetched, a picture under a reply,
  // the [img] tag's peek. HSC_DUMP=$d/x.ppm ... cargo test -j2 --release headless_feed_media -- --ignored
  #[test]
  #[ignore]
  fn headless_feed_media() {
    use crate::images::decode::{Decoded, Frame};
    use crate::model::post::parse;
    use crate::net::social;
    crate::heatsync::content::set_shown([]);
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, Vec::new(), true);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 520);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let op = parse(&serde_json::json!({"base36_id": "00004a", "username": "mellen9", "content": "porn test",
      "has_media": true, "media_count": "1", "media": [], "media_type": "image", "media_held": ["nsfw"]}))
    .unwrap();
    let re =
      parse(&serde_json::json!({"base36_id": "00004b", "username": "bob", "content": "nice", "reply_to": "00004a",
      "media_url": "https://heatsync.org/uploads/b.webp", "media_type": "image"}))
      .unwrap();
    app.feed.posts = vec![op.clone()];
    app.feed.loaded = true;
    let feed = app.tabs.iter().position(|t| t.kind == Kind::Feed).expect("a feed tab");
    app.select(feed);
    app.feed_open("00004a");
    app.on_net(NetEvent::Thread { id: "00004a".into(), res: Ok((op, vec![re])) });
    // every picture asked for lands as a flat 160x120 block, red
    let land = |app: &mut App, asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
      for _ in 0..3 {
        app.redraw();
        let wanted: Vec<_> = std::iter::from_fn(|| asked.try_recv().ok())
          .filter_map(|c| if let Cmd::Image(id, _, target) = c { Some((id, target)) } else { None })
          .collect();
        if wanted.is_empty() {
          break;
        }
        for (id, target) in wanted {
          let d = Decoded {
            w: 160,
            h: 120,
            frames: vec![Frame { rgba: [255, 0, 0, 255].repeat(160 * 120).into(), delay_ms: 0 }],
          };
          app.res.images.loaded(id, target, Ok(d));
        }
        for t in &mut app.tabs {
          t.view.relayout();
        }
      }
    };
    land(&mut app, &mut asked);
    let spot = |app: &mut App, f: fn(&mut ChatView, Rect, i32, i32, &mut Res) -> Option<Box<str>>| {
      let area = app.chat_area();
      (area.y..area.bottom()).step_by(2).find_map(|y| {
        (area.x..area.right()).step_by(2).find_map(|x| {
          let res = &mut app.res;
          f(&mut app.tabs[feed].view, area, x, y, res).map(|id| ((x, y), id))
        })
      })
    };
    let red = |app: &App| app.canvas.px.iter().filter(|&&p| p == 0xff0000).count();
    assert!(red(&app) >= 150 * 110, "the reply's picture draws under its words");
    let cyan = app.canvas.px.iter().filter(|&&p| p == 0x00ffff).count();
    assert!(cyan > 50, "the held box wears its dashed cyan");
    let (at, id) = spot(&mut app, ChatView::reveal_at).expect("the held box has a show button");
    assert_eq!(&*id, "00004a");
    while asked.try_recv().is_ok() {}
    app.pointer = at;
    app.click();
    let asked_media = std::iter::from_fn(|| asked.try_recv().ok())
      .any(|c| matches!(c, Cmd::Social(social::Cmd::Media { ref id }) if &**id == "00004a"));
    assert!(asked_media, "show asks for the withheld files");
    let files =
      crate::model::post::media_of(&serde_json::json!({"media_url": "/uploads/a.webp", "media_type": "image"}));
    app.on_net(NetEvent::Media { id: "00004a".into(), res: Ok(files) });
    land(&mut app, &mut asked);
    assert!(red(&app) >= 2 * 150 * 110, "shown: the op's picture too");
    let (at, _) = spot(&mut app, ChatView::reveal_at).expect("a hide over the shown picture");
    app.pointer = at;
    app.click();
    land(&mut app, &mut asked);
    assert!(red(&app) < 2 * 150 * 110, "hide folds it back into the box");
    // the op's [img] tag, held again: its peek is the box's words
    let (at, id) = spot(&mut app, ChatView::media_at).expect("the op's [img] tag");
    assert_eq!(&*id, "00004a");
    app.pointer = at;
    app.tip_hover();
    assert!(app.media_peek.is_some(), "hovering the tag opens the peek");
    land(&mut app, &mut asked);
    crate::bench::dump(&app.canvas);
  }

  // the hover popup and colored mentions, offline: needs a scratch HOME (App::new reads settings):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d HSC_DUMP=$d/x.ppm cargo test -j2 --release headless_hover -- --ignored
  #[test]
  #[ignore]
  fn headless_hover_and_mentions() {
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, Vec::new(), true);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let line = |login: &str, color: u32, body: &str, id: &str| {
      let mut m = Message::system(now_ms(), Platform::Twitch, "x");
      m.kind = crate::model::message::Kind::Chat;
      m.id = id.into();
      m.login = login.into();
      m.nick = login.into();
      m.color = color;
      m.body = body.into();
      Chat::Msg(m)
    };
    let tab: &str = FLOOD_TAB;
    // the flood tab is synthetic: nobody in it counts as heard unless the probe says so
    for (who, color) in [("alice", 0x00ff00), ("carol", 0xff00ff), ("bob", 0xffff00)] {
      if let Chat::Msg(m) = line(who, color, "", "") {
        app.res.chatters.heard(&m);
      }
    }
    app.on_net(NetEvent::Chat(tab.into(), line("alice", 0x00ff00, "morning chat", "1")));
    app.on_net(NetEvent::Chat(tab.into(), line("carol", 0xff00ff, "hey", "2")));
    app.on_net(NetEvent::Chat(
      tab.into(),
      line("bob", 0xffff00, "alice you are late, ask @carol or @dave what what", "3"),
    ));
    app.redraw();
    // an unknown @dave is looked up once, in the frame after the line landed
    let mut cards = Vec::new();
    while let Ok(c) = asked.try_recv() {
      if let Cmd::Social(crate::net::social::Cmd::Card { login, .. }) = c {
        cards.push(login);
      }
    }
    assert_eq!(cards, vec!["dave".into()], "only the stranger is asked, once");
    let ink = |c: &Canvas, rgb: u32| c.px.iter().filter(|&&p| p == rgb).count();
    assert!(
      ink(&app.canvas, 0x00ff00) > 30 && ink(&app.canvas, 0xff00ff) > 30,
      "alice and carol are drawn in their colors"
    );
    // find alice inside bob's line and rest the pointer on her
    // a real room, so a card can say where it was opened from
    let active = app.active;
    app.tabs[active].sources.push((Platform::Twitch, "lobby".into()));
    let area = app.chat_area();
    let spot = (area.x..area.right()).step_by(2).find_map(|x| {
      (area.y..area.bottom()).find_map(|y| {
        let res = &mut app.res;
        let h = app.tabs[app.active].view.who_at(area, x, y, res)?;
        (h.mention.as_ref().is_some_and(|n| &*n.login == "alice")).then_some((x, y))
      })
    });
    app.pointer = spot.expect("alice's name in bob's line answers the pointer");
    app.tip_hover();
    assert!(app.peek.is_some() && app.peek_due.is_some(), "a cold name shows at once and asks after resting");
    app.peek_fire();
    assert!(matches!(asked.try_recv(), Ok(Cmd::Social(crate::net::social::Cmd::Card { .. }))));
    // heatsync answers
    let info = crate::model::card::Info {
      registered: true,
      rows: vec![
        ("age", "9y 2mo".into()),
        ("heat", "1.2k".into()),
        ("followers", "15k".into()),
        ("ch follow", "2y".into()),
      ],
      color: None,
      twitch_name: None,
      avatar: Some("https://static-cdn.jtvnw.net/jtv_user_pictures/a-70x70.png".into()),
      twitch_id: None,
      ..Default::default()
    };
    app.on_net(NetEvent::Card { platform: Platform::Twitch, login: "alice".into(), res: Ok(info) });
    app.redraw();
    assert!(app.peek.is_some());
    // her picture is asked for and drawn beside the name
    let (id, target) = std::iter::from_fn(|| asked.try_recv().ok())
      .find_map(|c| if let Cmd::Image(id, url, t) = c { url.contains("a-70x70").then_some((id, t)) } else { None })
      .expect("the popup asks for her picture");
    let d = crate::images::decode::Decoded {
      w: 40,
      h: 40,
      frames: vec![crate::images::decode::Frame { rgba: [0, 255, 0, 255].repeat(40 * 40).into(), delay_ms: 0 }],
    };
    app.res.images.loaded(id, target, Ok(d));
    app.redraw();
    assert!(app.canvas.px.iter().filter(|&&p| p == 0x00ff00).count() >= 30 * 30, "the picture shows");
    // the popup is up: more than the bare line shows, a warm name needs no second ask
    app.hide_tip();
    app.tip_hover();
    assert!(app.peek.is_some() && app.peek_due.is_none(), "an answered name opens at once");
    app.redraw();
    crate::bench::dump(&app.canvas);
    // a click on her name in bob's line opens her card from the cached answer: nothing more is asked
    app.hide_tip();
    assert!(app.name_press(), "the mention answers a click");
    assert!(app.card.is_some() && asked.try_recv().is_err());
  }

  // feed engagement offline: upvote and bookmark show at once and roll back on a refusal, your own post
  // edits in the box and deletes only after a yes, /op posts, logged out shows no post rows (scratch HOME as above):
  //   HOME=$d XDG_CONFIG_HOME=$d XDG_CACHE_HOME=$d cargo test -j2 --release headless_feed_engage -- --ignored
  #[test]
  #[ignore]
  fn headless_feed_engage() {
    use crate::app::pop::Act;
    use crate::model::post::parse;
    use crate::net::social::{self, Page};
    let (net, mut asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.set = Settings::default();
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let now = crate::util::time::now_ms();
    let post = |id: &str, user: &str, at: i64| {
      let mut p =
        parse(&serde_json::json!({"base36_id": id, "username": user, "user_id": "9", "content": "hello feed"}))
          .unwrap();
      p.ts = at;
      p
    };
    let (theirs, mine, old) = (
      post("00004a", "bob", now - 5000),
      post("00004b", "mellen", now - 60_000),
      post("00004c", "mellen", now - 3_600_000),
    );
    let cmds = |asked: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| {
      std::iter::from_fn(|| asked.try_recv().ok()).collect::<Vec<_>>()
    };
    let feed = app.tabs.iter().position(|t| t.kind == Kind::Feed).expect("a feed tab");
    app.select(feed);
    let load = |app: &mut App| {
      let posts = vec![theirs.clone(), mine.clone(), old.clone()];
      app.on_net(NetEvent::Feed(Page { page: 1, posts, hot: false, cold: false, more: false }));
      app.redraw();
    };
    let pick = |app: &mut App, id: &str| {
      let h = app.chat_area().h;
      let res = &mut app.res;
      assert!(app.tabs[feed].view.select_id(id, res, h), "{id} is on screen");
    };
    let rows = |app: &mut App, id: &str| {
      pick(app, id);
      app.line_menu_key();
      app.pop.as_ref().map(|p| p.menu.items.iter().map(|r| r.1.clone()).collect::<Vec<_>>()).unwrap_or_default()
    };
    // logged out: the menu opens (copies) but has no post rows, and nothing is asked about bookmarks
    load(&mut app);
    assert!(
      !cmds(&mut asked).iter().any(|c| matches!(c, Cmd::Social(social::Cmd::BookmarkCheck { .. }))),
      "no check logged out"
    );
    let r = rows(&mut app, "00004a");
    assert!(
      r.contains(&"copy message".to_owned()) && !r.iter().any(|l| l.contains("upvote") || l.contains("bookmark"))
    );
    app.pop = None;
    // log in as mellen
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
    app.feed = Feed::default();
    load(&mut app);
    let checks: Vec<_> = cmds(&mut asked)
      .into_iter()
      .filter_map(|c| if let Cmd::Social(social::Cmd::BookmarkCheck { ids }) = c { Some(ids) } else { None })
      .collect();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].len(), 3, "one check for the page");
    app.on_net(NetEvent::Marks { ids: checks[0].clone(), res: Ok(vec![("00004a".into(), true)]) });
    // someone else's post: upvote + bookmark (already saved: remove)
    let r = rows(&mut app, "00004a");
    assert!(r.contains(&"upvote".to_owned()) && r.contains(&"remove bookmark".to_owned()), "{r:?}");
    assert!(!r.iter().any(|l| l.starts_with("edit") || l == "delete"));
    app.pop = None;
    // upvote: shows at once, a refusal puts it back
    pick(&mut app, "00004a");
    app.act(Act::Upvote, None, None);
    assert!(app.feed.find("00004a").unwrap().voted, "the ▲ lights at once");
    assert!(cmds(&mut asked).iter().any(|c| matches!(c, Cmd::Social(social::Cmd::Vote { id }) if &**id == "00004a")));
    app.on_net(NetEvent::Voted { id: "00004a".into(), res: Err(500) });
    assert!(!app.feed.find("00004a").unwrap().voted, "rolled back");
    app.act(Act::Upvote, None, None);
    app.on_net(NetEvent::Voted { id: "00004a".into(), res: Ok((true, Some(7))) });
    let p = app.feed.find("00004a").unwrap();
    assert!(p.voted && p.heat == 7);
    assert!(rows(&mut app, "00004a").contains(&"remove upvote".to_owned()));
    app.pop = None;
    // bookmark off: the request says off, a refusal keeps it
    pick(&mut app, "00004a");
    app.act(Act::Bookmark, None, None);
    assert!(!app.feed.marks.get("00004a"));
    assert!(
      cmds(&mut asked)
        .iter()
        .any(|c| matches!(c, Cmd::Social(social::Cmd::Bookmark { id, on: false }) if &**id == "00004a"))
    );
    app.on_net(NetEvent::Bookmarked { id: "00004a".into(), on: false, res: Err(500) });
    assert!(app.feed.marks.get("00004a"), "rolled back");
    // your own post: no upvote, edit with a clock, delete
    let r = rows(&mut app, "00004b");
    assert!(!r.iter().any(|l| l.contains("upvote")));
    assert!(r.iter().any(|l| l.starts_with("edit (")) && r.last().map(String::as_str) == Some("delete"), "{r:?}");
    app.pop = None;
    let r = rows(&mut app, "00004c");
    assert!(r.contains(&"edit (expired)".to_owned()));
    app.pop = None;
    // edit: the words come into the box, the draft waits, enter saves, a refusal keeps the words
    pick(&mut app, "00004c");
    app.act(Act::Edit, None, None);
    assert!(app.edit.is_none() && app.line.text.is_empty(), "past the window: no edit mode");
    app.line.insert("my draft");
    pick(&mut app, "00004b");
    app.act(Act::Edit, None, None);
    assert!(app.edit.is_some());
    assert_eq!(app.line.text, "hello feed");
    app.line.insert(" fixed");
    app.send();
    let sent = cmds(&mut asked);
    assert!(sent.iter().any(
      |c| matches!(c, Cmd::Social(social::Cmd::Edit { id, text }) if &**id == "00004b" && text == "hello feed fixed")
    ));
    app.on_net(NetEvent::Edited { id: "00004b".into(), res: Err(403) });
    assert!(app.edit.is_some() && app.line.text == "hello feed fixed", "a refused edit keeps the words");
    app.send();
    app.on_net(NetEvent::Edited { id: "00004b".into(), res: Ok("hello feed fixed".into()) });
    assert!(app.edit.is_none());
    assert_eq!(app.line.text, "my draft", "the draft is back");
    let p = app.feed.find("00004b").unwrap();
    assert!(p.edited && &*p.text == "hello feed fixed");
    // esc drops an edit and gives the draft back
    pick(&mut app, "00004b");
    app.act(Act::Edit, None, None);
    app.key_insert(Key::Named(NamedKey::Escape), None);
    assert!(app.edit.is_none() && app.line.text == "my draft");
    while asked.try_recv().is_ok() {}
    // delete asks first: cancel sends nothing, yes sends it, the post leaves on the answer
    pick(&mut app, "00004b");
    app.act(Act::Delete, None, None);
    let p = app.pop.as_ref().expect("delete asks");
    assert_eq!(p.menu.items.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["yes, delete", "cancel"]);
    assert!(cmds(&mut asked).is_empty(), "nothing sent before the yes");
    app.pop.as_mut().unwrap().menu.at = 1;
    app.pop_key(Key::Named(NamedKey::Enter));
    assert!(cmds(&mut asked).is_empty() && app.feed.find("00004b").is_some(), "cancel keeps it");
    app.act(Act::Delete, None, None);
    app.pop_key(Key::Named(NamedKey::Enter));
    assert!(cmds(&mut asked).iter().any(|c| matches!(c, Cmd::Social(social::Cmd::Delete { id }) if &**id == "00004b")));
    app.on_net(NetEvent::Deleted { id: "00004b".into(), res: Err(500) });
    assert!(app.feed.find("00004b").is_some(), "a refused delete keeps it");
    app.on_net(NetEvent::Deleted { id: "00004b".into(), res: Ok(()) });
    assert!(app.feed.find("00004b").is_none());
    // /op posts a thread from any tab; /re answers the last [OP] heard in chat
    app.line.take();
    app.line.insert("/op hi there");
    app.send();
    assert!(
      cmds(&mut asked)
        .iter()
        .any(|c| matches!(c, Cmd::Social(social::Cmd::Post { text, reply_to: None }) if text == "hi there"))
    );
    app.line.insert("/re yes");
    app.send();
    assert!(cmds(&mut asked).is_empty(), "no [OP] in chat yet: nothing sent");
    assert_eq!(app.line.text, "", "the say line took it");
    app.on_feed_live(theirs.clone());
    app.set.inline_op = true;
    app.on_feed_live(post("00004d", "bob", now));
    app.line.insert("/re yes");
    app.send();
    assert!(cmds(&mut asked).iter().any(
      |c| matches!(c, Cmd::Social(social::Cmd::Post { text, reply_to: Some(r) }) if text == "yes" && &**r == "00004d")
    ));
    // a failed /op comes back into the box with its command
    app.on_net(NetEvent::Posted { res: Err(500) });
    assert_eq!(app.line.text, "/re yes");
    // op to feed on a chat line: the box gets /op @name: words
    let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(chat);
    app.line.take();
    let mut m = Message::system(now_ms(), Platform::Twitch, "x");
    m.kind = crate::model::message::Kind::Chat;
    m.id = "abc".into();
    m.nick = "Carol".into();
    m.login = "carol".into();
    m.body = "this is   news".into();
    app.on_chat("forsen", Chat::Msg(m));
    app.redraw();
    app.tabs[app.active].view.select_start(&mut app.res, 300);
    app.open_line_menu((10, 10));
    let labels: Vec<_> = app.pop.as_ref().unwrap().menu.items.iter().map(|r| r.1.clone()).collect();
    assert!(labels.contains(&"op to feed".to_owned()), "{labels:?}");
    app.act(Act::OpToFeed, None, None);
    assert_eq!(app.line.text, "/op @Carol: this is news");
  }
}
