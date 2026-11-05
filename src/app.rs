// window + event loop glue: tabs of chat views fed by the net thread
use std::rc::Rc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::platform::x11::WindowAttributesExtX11;
use winit::window::{Window, WindowId};

mod account;
mod acct;
mod blocked;
mod builtin;
mod card;
mod chantools;
mod chips;
mod clip;
mod complete;
mod emote;
mod engage;
mod feed;
mod filters;
mod find;
mod follow;
mod following;
mod gifs;
mod held;
mod ime;
mod img_view;
mod input;
mod inventory;
mod jump;
mod keys;
mod layout;
mod link;
mod lock;
mod logs;
mod media_peek;
mod mentions;
mod mods;
mod modsuite;
mod modtools;
mod net_events;
mod notes;
mod notifs;
mod overlay;
mod peek;
mod picker;
mod pin;
mod pointer;
mod pop;
mod pred;
mod pronouns;
mod quick;
mod render;
mod resume;
mod rover;
mod screen;
mod sendto;
mod stats;
mod status;
mod stream;
mod suggest;
mod tabs;
mod tabsync;
mod tagcard;
#[cfg(test)]
mod tests_input;
#[cfg(test)]
mod tests_kickmod;
#[cfg(test)]
mod tests_pickers;
#[cfg(test)]
mod tests_screens;
#[cfg(test)]
mod tests_sendto;
#[cfg(test)]
mod tests_store;
#[cfg(test)]
mod tests_tabs;
mod tip;
mod unseen;
mod update;
mod upload;
#[cfg(test)]
mod ux_tests;
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
use crate::model::message::Platform;
use crate::model::picker::{Picker, Recents};
use crate::model::settings::Settings;
use crate::model::stream_events::Moments;
use crate::model::tabs::Kind;
use crate::model::vi::{Mode, Vi};
use crate::net::auth::Me;
use crate::net::irc::Chat;
use crate::net::{Net, NetEvent, Reply};
use crate::store;
use crate::ui::canvas::{Canvas, Rect};
use crate::ui::chat_view::ChatView;
use crate::ui::form::Hit;
use crate::ui::input_bar;
use crate::ui::mode_label;
use crate::ui::res::Res;
use crate::util::time::now_ms;

const FLOOD_EVERY: Duration = Duration::from_millis(50);
const FLOOD_TAB: &str = "flood";
// matches packaging/heatsync-chat.desktop (StartupWMClass) and its icon name
const APP_ID: &str = "heatsync-chat";
const SCROLLBACK: usize = 1000;
// the empty box's first words while logged out: a button
const LOGIN_HINT: &str = "ctrl-l log in";
// animation repaint ceiling: 30fps — smooth for emotes, ~35% fewer wakeups than native gif timing
const ANIM_MIN_MS: u32 = 33;
const TRIM_AFTER: Duration = Duration::from_secs(2);

struct Gfx {
  window: Rc<Window>,
  surface: Surface<Rc<Window>, Rc<Window>>,
}

struct Tab {
  name: Box<str>,
  kind: Kind,
  view: ChatView,
  unread: bool,
  // a stream line (live, offline, game switch) landed while you were elsewhere: the tab reads yellow
  event: bool,
  // every feed merged into this tab: (platform, routing key)
  sources: Vec<(Platform, Box<str>)>,
  // the source the box speaks as (completions, the hint, watch): one of the feeds a send reaches
  target: usize,
  // which feeds a send reaches (ctrl-n and alt-t/k/y on merged tabs); synced in the tab's entry
  send: crate::model::sendto::Targets,
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
  // the chat search: is the box the search, the query last kept, its count
  find: find::FindUi,
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
  // kick slugs where our own lines wear a moderator or broadcaster badge (model::modrole)
  kick_mod: std::collections::HashSet<Box<str>>,
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
  // heatsync refs that beat their twitch line here
  late_refs: crate::heatsync::late_refs::LateRefs,
  // the tab tooltip or a suggestion list sat over the chat last frame (repaint under it)
  over_input: bool,
  sug: suggest::SuggestUi,
  // where the box's text was last drawn (a click places the cursor)
  input_hit: Option<input_bar::Hit>,
  // the "ctrl-l log in" button of the empty box while logged out
  login_link: Option<Rect>,
  // which button the pointer is on
  hover: pointer::Hover,
  // an animated emote in the message box changes frame
  input_next: Option<Instant>,
  // the name hover popup, when the pointer rested long enough to ask for it, and what heatsync said
  peek: Option<peek::Peek>,
  media_peek: Option<media_peek::MediaPeek>,
  peek_due: Option<Instant>,
  cards: peek::Cards,
  pronouns: crate::model::pronouns::Pronouns,
  tag: tagcard::TagUi,
  // automod holds in rooms we moderate
  held: held::Queue,
  // who you follow that is live (following.rs)
  following: following::Following,
  notes: crate::model::notes::Notes,
  occluded: bool,
  damage: render::Damage,
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
  // your channel tabs as the account holds them (tabsync.rs)
  tabsync: crate::model::tabsync::TabSync,
  // input-method composition, and the window's side of it
  ime: Ime,
  ime_win: ime::ImeWin,
  // the floating menu that is open, if any: login and line actions, the user card, the menu on an emote,
  // the menu on a right-clicked tab (overlay.rs)
  overlay: Option<overlay::Overlay>,
  // the add/edit channel form (covers the chat)
  form: Option<Form>,
  // the first window of a run with no channels asks for one (tabs::first_run)
  welcome: bool,
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
  // the settings screen or the key map, covering the chat
  screen: Option<screen::Screen>,
  // last drawn tab rects, then the + button
  bar_hits: Vec<Rect>,
  // the tab index behind each drawn rect (hidden tabs have none)
  bar_map: Vec<usize>,
  upd: update::Update,
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
  // tab-complete's catalog search: the newest ask's number, and what was taken from it (never read by chat)
  catalog_epoch: u32,
  catalog_reg: crate::model::catalog::Registry,
  // the built-in path prompt (no file dialog installed), and an upload or dialog in flight
  prompt: Option<Line>,
  uploading: bool,
  // a sign-in is waiting on the browser: esc gives it up
  login_wait: bool,
  // the pinned-lines bar and the prediction banner under the tabs
  pin_bar: pin::PinBar,
  jump: jump::Jump,
  pred_bar: pred::PredBar,
  // why you can't chat in each room, and when the box's countdown next changes a digit
  locks: crate::model::lock::Locks,
  lock_sec: Option<Instant>,
  moments: Moments,
  // followed streamers' twitch colors (`follow:colors`), by lowercase login
  follow_colors: std::collections::HashMap<Box<str>, u32>,
  // names that ping you, and whether the window has focus (notifications only fire when it does not)
  mentions: Mentions,
  // what you sent through heatsync.org lately: its echo wears [H]
  origins: crate::model::origin::Origins,
  // sends to several feeds, so their echoes fold into one line
  sentfold: crate::model::sentfold::Sent,
  focused: bool,
  whisper: whispers::Whispers,
  unseen: crate::model::unseen::Unseen,
  // the notifs tab's list and unread count
  notifs: crate::model::inbox::Inbox,
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
        send: Default::default(),
        under: None,
        count: Default::default(),
        stats: Default::default(),
      });
    }
    // no channel to open (a first run, or nothing saved): the add-channel form greets the window
    let welcome = tabs.is_empty();
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
    res.cosmetics.hs_on = set.hs_paints;
    res.cosmetics.hidden = crate::cosmetics::hidden::Hidden::load();
    for t in &tabs {
      tabs::seat_hosts(&mut res.chatters, &t.sources);
    }
    res.arts.set_motion(set.paint_motion);
    if flood {
      crate::fake::seed_paints(&mut res.cosmetics);
      res.heats.mute();
      res.flairs.mute();
    }
    res.media = set.media;
    res.apply_look(&set);
    res.partial_links = set.partial_links;
    res.links = set.links;
    Self {
      gfx: None,
      canvas: Canvas::new(0, 0),
      res,
      tabs,
      active,
      net,
      bar_dirty: true,
      find: Default::default(),
      mods: ModifiersState::empty(),
      chord: None,
      fake: Fake::new(0x5eed),
      flood: flood.then(Instant::now),
      prof: std::env::var_os("HSC_PROF").is_some(),
      rooms: Vec::new(),
      mod_in: Default::default(),
      kick_mod: Default::default(),
      modes: Default::default(),
      modes_busy: false,
      mute_due,
      next_anim: None,
      tip: None,
      tip_next: None,
      heat_due: None,
      pacer: Default::default(),
      pace_due: None,
      late_refs: Default::default(),
      input_next: None,
      input_hit: None,
      login_link: None,
      hover: Default::default(),
      over_input: false,
      sug: Default::default(),
      peek: None,
      media_peek: None,
      peek_due: None,
      cards: peek::Cards::default(),
      pronouns: Default::default(),
      tag: Default::default(),
      held: Default::default(),
      following: Default::default(),
      notes: crate::model::notes::Notes::load(),
      occluded: false,
      damage: Default::default(),
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
      tabsync: Default::default(),
      ui_get_at: None,
      ime: Ime::default(),
      ime_win: Default::default(),
      overlay: None,
      form: None,
      welcome,
      form_hover: None,
      cursor: winit::window::CursorIcon::Default,
      img_view: None,
      logs: None,
      rover: None,
      screen: None,
      bar_hits: Vec::new(),
      bar_map: Vec::new(),
      upd: update::Update { release: None, installing: false, due: Some(Instant::now() + update::FIRST_CHECK) },
      flash: None,
      flash_due: None,
      dpi: 1.0,
      zoom_acc: 0.0,
      picker: None,
      gifs: Default::default(),
      recents: Recents::load(),
      frecency: Frecency::load(Recents::load().names(), now_ms()),
      catalog_epoch: 0,
      catalog_reg: Default::default(),
      prompt: None,
      uploading: false,
      login_wait: false,
      pin_bar: Default::default(),
      jump: Default::default(),
      pred_bar: Default::default(),
      locks: Default::default(),
      lock_sec: None,
      moments: Moments::default(),
      follow_colors: Default::default(),
      mentions: Mentions::default(),
      origins: Default::default(),
      sentfold: Default::default(),
      focused: true,
      whisper: Default::default(),
      unseen: Default::default(),
      notifs: Default::default(),
      feed: Feed::default(),
      pending_post: None,
      edit: None,
      pending_delete: None,
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
      self.close_card();
      self.close_pop();
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
    self.first_run();
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
          self.damage.expose = true;
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
        self.input_hover();
        self.jump_hover();
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
    self.say_store_notes();
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
    if self.upd.due.is_some_and(|t| t <= now) {
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
    if self.pred_bar.sec.is_some_and(|t| t <= now) {
      self.pred_second();
    }
    if self.lock_sec.is_some_and(|t| t <= now) {
      self.lock_second();
    }
    if self.pred_due(now).is_some_and(|t| t <= now) {
      self.pred_tick(now);
    }
    if self.following_due().is_some_and(|t| t <= now) {
      self.following_tick(now);
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
      self.upd.due,
      self.heat_due,
      self.pace_due,
      self.flash_due,
      self.held.due(),
      self.ui_state_due(),
      self.following_due(),
      self.pred_due(now),
      self.pred_bar.sec.filter(|_| !self.occluded),
      self.lock_sec.filter(|_| !self.occluded),
    ]
    .into_iter()
    .flatten()
    .min();
    el.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
  }
}

pub fn persist(channels: &[Box<str>]) {
  store::save_tabs(channels);
}

impl App {
  // a save that failed, a file set aside: said once, in the chat on screen
  fn say_store_notes(&mut self) {
    for note in store::take_notes() {
      self.say(note);
    }
  }
}
