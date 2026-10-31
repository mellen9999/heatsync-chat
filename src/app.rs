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

mod builtin;
mod card;
mod clip;
mod emote;
mod feed;
mod filters;
mod find;
mod link;
mod mentions;
mod mods;
mod picker;
mod pin;
mod pop;
mod screen;
mod stream;
mod tabs;
mod tip;
mod upload;
mod whispers;
mod zoom;

use crate::cosmetics::id::PaintId;
use crate::fake::Fake;
use crate::model::chan::{self, Form};
use crate::model::complete::{Complete, rank};
use crate::model::feed::Feed;
use crate::model::ime::Ime;
use crate::model::line::Line;
use crate::model::mention::Mentions;
use crate::model::message::{Message, Platform, flag};
use crate::model::picker::{Focus, Picker, Recents};
use crate::model::pin::Pins;
use crate::model::settings::{Local, Settings, commands, local_cmd};
use crate::model::stream_events::Moments;
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
  // every feed merged into this tab: (platform, routing key)
  sources: Vec<(Platform, Box<str>)>,
  // which source the input sends to (ctrl-n cycles on merged tabs)
  target: usize,
}

pub struct App {
  gfx: Option<Gfx>,
  canvas: Canvas,
  res: Res,
  tabs: Vec<Tab>,
  active: usize,
  net: Net,
  bar_dirty: bool,
  status: String,
  mods: ModifiersState,
  // first key of a two-key vim chord (g)
  chord: Option<char>,
  fake: Fake,
  flood: Option<Instant>,
  prof: bool,
  // twitch room id per channel, once known (roomstate repeats on every mode change)
  rooms: Vec<(Box<str>, Box<str>)>,
  // twitch channels where our badges say we moderate (or own) the room
  mod_in: std::collections::HashSet<Box<str>>,
  // when the soonest timed mute ends
  mute_due: Option<Instant>,
  next_anim: Option<Instant>,
  // the emote hover popup, and when its animated emote flips next
  tip: Option<crate::ui::tip::Tip>,
  tip_next: Option<Instant>,
  occluded: bool,
  // damage of recent presents, for buffers handed back with age > 1
  damage_hist: [Option<Rect>; 2],
  // pending heap trim after bursts of parsing (emote sets, history)
  trim_at: Option<Instant>,
  // the message being typed. mode: with vi keys off it is always Insert (a plain text box);
  // with them on, Chat drives the chat, Insert types, Normal is vim on the box
  line: Line,
  mode: Mode,
  vi: Vi,
  set: Settings,
  complete: Option<Complete>,
  // replying to: (platform message id, @nick), sent with the next line
  reply: Option<(Box<str>, Box<str>)>,
  // last pointer position (window px), for clicks
  pointer: (i32, i32),
  input_h: i32,
  input_dirty: bool,
  me: Option<Me>,
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
  link_hand: bool,
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
  // display scale factor (zoom multiplies it), and smooth-wheel travel toward a zoom step
  dpi: f32,
  zoom_acc: f32,
  picker: Option<Picker>,
  recents: Recents,
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
  // names that ping you, and whether the window has focus (notifications only fire when it does not)
  mentions: Mentions,
  focused: bool,
  // the whispers timeline, and whether this login's history was asked for
  whispers: Timeline,
  whispers_loaded: bool,
  // bumped on every login change: a history read from an earlier one is dropped
  whisper_epoch: u32,
  // the feed tab's posts and open thread; the text of a post in flight (put back if it fails)
  feed: Feed,
  pending_post: Option<String>,
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
        sources: Vec::new(),
        target: 0,
      });
    }
    let status =
      if tabs.is_empty() { "no channels — click + or press ctrl-t to add one".into() } else { String::new() };
    let set = Settings::load();
    crate::model::filter::edit(|f| {
      *f = crate::model::filter::Filters::load();
      (f.bots, f.cmds, f.dupes) = (set.hide_bots, set.hide_cmds, set.hide_dupes);
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
    res.arts.set_motion(set.paint_motion);
    if flood {
      crate::fake::seed_paints(&mut res.cosmetics);
    }
    res.media = set.media;
    res.partial_links = set.partial_links;
    Self {
      gfx: None,
      canvas: Canvas::new(0, 0),
      res,
      tabs,
      active,
      net,
      bar_dirty: true,
      status,
      mods: ModifiersState::empty(),
      chord: None,
      fake: Fake::new(0x5eed),
      flood: flood.then(Instant::now),
      prof: std::env::var_os("HSC_PROF").is_some(),
      rooms: Vec::new(),
      mod_in: Default::default(),
      mute_due,
      next_anim: None,
      tip: None,
      tip_next: None,
      occluded: false,
      damage_hist: [None; 2],
      trim_at: None,
      line: Line::default(),
      mode: if set.vi { Mode::Chat } else { Mode::Insert },
      vi: Vi::default(),
      set,
      complete: None,
      reply: None,
      pointer: (0, 0),
      input_h: 0,
      input_dirty: true,
      me: None,
      ime: Ime::default(),
      ime_on: false,
      ime_area: None,
      ime_sent: None,
      menu: None,
      form: None,
      form_hover: None,
      link_hand: false,
      tab_menu: None,
      pop: None,
      card: None,
      screen: None,
      bar_hits: Vec::new(),
      bar_map: Vec::new(),
      plus_hover: false,
      gear_hover: false,
      dpi: 1.0,
      zoom_acc: 0.0,
      picker: None,
      recents: Recents::load(),
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
      mentions: Mentions::default(),
      focused: true,
      whispers: Timeline::default(),
      whispers_loaded: false,
      whisper_epoch: 0,
      feed: Feed::default(),
      pending_post: None,
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

  fn bar(&self) -> Rect {
    Rect::new(0, 0, self.canvas.w, self.res.text.line_px as i32)
  }

  fn chat_area(&self) -> Rect {
    let b = self.bar().h;
    let top = b + self.pin_h;
    Rect::new(0, top, self.canvas.w, (self.canvas.h - top - self.input_h).max(0))
  }

  fn input_area(&self) -> Rect {
    Rect::new(0, (self.canvas.h - self.input_h).max(0), self.canvas.w, self.input_h)
  }

  fn hint(&self) -> String {
    if self.kind() == Kind::Mentions {
      return "enter or a click opens the line in its channel".into();
    }
    if self.kind() == Kind::Feed {
      return self.feed_hint();
    }
    if self.kind() == Kind::Whispers {
      return match self.whisper_label() {
        Some(to) => format!("type to whisper {to}· /w user msg"),
        None => "/w user msg to whisper someone".into(),
      };
    }
    let lead = if self.set.vi { "i type" } else { "type to chat" };
    match &self.me {
      None => format!("ctrl-l log in · {lead}"),
      Some(me) => format!("{lead} as {}", me.username),
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
      Local::Tab(Some(k), shown) => {
        self.set_shown(k, shown);
        self.say(format!("{} tab {}", k.name(), if shown { "shown" } else { "hidden" }));
      }
      Local::Tab(None, shown) => {
        self.say(format!("usage: /{} feed, whispers or mentions", if shown { "show" } else { "hide" }))
      }
      Local::Zoom(Some(pct)) => self.zoom_to(pct),
      Local::Zoom(None) => self.say("usage: /zoom 1.5 (anything from 0.5 to 3)"),
      Local::Settings => self.open_screen(screen::Page::Settings),
      Local::Keys => self.open_screen(screen::Page::Keys),
    }
  }

  fn send(&mut self) {
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
    if let Some(cmd) = crate::model::modcmd::command(&self.line.text) {
      self.line.take();
      self.mod_slash(cmd);
      return self.after_send();
    }
    if self.kind() == Kind::Feed && !self.line.text.trim().is_empty() {
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
      let reply = self.reply.take().map(|(id, nick)| Reply { id, nick });
      if let Some(v) = self.view() {
        v.deselect();
      }
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
    self.wake();
  }

  fn select(&mut self, i: usize) {
    if i < self.tabs.len() && i != self.active {
      self.find_clear();
      self.hide_tip();
      self.active = i;
      // a card or menu belongs to the tab it was opened in
      self.card = None;
      self.pop = None;
      self.tabs[i].unread = false;
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
        t.view.relayout();
      }
    }
    self.wake();
  }

  fn touch_sender(&mut self, id: u64) {
    let uid = id.to_string();
    for t in &mut self.tabs {
      t.view.touch_sender(&uid);
    }
    self.wake();
  }

  // a paint or badge landed: redo only the lines whose name it dresses
  fn touch_paint(&mut self, id: &PaintId) {
    for t in &mut self.tabs {
      t.view.touch_paint(id);
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
      self.scan_mention(p, key, m);
    }
    for i in self.tabs_with(p, key) {
      let v = &mut self.tabs[i].view;
      if history {
        v.prepend(msgs.clone());
        if i == self.active {
          self.wake();
        }
      } else {
        for m in &msgs {
          v.push(m.clone());
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
    t.view.tags = t.sources.len() > 1;
    t.view.relayout();
    self.input_dirty = true;
    if i == self.active {
      self.wake();
    }
  }

  fn on_chat(&mut self, ch: &str, ev: Chat) {
    let ev = match ev {
      Chat::Msg(mut m) if ch != FLOOD_TAB => {
        self.scan_mention(Platform::Twitch, ch, &mut m);
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
    let tab = &mut self.tabs[i];
    match ev {
      Chat::Msg(m) => tab.view.push(m),
      Chat::ClearMsg(id) => tab.view.clear_msg(&id),
      Chat::ClearUser { login, secs } => {
        tab.view.clear_user(&login);
        let what = secs.map_or_else(|| "banned".to_owned(), |s| format!("timed out for {s}s"));
        tab.view.push(Message::system(now_ms(), Platform::Twitch, format!("{login} was {what}")));
      }
      Chat::ClearAll => tab.view.push(Message::system(now_ms(), Platform::Twitch, "chat was cleared by a moderator")),
      Chat::Room { id } => {
        if !self.rooms.iter().any(|r| *r.0 == *ch) {
          self.rooms.push((ch.into(), id.clone()));
          self.net.send(Cmd::Room(ch.into(), id));
        }
        return;
      }
      Chat::Mod(on) => return self.on_mod_state(ch, on),
    }
    if i == self.active {
      self.wake();
    } else if !tab.unread {
      tab.unread = true;
      self.bar_dirty = true;
      self.wake();
    }
  }

  fn redraw(&mut self) {
    let mut dirty: Option<Rect> = None;
    if std::mem::take(&mut self.input_dirty) {
      let show_hint = self.line.text.is_empty() && (!self.set.vi || self.mode == Mode::Chat);
      let hint = match &self.prompt {
        Some(_) if self.finding => "type to filter · @name for one person · enter keeps it · esc clears".to_owned(),
        Some(_) => "type or drop a file path · tab completes · enter uploads · esc cancels".to_owned(),
        None if show_hint => self.hint(),
        None => String::new(),
      };
      let hidden = self.hidden();
      let tag = match &self.prompt {
        Some(_) if self.finding => Some(("find: ", crate::ui::theme::YELLOW)),
        Some(_) => Some(("path: ", crate::ui::theme::YELLOW)),
        None => self.target().and_then(|t| t.2),
      };
      let reply = self.reply.as_ref().filter(|_| self.prompt.is_none()).map(|r| &*r.1);
      let bar = input_bar::Bar {
        line: self.prompt.as_ref().unwrap_or(&self.line),
        mode: if self.prompt.is_some() { Mode::Insert } else { self.mode },
        target: tag,
        hint: &hint,
        reply,
        preedit: self.ime.preedit(),
        preedit_caret: self.ime.caret(),
        picker_on: self.picker.is_some() || self.btn_hover,
        upload_on: self.uploading || (self.prompt.is_some() && !self.finding) || self.up_hover,
        logo: &self.res.logo,
      };
      let h = if hidden { 0 } else { input_bar::height(&mut self.res.text, &bar, self.canvas.w) };
      if h != self.input_h {
        // the chat area changed size: repaint it whole
        self.input_h = h;
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.invalidate();
        }
      }
      if h > 0 {
        let r = self.input_area();
        self.ime_area = Some(input_bar::draw(&mut self.canvas, &mut self.res.text, r, &bar));
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
      || self.tip.is_some())
      && let Some(t) = self.tabs.get_mut(self.active)
    {
      // the menu floats over the chat: repaint under it so blits and scrolls leave no ghosts
      t.view.invalidate();
    }
    if std::mem::take(&mut self.bar_dirty) {
      let shown: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.visible(i)).collect();
      let labels: Vec<TabLabel> = shown
        .iter()
        .map(|&i| {
          let t = &self.tabs[i];
          let hot = match t.kind {
            Kind::Mentions => crate::ui::theme::RED,
            Kind::Whispers => crate::ui::theme::CYAN,
            _ => crate::ui::theme::WHITE,
          };
          TabLabel { name: &t.name, active: i == self.active, unread: t.unread, hot }
        })
        .collect();
      let bar = self.bar();
      self.bar_hits = tab_bar::draw(
        &mut self.canvas,
        &mut self.res.text,
        bar,
        &labels,
        &self.status,
        (self.plus_hover, self.gear_hover || self.screen.is_some()),
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
    if let Some(r) = self.draw_tip() {
      dirty = Some(dirty.map_or(r, |d| union(d, r)));
    }
    if let Some(t) = self.tip_next {
      self.next_anim = Some(self.next_anim.map_or(t, |n| n.min(t)));
    }
    for (id, url, h) in self.res.images.take_wanted() {
      self.net.send(Cmd::Image(id, url, h));
    }
    for id in self.res.senders.take_wanted() {
      self.net.send(Cmd::Sender(id, None));
    }
    for id in self.res.cosmetics.take_wanted() {
      self.net.send(Cmd::Cosmetics(id));
    }
    self.present(dirty);
  }

  fn present(&mut self, dirty: Option<Rect>) {
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
    self.canvas.resize(w as i32, h as i32);
    self.damage_hist = [None; 2];
    self.input_dirty = true;
    for t in &mut self.tabs {
      t.view.resize(w as i32);
    }
    self.bar_dirty = true;
    self.wake();
  }

  // tab / shift-tab: first press finds candidates for the word before the cursor, then cycles them
  fn tab_complete(&mut self, back: bool) {
    if self.complete.is_none() {
      let (start, word) = self.line.current_word();
      let end = start + word.len();
      let tab = self.tabs.get(self.active);
      let chan = tab.map_or("", |t| &*t.view.channel);
      let matches = if let Some(nick) = word.strip_prefix('@') {
        let names = tab.map_or_else(Vec::new, |t| t.view.chatters());
        rank(nick, [names]).into_iter().map(|n| format!("@{n}").into()).collect()
      } else if start == 0 && word.starts_with('/') {
        rank(
          word,
          [
            commands(),
            chan::COMMANDS.to_vec(),
            crate::model::modcmd::COMMANDS.to_vec(),
            crate::model::filtercmd::COMMANDS.to_vec(),
          ],
        )
      } else {
        let own =
          self.me.as_ref().and_then(|m| m.twitch_id).map_or_else(Vec::new, |id| self.res.senders.names(id, now_ms()));
        rank(word, [self.res.emotes.channel_names(chan), own, self.res.emotes.global_names()])
      };
      if matches.is_empty() {
        return;
      }
      self.complete = Some(Complete { start, end, matches, at: usize::MAX });
    }
    let Some(c) = &mut self.complete else { return };
    let n = c.matches.len();
    c.at = match (c.at, back) {
      (usize::MAX, false) => 0,
      (usize::MAX, true) => n - 1,
      (i, false) => (i + 1) % n,
      (i, true) => (i + n - 1) % n,
    };
    let pick = c.matches[c.at].clone();
    self.line.replace(c.start..c.end, &pick);
    c.end = c.start + pick.len();
    self.input_dirty = true;
    self.wake();
  }

  fn key_insert(&mut self, key: Key<&str>, text: Option<&str>) {
    let ctrl = self.mods.control_key();
    if self.global_key(key.clone()) {
      return;
    }
    if key == Key::Named(NamedKey::Tab) {
      return self.tab_complete(self.mods.shift_key());
    }
    // any other key commits the current candidate; an emote pick joins the recents
    if let Some(name) = self.complete.take().and_then(|c| c.emote().map(str::to_owned)) {
      self.recents.record(&name);
    }
    let l = &mut self.line;
    match key {
      Key::Named(NamedKey::Escape) if self.set.vi => {
        let empty = l.text.is_empty();
        if empty {
          self.reply = None;
        }
        self.vi.end_insert(l);
        return self.set_mode(if empty { Mode::Chat } else { Mode::Normal });
      }
      Key::Named(NamedKey::Escape) => {
        self.reply = None;
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
      Key::Character("n") if ctrl => {
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.target = (t.target + 1) % t.sources.len().max(1);
        }
      }
      Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !self.mods.alt_key() => {
        // mid-composition the input method owns the text; its commit arrives as an ime event
        if let Some(t) = text.filter(|_| self.ime.accepts_key_text()) {
          l.insert(t);
        }
      }
      _ => return,
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

  // vim normal mode on the message box (vi keys on)
  fn key_normal(&mut self, key: Key<&str>, text: Option<&str>) {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    if self.global_key(key.clone()) {
      return;
    }
    // an empty box has nothing to navigate: a printable key starts the message
    if !ctrl
      && !alt
      && self.line.text.is_empty()
      && self.vi.idle()
      && let (Key::Character(_) | Key::Named(NamedKey::Space), Some(t)) = (key.clone(), text)
    {
      self.enter_insert();
      self.line.insert(t);
      self.input_dirty = true;
      return self.wake();
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
      }
      Action::HistoryNewer => {
        self.line.newer();
        Vi::settle(&mut self.line);
      }
      Action::Leave => {
        self.reply = None;
        self.mode = Mode::Chat;
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
      return;
    }
    let (id, nick, p) = (m.id.clone(), m.nick.clone(), m.platform);
    if let Some(i) = t.sources.iter().position(|s| s.0 == p) {
      t.target = i;
    }
    self.reply = Some((id, nick));
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
      return self.enter_insert();
    }
    if !chat.contains(x, y) {
      return;
    }
    if self.vote_press() || self.link_press() || self.emote_press(false) || self.name_press() {
      return;
    }
    let res = &mut self.res;
    let picked = self.tabs.get_mut(self.active).is_some_and(|t| t.view.select_at(chat, y, res));
    if picked && self.tabs[self.active].kind == Kind::Mentions {
      self.jump_to_mention();
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

  fn key(&mut self, key: Key<&str>) {
    let line = self.res.text.line_px as i32;
    let half = self.chat_area().h / 2;
    let ctrl = self.mods.control_key();
    let alt = self.mods.alt_key();
    let chord = self.chord.take();
    if self.global_key(key.clone()) {
      return;
    }
    // a selected line: j/k move it, r replies, esc lets go
    if self.selecting() {
      match key {
        Key::Character("j") | Key::Named(NamedKey::ArrowDown) => return self.select_move(1),
        Key::Character("k") | Key::Named(NamedKey::ArrowUp) => return self.select_move(-1),
        Key::Named(NamedKey::Enter) if self.kind() == Kind::Mentions => return self.jump_to_mention(),
        Key::Named(NamedKey::Enter) if self.kind() == Kind::Feed => return self.feed_activate(),
        Key::Character("u") if self.kind() == Kind::Feed => return self.feed_vote(),
        Key::Character("r") if self.kind() == Kind::Feed => return self.feed_reply(),
        Key::Character("h") | Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Backspace) if self.feed_back() => {
          return;
        }
        Key::Named(NamedKey::Enter) if self.kind() == Kind::Whispers => return self.whisper_activate(),
        Key::Character("r") if self.kind() == Kind::Whispers => return self.whisper_reply_selected(),
        Key::Character("r") => return self.reply_selected(),
        Key::Character("m") if !ctrl => return self.line_menu_key(),
        Key::Character("c") if !ctrl => return self.card_selected(),
        Key::Character("o") if !ctrl && !alt => return self.open_selected_link(),
        Key::Character("c") if ctrl => return self.copy_selected(),
        Key::Named(NamedKey::Escape) | Key::Character("v") => {
          if let Some(v) = self.view() {
            v.deselect();
          }
          return self.wake();
        }
        _ => {}
      }
    }
    match key {
      Key::Named(NamedKey::Escape) | Key::Character("h") if !ctrl && self.feed_back() => {}
      Key::Character("?") => self.open_screen(screen::Page::Keys),
      Key::Character("/") if !ctrl => self.find_open(),
      Key::Character("n") if !ctrl && self.find_on() => self.find_step(1),
      Key::Character("N") if !ctrl && self.find_on() => self.find_step(-1),
      Key::Named(NamedKey::Escape) if self.find_on() => _ = self.find_clear(),
      Key::Character("v") if !ctrl => self.select_start(),
      Key::Character("x") if !ctrl && !alt => self.pin_hide(),
      Key::Character("t") if chord == Some('g') => self.select(self.step_tab(self.active, false)),
      Key::Character("T") if chord == Some('g') => self.select(self.step_tab(self.active, true)),
      Key::Character("g") if chord == Some('g') => {
        let res = &mut self.res;
        if let Some(t) = self.tabs.get_mut(self.active) {
          t.view.scroll_to_top(res);
        }
        self.wake();
      }
      Key::Character("g") => self.chord = Some('g'),
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => self.scroll(-line),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => self.scroll(line),
      Key::Character("w") if ctrl => self.close_tab(),
      Key::Character("i" | "a") | Key::Named(NamedKey::Enter) if !ctrl && !alt => self.enter_insert(),
      Key::Character("d") if ctrl => self.scroll(-half),
      Key::Character("u") if ctrl => self.scroll(half),
      Key::Character("G") | Key::Named(NamedKey::End) => {
        if let Some(v) = self.view() {
          v.scroll_to_bottom();
        }
        self.wake();
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
      .with_title("heatsync")
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
        if !f {
          self.hide_tip();
        }
      }
      WindowEvent::Occluded(o) => {
        self.occluded = o;
        if !o {
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
        self.link_hover();
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
        } else if self.picker.is_some() {
          self.picker_wheel(px);
        } else {
          self.scroll(px);
        }
      }
      WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
        self.hide_tip();
        let (k, t) = (event.logical_key.as_ref(), event.text.as_deref());
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
        if self.menu.is_some() {
          return self.menu_key(k);
        }
        match self.mode {
          Mode::Insert => self.key_insert(k, t),
          Mode::Normal => self.key_normal(k, t),
          Mode::Chat => self.key(k),
        }
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
    let wake = [self.flood, anim, self.trim_at, self.mute_due].into_iter().flatten().min();
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
        let history: Vec<Message> = events
          .into_iter()
          .filter_map(|e| match e {
            Chat::Msg(mut m) => {
              m.flags |= flag::HISTORY;
              self.scan_mention(Platform::Twitch, &ch, &mut m);
              Some(m)
            }
            _ => None,
          })
          .collect();
        self.tabs[i].view.prepend(history);
        self.trim_soon();
        if i == self.active {
          self.wake();
        }
      }
      NetEvent::ModDone(d) => self.on_mod_done(*d),
      NetEvent::Card { platform, login, res } => self.on_card(platform, &login, res),
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
      NetEvent::Voted { id, res } => self.on_voted(id, res),
      NetEvent::Posted { res } => self.on_posted(res),
      NetEvent::FeedLive(p) => self.on_feed_live(p),
      NetEvent::Pin { platform, key, pin } => self.on_pin(platform, &key, pin),
      NetEvent::Stream(l) => self.on_stream(l),
      NetEvent::Moment { platform, key, text } => self.on_moment(platform, &key, &text),
      NetEvent::Emotes(ch, p, set) => {
        self.res.emotes.set(ch.as_deref(), p, set);
        self.trim_soon();
        self.relayout_where(ch.as_deref());
      }
      NetEvent::Image(id, h, res) => {
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
        self.status = match &me {
          Some(m) => format!("logged in as {}", m.username),
          None => String::new(),
        };
        // a different login starts with a clean whispers tab
        if self.me.as_ref().map(|m| &m.username) != me.as_ref().map(|m| &m.username) {
          self.whispers_reset();
          self.filters_login_changed();
          self.feed_reset();
          self.res.blocked.clear_server();
          self.mod_in.clear();
          self.relayout_where(None);
        }
        self.me = me;
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
        for i in self.tabs_with(platform, &key) {
          self.tabs[i].view.clear_user(&login);
          self.touched(i);
        }
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
          self.tabs[i].view.push(Message::system(now_ms(), Platform::Youtube, format!("{line} ({key})")));
          self.touched(i);
        }
      }
      NetEvent::Links { tab, kick, yt } => self.on_links(&tab, kick, yt),
      NetEvent::Upload(u) => self.on_upload(u),
      NetEvent::Status(s) => {
        self.status = s;
        self.bar_dirty = true;
        self.wake();
      }
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
}
