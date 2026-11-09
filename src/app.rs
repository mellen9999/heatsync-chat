// window + event loop glue: tabs of chat views fed by the net thread
use std::rc::Rc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

mod account;
mod acct;
mod blocked;
mod builtin;
mod card;
mod cardclip;
mod chain;
mod chantools;
mod chips;
mod clip;
mod complete;
mod dmdel;
mod emote;
mod emote_up;
mod engage;
mod feed;
mod filters;
mod find;
mod follow;
mod following;
mod font;
mod gifs;
mod held;
mod ime;
mod img_view;
mod import;
mod input;
mod inventory;
mod jump;
mod keys;
mod layout;
mod link;
mod lists;
mod live_dots;
mod lock;
mod logs;
mod media_peek;
mod mentions;
mod modbar;
mod mods;
mod modseed;
mod modsuite;
mod modtools;
mod mrules;
mod net_events;
mod notes;
mod overlay;
mod peek;
mod personal;
mod picker;
mod picker_search;
mod pin;
mod pings;
mod platforms;
mod pointer;
mod pop;
mod pred;
mod pronouns;
mod pushed;
mod quick;
mod raise;
mod react;
mod render;
mod resume;
mod rover;
mod rulesound;
mod screen;
mod sendto;
mod sent;
mod stats;
mod status;
mod stream;
mod streamer;
mod suggest;
mod tabs;
mod tabsync;
mod tagcard;
mod temp;
#[cfg(test)]
mod tests_cardclip;
#[cfg(test)]
mod tests_chain;
#[cfg(test)]
mod tests_copy;
#[cfg(test)]
mod tests_emote_up;
#[cfg(test)]
mod tests_findbar;
#[cfg(test)]
mod tests_font;
#[cfg(test)]
mod tests_input;
#[cfg(test)]
mod tests_kickmod;
#[cfg(test)]
mod tests_lists;
#[cfg(test)]
mod tests_mentions;
#[cfg(test)]
mod tests_modbar;
#[cfg(test)]
mod tests_mrules;
#[cfg(test)]
mod tests_nuke;
#[cfg(test)]
mod tests_parity;
#[cfg(test)]
mod tests_path;
#[cfg(test)]
mod tests_pickers;
#[cfg(test)]
mod tests_pin;
#[cfg(test)]
mod tests_pings;
#[cfg(test)]
mod tests_platfilter;
#[cfg(test)]
mod tests_pushed;
#[cfg(test)]
mod tests_reach;
#[cfg(test)]
mod tests_react;
#[cfg(test)]
mod tests_rowmode;
#[cfg(test)]
mod tests_runs;
#[cfg(test)]
mod tests_screens;
#[cfg(test)]
mod tests_sendto;
#[cfg(test)]
mod tests_sound;
#[cfg(test)]
mod tests_store;
#[cfg(test)]
mod tests_streamer;
#[cfg(test)]
mod tests_tabs;
#[cfg(test)]
mod tests_usercmd;
#[cfg(test)]
mod tests_whisper_toast;
#[cfg(test)]
mod tests_ytgone;
mod tip;
mod unseen;
mod update;
mod upload;
mod usercmd;
#[cfg(test)]
mod ux_tests;
mod whispers;
#[cfg_attr(unix, path = "app/winsys_unix.rs")]
#[cfg_attr(windows, path = "app/winsys_windows.rs")]
mod winsys;
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
  // the one unsaved tab: a channel opened to look at (temp.rs). never written, never synced
  temp: bool,
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
  // twitch channels whose history just loaded, asked whether they are live and since when (stream.rs)
  live_asked: std::collections::HashSet<Box<str>>,
  // channels whose went-live line has shown and when, until they sign off (the socket and the poll both say it)
  told: std::collections::HashMap<(crate::model::message::Platform, Box<str>), i64>,
  // which open channels are live (the tab's red dot), and when to ask again
  live: crate::model::live_dots::Live,
  live_due: Option<Instant>,
  // when the soonest timed mute ends
  mute_due: Option<Instant>,
  next_anim: Option<Instant>,
  // the emote hover popup, and when its animated emote flips next
  tip: Option<crate::ui::tip::Tip>,
  tip_next: Option<Instant>,
  // heat counts fall with the clock: the tab bar repaints each second while one shows
  heat_due: Option<Instant>,
  // a feed post's heat moved live: the redraw waits this long so a run of frames is one (react.rs)
  feed_heat_due: Option<Instant>,
  // pushes that mean "ask again" (follows, blocked emotes, blocks), one request per burst (pushed.rs)
  pushed: crate::model::livegate::Gates,
  // your mention rules as the server holds them (app/mrules.rs); empty until read
  mrules: Vec<crate::model::mrules::Rule>,
  // the first answer for this login has come
  mrules_read: bool,
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
  // your twitch follows being picked into tabs (import.rs)
  import: import::Import,
  notes: crate::model::notes::Notes,
  // per-channel go-live bell and muted pings (model::pings)
  pings: crate::model::pings::Pings,
  // platforms each merged tab hides (model::platfilter): device-local, never synced
  plats: crate::model::platfilter::PlatFilter,
  // your own slash commands
  ucmds: crate::model::usercmd::Cmds,
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
  // the hover reply button was mentioned once already this run
  reply_nudged: bool,
  me: Option<Me>,
  // settings that follow the heatsync account: what it holds, whether the heatsync socket is signed in, and
  // when the settings were last asked for
  acct: crate::model::sync::Acct,
  hs_authed: bool,
  // our heatsync user id, as the socket signed in (our own reaction frames are told apart by it)
  hs_uid: Option<Box<str>>,
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
  // a reply chain over the active tab (the tab's own lines wait in `Tab::under`)
  chain: Option<chain::Open>,
  // tab mode's cursor on the tab bar (space t)
  rover: Option<rover::Rover>,
  // our own mod actions waiting for twitch's echo, so the mod log can name who did them: (room, target, when)
  mod_by: Vec<(Box<str>, Box<str>, i64)>,
  // alt-r row mode is on: keys drive the picked line, not the box (never set with vi, whose normal mode does)
  rows: bool,
  // the settings screen or the key map, covering the chat
  screen: Option<screen::Screen>,
  // last drawn tab rects, then the + button
  bar_hits: Vec<Rect>,
  // the tab index behind each drawn rect (hidden tabs have none)
  bar_map: Vec<usize>,
  upd: update::Update,
  // streamer mode: on or not, the obs look, whispers shown on purpose
  streamer: streamer::State,
  // the title flashing who @'d you while the window is away: their name, and when it next turns
  flash: Option<(Box<str>, bool)>,
  flash_due: Option<Instant>,
  // the taskbar was asked to light up for a mention and the window hasn't been looked at since
  attention: bool,
  // the font family last asked of fontconfig, and whether it was typed (a failure then takes it back); the answer
  // for any other name is stale and dropped
  font_ask: Option<(Box<str>, bool)>,
  // desktop mention popups: one per 3s, the rest counted
  notify_pace: crate::util::notify::Coalesce,
  // the same for went-live and game-switch popups
  live_pace: crate::util::notify::Coalesce,
  // display scale factor (zoom multiplies it), and smooth-wheel travel toward a zoom step
  dpi: f32,
  zoom_acc: f32,
  picker: Option<Picker>,
  // when the picker's typed word has rested long enough to be searched in the catalogs
  pick_due: Option<std::time::Instant>,
  gifs: gifs::State,
  recents: Recents,
  // how often each emote gets picked: tab ranks by it
  frecency: Frecency,
  // tab-complete's catalog search: the newest ask's number, and what was taken from it (never read by chat)
  catalog_epoch: u32,
  catalog_reg: crate::model::catalog::Registry,
  // the built-in path prompt (no file dialog installed), and an upload or dialog in flight
  prompt: Option<Line>,
  // the newest tab-completion asked of the path prompt; an answer to an older one is dropped
  path_seq: u64,
  uploading: bool,
  // a sign-in is waiting on the browser: esc gives it up
  login_wait: bool,
  // the pinned-lines bar and the prediction banner under the tabs
  pin_bar: pin::PinBar,
  jump: jump::Jump,
  modbar: modbar::ModBar,
  pred_bar: pred::PredBar,
  // why you can't chat in each room, and when the box's countdown next changes a digit
  locks: crate::model::lock::Locks,
  lock_sec: Option<Instant>,
  moments: Moments,
  // followed streamers' twitch colors (`follow:colors`), by lowercase login
  follow_colors: std::collections::HashMap<Box<str>, u32>,
  // names that ping you, and whether the window has focus (notifications only fire when it does not)
  mentions: Mentions,
  // the mentions tab's lines are written to disk at this time (debounced)
  mentions_save: Option<Instant>,
  // every live line goes to a file on disk when "save chat logs" is on (off by default)
  chatlog: crate::chatlog::ChatLog,
  // what you sent through heatsync.org lately: its echo wears [H]
  origins: crate::model::origin::Origins,
  // sends to several feeds, so their echoes fold into one line
  sentfold: crate::model::sentfold::Sent,
  focused: bool,
  whisper: whispers::Whispers,
  unseen: crate::model::unseen::Unseen,
  // heatsync notification rows in the mentions tab the account doesn't know are read yet
  notif_unread: bool,
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
        temp: false,
      });
    }
    // no channel to open (a first run, or nothing saved): the add-channel form greets the window
    let welcome = tabs.is_empty();
    let set = Settings::load();
    let acct = crate::model::sync::Acct::new(&set);
    crate::heatsync::content::set_shown(set.content());
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
    res.cosmetics.hs_on = set.hs_paints;
    res.cosmetics.hidden = crate::cosmetics::hidden::Hidden::load();
    for t in &tabs {
      tabs::seat_hosts(&mut res.chatters, &t.sources);
    }
    res.arts.set_motion(set.animate_emotes == crate::model::settings::Animate::Always);
    res.personal.set_on(set.personal_emotes);
    if flood {
      crate::fake::seed_paints(&mut res.cosmetics);
      res.heats.mute();
      res.flairs.mute();
    }
    let streamer = streamer::State::boot(&set);
    res.previews.hush = streamer.on || !set.link_previews;
    res.media = set.media && !streamer.on;
    res.apply_look(&set);
    res.links = set.links;
    let mut app = Self {
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
      live_asked: Default::default(),
      told: Default::default(),
      live: Default::default(),
      live_due: Some(Instant::now() + std::time::Duration::from_secs(2)),
      modes_busy: false,
      mute_due,
      next_anim: None,
      tip: None,
      tip_next: None,
      heat_due: None,
      feed_heat_due: None,
      pushed: Default::default(),
      mrules: Vec::new(),
      mrules_read: false,
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
      import: Default::default(),
      notes: crate::model::notes::Notes::load(),
      pings: crate::model::pings::Pings::load(),
      plats: crate::model::platfilter::PlatFilter::load(),
      ucmds: crate::model::usercmd::Cmds::load(),
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
      reply_nudged: false,
      me: None,
      acct,
      hs_authed: false,
      hs_uid: None,
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
      chain: None,
      rover: None,
      mod_by: Vec::new(),
      rows: false,
      screen: None,
      bar_hits: Vec::new(),
      bar_map: Vec::new(),
      streamer,
      upd: update::Update { release: None, installing: false, due: Some(Instant::now() + update::FIRST_CHECK) },
      flash: None,
      flash_due: None,
      attention: false,
      font_ask: None,
      notify_pace: Default::default(),
      live_pace: crate::util::notify::Coalesce::new("stream update", "stream updates"),
      dpi: 1.0,
      zoom_acc: 0.0,
      picker: None,
      pick_due: None,
      gifs: Default::default(),
      recents: Recents::load(),
      frecency: Frecency::load(Recents::load().names(), now_ms()),
      catalog_epoch: 0,
      catalog_reg: Default::default(),
      prompt: None,
      path_seq: 0,
      uploading: false,
      login_wait: false,
      pin_bar: Default::default(),
      jump: Default::default(),
      modbar: Default::default(),
      pred_bar: Default::default(),
      locks: Default::default(),
      lock_sec: None,
      moments: Moments::default(),
      follow_colors: Default::default(),
      mentions: Mentions::default(),
      mentions_save: None,
      chatlog: crate::chatlog::ChatLog::new(set.chat_logs),
      origins: Default::default(),
      sentfold: Default::default(),
      focused: true,
      whisper: Default::default(),
      unseen: Default::default(),
      notif_unread: false,
      feed: Feed::default(),
      pending_post: None,
      edit: None,
      pending_delete: None,
    };
    // the saved font family is looked up now; until it answers, the system monospace
    app.font_follow(false);
    app.sync_plats();
    app.modlog_restore();
    app.sent_restore();
    app
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
      self.mark_read();
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

impl App {
  // makes the window and its pixel surface; false (and the app exits) when either can't be had
  fn open_window(&mut self, el: &ActiveEventLoop, size: winit::dpi::Size) -> bool {
    // x11 shows it; wayland compositors take their icon from the .desktop file instead
    let icon = crate::ui::logo::window_rgba().and_then(|(px, w)| winit::window::Icon::from_rgba(px, w, w).ok());
    let attrs = Window::default_attributes().with_title(mentions::TITLE).with_window_icon(icon).with_inner_size(size);
    let attrs = winsys::named(attrs, APP_ID);
    let window = match el.create_window(attrs) {
      Ok(w) => Rc::new(w),
      Err(e) => {
        eprintln!("window: {e}");
        el.exit();
        return false;
      }
    };
    let surface = Context::new(window.clone()).and_then(|ctx| Surface::new(&ctx, window.clone()));
    let surface = match surface {
      Ok(s) => s,
      Err(e) => {
        eprintln!("surface: {e}");
        el.exit();
        return false;
      }
    };
    self.dpi = window.scale_factor() as f32;
    self.res.set_scale(self.scale());
    let size = window.inner_size();
    self.gfx = Some(Gfx { window, surface });
    self.occluded = false;
    self.resize(size.width, size.height);
    true
  }
}

impl ApplicationHandler<NetEvent> for App {
  fn resumed(&mut self, el: &ActiveEventLoop) {
    if self.gfx.is_some() {
      return;
    }
    let first = self.open_window(el, winit::dpi::LogicalSize::new(420, 800).into());
    if first {
      self.first_run();
    }
  }

  fn user_event(&mut self, el: &ActiveEventLoop, ev: NetEvent) {
    match ev {
      NetEvent::Raise(open) => self.raise(el, &open),
      ev => self.on_net(ev),
    }
  }

  fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, ev: WindowEvent) {
    match ev {
      WindowEvent::CloseRequested => {
        self.flush_to_disk();
        el.exit()
      }
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
      WindowEvent::Focused(f) => self.on_window_focus(f),
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
        self.modbar_hover();
        self.chat_hover();
        self.suggest_hover();
      }
      WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.click(),
      WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => self.right_click(),
      WindowEvent::Ime(ev) => self.on_ime(ev),
      WindowEvent::DroppedFile(p) => self.upload_file(p),
      WindowEvent::CursorLeft { .. } => {
        self.hide_tip();
        if self.hover.tab.take().is_some() {
          self.bar_dirty = true;
          self.wake();
        }
        self.modbar_leave();
      }
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
    if self.pick_due.is_some_and(|t| t <= now) {
      self.picker_search_tick();
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
    self.streamer_tick(now);
    self.notify_pace_tick(now);
    self.live_pace_tick(now);
    self.mentions_save_tick(now);
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
    if self.live_due().is_some_and(|t| t <= now) {
      self.live_tick(now);
    }
    if self.pushed_due().is_some_and(|t| t <= now) {
      self.pushed_tick(now);
    }
    self.ui_state_tick();
    if self.heat_due.is_some_and(|t| t <= now) {
      self.heat_due = None;
      self.bar_dirty = true;
      self.wake();
    }
    if self.feed_heat_due.is_some_and(|t| t <= now) {
      self.feed_heat_tick();
    }
    let wake = [
      self.flood,
      anim,
      self.trim_at,
      self.mute_due,
      self.peek_due,
      self.tag_due(),
      self.gifs.due(),
      self.pick_due,
      self.upd.due,
      self.heat_due,
      self.feed_heat_due,
      self.pace_due,
      self.flash_due,
      self.streamer.due,
      self.notify_pace.due(),
      self.live_pace.due(),
      self.mentions_save,
      self.held.due(),
      self.ui_state_due(),
      self.following_due(),
      self.live_due(),
      self.pushed_due(),
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
