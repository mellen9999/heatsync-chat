// user prefs, persisted as `settings` (key=value lines). vi + auto_hide default off: a plain chat box
use crate::heatsync::content::Cat;
use crate::model::schema::{self, Ctl};
use crate::model::tabs::Kind;
use crate::store;

pub const ZOOM_MIN: u16 = 90;
pub const ZOOM_MAX: u16 = 300;
pub const ZOOM_STEP: u16 = 10;

// percent, kept whole so repeated steps never drift
pub fn clamp_zoom(pct: i32) -> u16 {
  pct.clamp(ZOOM_MIN as i32, ZOOM_MAX as i32) as u16
}

// where the tab bar sits (ext tabPosition): a row on top or bottom, a column on the left or right
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Side {
  #[default]
  Top,
  Right,
  Bottom,
  Left,
}

impl Side {
  // the ext's order, which h / l step through
  pub const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

  pub fn name(self) -> &'static str {
    match self {
      Side::Top => "top",
      Side::Right => "right",
      Side::Bottom => "bottom",
      Side::Left => "left",
    }
  }

  pub fn parse(s: &str) -> Option<Side> {
    Self::ALL.into_iter().find(|d| d.name() == s.trim())
  }

  pub fn step(self, up: bool) -> Side {
    let i = Self::ALL.iter().position(|&d| d == self).unwrap_or(0);
    Self::ALL[(i + if up { 1 } else { 3 }) % 4]
  }
}

// the number after a channel tab's name (ext tabCounter): unread since you looked, the last minute's
// messages, or nothing
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Counter {
  Off,
  #[default]
  Unread,
  Heat,
}

impl Counter {
  pub const ALL: [Counter; 3] = [Counter::Off, Counter::Unread, Counter::Heat];

  pub fn name(self) -> &'static str {
    match self {
      Counter::Off => "off",
      Counter::Unread => "unread",
      Counter::Heat => "heat",
    }
  }

  pub fn parse(s: &str) -> Option<Counter> {
    Self::ALL.into_iter().find(|d| d.name() == s.trim())
  }

  pub fn step(self, up: bool) -> Counter {
    let i = Self::ALL.iter().position(|&d| d == self).unwrap_or(0);
    Self::ALL[(i + if up { 1 } else { 2 }) % 3]
  }
}

// a setting with a few named values, stepped with h / l and typed as
// `/cmd value`. one row in schema.rs reads it through `Ctl::Pick`; the file stores the value's name
macro_rules! choice {
  ($(#[$doc:meta])* $name:ident { $fv:ident = $fs:literal $(, $v:ident = $s:literal)* $(,)? }) => {
    $(#[$doc])*
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum $name {
      $fv,
      $($v),*
    }

    impl $name {
      pub const ALL: &'static [$name] = &[$name::$fv, $($name::$v),*];
      pub const CHOICE: Choice = Choice { names: &[$fs, $($s),*], usage: concat!("<", $fs, $("|", $s,)* ">") };
    }

  };
}

pub struct Choice {
  pub names: &'static [&'static str],
  // `<a|b|c>`, for the slash list
  pub usage: &'static str,
}

choice! {
  // the clock on a timestamped line (ext timestampFormat)
  TimeFmt { H24 = "24h", H12 = "12h" }
}
choice! {
  // emote size in chat, times the base (ext hs_emote_size)
  EmoteSize { X1 = "1x", X2 = "2x", X4 = "4x" }
}
choice! {
  // chat row line height in the ext's px: 20 is tight, a bigger one adds room between lines
  LineHeight { L20 = "20", L22 = "22", L26 = "26" }
}
choice! {
  // the mention ping's volume, percent (ext mentionSoundVolume, 0.3 by default); 0 is silent
  Volume { V0 = "0", V10 = "10", V20 = "20", V30 = "30", V50 = "50", V70 = "70", V100 = "100" }
}
choice! {
  // streamer mode: hide what must not show on a stream. `auto` = while obs is running
  Streamer { Off = "off", On = "on", Auto = "auto" }
}
choice! {
  // animated emotes: play, or hold their first frame (ext animateEmotes, minus its `hover`)
  Animate { Always = "always", Never = "never" }
}

impl Volume {
  pub fn percent(self) -> u8 {
    [0, 10, 20, 30, 50, 70, 100][self as usize]
  }
}

impl EmoteSize {
  pub fn mul(self) -> u16 {
    [1, 2, 4][self as usize]
  }
}

impl LineHeight {
  // the row's line height as a ratio of the base line
  pub fn ratio(self) -> f32 {
    [1.0, 1.1, 1.3][self as usize]
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
  // everything's size, percent of the display scale (100 = native)
  pub zoom: u16,
  pub tab_side: Side,
  pub tab_counter: Counter,
  // vim keys in the message box + chat-driving mode
  pub vi: bool,
  // the input bar takes no room while empty and unused
  pub auto_hide: bool,
  // a time before each chat line, and its clock. default off, like the ext
  pub timestamps: bool,
  pub time_fmt: TimeFmt,
  // the #444444 stripe on every other row. default on
  pub zebra: bool,
  // lift dim name colors until they read on black (ext hs_readable_names). default on
  pub readable: bool,
  // [T] [K] [Y] on merged and non-twitch tabs (ext showPlatformBadges). default on
  pub platform_tags: bool,
  // a picture beside each name (ext avatars). default off, like the ext
  pub avatars: bool,
  pub emote_size: EmoteSize,
  // unicode emoji in chat bodies, same steps (ext hs_emoji_size). default 2x
  pub emoji_size: EmoteSize,
  pub line_height: LineHeight,
  pub animate_emotes: Animate,
  // a timed-out or banned person's lines go dim (ext hs_dim_timeouts). default on
  pub dim_timeouts: bool,
  // orange edge on someone's first line of the session. default on, like the ext
  pub first_glow: bool,
  // a line where you stopped reading when you left the tab or the window. default on
  pub read_line: bool,
  // 7tv paints on names and 7tv badges. default on, like the ext (sevenTvPaints)
  pub paints: bool,
  // heatsync's own animated name paints, apart from 7tv's. default on, like the ext (showNamePaints)
  pub hs_paints: bool,
  // the per-chatter badge sets from bttv, ffz and chatterino, one switch for all three. a channel's own ffz
  // mod / vip art is not one of them: it stays. default on
  pub third_party_badges: bool,
  // what a twitch line wears before the name from heatsync itself: the sub flair a chatter picked there,
  // a flame for a known heatsync user, a heat bar. all default on; the ext has no key for them
  pub sub_flair: bool,
  pub flame: bool,
  pub heat_bars: bool,
  // badges as short text chips instead of pictures (ext textBadges). default off, like the ext
  pub text_badges: bool,
  // pronoundb pronouns beside a twitch name on the card and the name popup. default on, like the ext
  // (hs_show_pronouns). off, pronoundb is never asked
  pub pronouns: bool,
  // count lines, chatters and emotes per channel tab for /summary (ext subsystems stream-stats)
  pub stream_stats: bool,
  // name paints move (scenes + effects). off freezes each to its first frame, like the ext's animatePaints
  pub paint_motion: bool,
  // stream lines: went live (on), went offline (off), game switch (on), chat-spike line (on)
  pub ev_online: bool,
  pub ev_offline: bool,
  pub ev_game: bool,
  // the room's raids out and gift bombs (ext hermesEvents raid / sub, both on)
  pub ev_raid: bool,
  pub ev_sub: bool,
  // other people's channel-point redeems (ext hermesEvents redeem, on)
  pub ev_redeem: bool,
  // a banner under the tabs while a prediction is open or locked (ext hermesEvents pred)
  pub ev_pred: bool,
  pub moments: bool,
  // heatsync feed posts as lines in every chat tab (ext inlineNotifs): new threads, the OP answering in
  // their own thread, replies. all on, like the ext
  pub inline_op: bool,
  pub inline_mop: bool,
  pub inline_re: bool,
  // a whisper or dm while you are on another tab: a toast line (ext whisperToast, default on), and a [DM] row in
  // the chat you are on (ext inlineNotifs dm, default off)
  pub whisper_toast: bool,
  pub inline_dm: bool,
  // inline pictures under chat lines. default on, like the ext (mediaEmbedsEnabled)
  pub media: bool,
  // urls in chat are links at all. default on, like the ext (linksEnabled)
  pub links: bool,
  // draw emotes from a heatsync inventory (the sender's) and suggest your own. default on, like the ext
  // (renderInventoryEmotes, suggestInventoryEmotes)
  pub render_inventory: bool,
  pub suggest_inventory: bool,
  // built-in tabs on the bar (all on, like the ext once you are signed in; /hide and /show)
  pub tab_feed: bool,
  pub tab_whispers: bool,
  pub tab_mentions: bool,
  pub tab_modlog: bool,
  // a desktop notification for an @mention while the window is not focused. default off, like the
  // ext's hs_notifications: nothing leaves the window unless asked
  pub notify: bool,
  // the window title flashes `@name ← heatsync` while unfocused after an @mention. default on, like the ext
  pub title_flash: bool,
  pub mention_volume: Volume,
  // streamer mode (src/app/streamer.rs): whisper text, link previews, popup text and sounds are held back
  pub streamer: Streamer,
  // ...and the mention ping and rule sounds still play while it is on
  pub streamer_sounds: bool,
  // look for a newer release now and then (github sees the ask). default on
  pub updates: bool,
  // every live line of every open channel is appended to a file on this machine (src/chatlog.rs). default off
  pub chat_logs: bool,
  // chat filters (ext hideBots, hideCommands, hideDuplicates): all off by default
  pub hide_bots: bool,
  pub hide_cmds: bool,
  pub hide_dupes: bool,
  // ask before a ban goes out (the ext's modConfirmBan, off by default like the ext)
  pub confirm_ban: bool,
  // the hover toolbar's picked buttons, a mask over model::modbar::CATALOG (ext hs_mod_toolbar_buttons). none by default
  pub mod_buttons: u16,
  // flagged pictures and emotes shown, by category (ext viewer_show_*): all off by default
  pub show_nsfw: bool,
  pub show_violence: bool,
  pub show_weapons: bool,
  pub show_drugs: bool,
  pub show_hate: bool,
}

impl Default for Settings {
  fn default() -> Self {
    Settings {
      zoom: 100,
      tab_side: Side::Top,
      tab_counter: Counter::Unread,
      vi: false,
      auto_hide: false,
      timestamps: false,
      time_fmt: TimeFmt::H24,
      zebra: true,
      readable: true,
      platform_tags: true,
      avatars: false,
      emote_size: EmoteSize::X1,
      emoji_size: EmoteSize::X2,
      line_height: LineHeight::L20,
      animate_emotes: Animate::Always,
      dim_timeouts: true,
      first_glow: true,
      read_line: true,
      paints: true,
      hs_paints: true,
      third_party_badges: true,
      sub_flair: true,
      flame: true,
      heat_bars: true,
      text_badges: false,
      pronouns: true,
      stream_stats: true,
      paint_motion: true,
      ev_online: true,
      ev_offline: false,
      ev_game: true,
      ev_raid: true,
      ev_sub: true,
      ev_redeem: true,
      ev_pred: true,
      moments: true,
      inline_op: true,
      inline_mop: true,
      inline_re: true,
      whisper_toast: true,
      inline_dm: false,
      media: true,
      links: true,
      render_inventory: true,
      suggest_inventory: true,
      tab_feed: true,
      tab_whispers: true,
      tab_mentions: true,
      tab_modlog: false,
      notify: false,
      title_flash: true,
      mention_volume: Volume::V30,
      streamer: Streamer::Off,
      streamer_sounds: false,
      updates: true,
      chat_logs: false,
      hide_bots: false,
      hide_cmds: false,
      hide_dupes: false,
      confirm_ban: false,
      mod_buttons: 0,
      show_nsfw: false,
      show_violence: false,
      show_weapons: false,
      show_drugs: false,
      show_hate: false,
    }
  }
}

impl Settings {
  // the multiplier on the window's scale factor
  pub fn scale(&self) -> f32 {
    self.zoom as f32 / 100.0
  }

  // the content switches, for the one rule in heatsync::content
  pub fn content(&self) -> [(Cat, bool); 5] {
    [
      (Cat::Sexual, self.show_nsfw),
      (Cat::Violence, self.show_violence),
      (Cat::Weapons, self.show_weapons),
      (Cat::Drugs, self.show_drugs),
      (Cat::Hate, self.show_hate),
    ]
  }

  // the site says a category is shown or hidden. true = this moved it
  pub fn set_content(&mut self, c: Cat, on: bool) -> bool {
    let f = match c {
      Cat::Sexual => &mut self.show_nsfw,
      Cat::Violence => &mut self.show_violence,
      Cat::Weapons => &mut self.show_weapons,
      Cat::Drugs => &mut self.show_drugs,
      Cat::Hate => &mut self.show_hate,
    };
    std::mem::replace(f, on) != on
  }

  pub fn shown(&self, k: Kind) -> bool {
    match k {
      Kind::Chat => true,
      Kind::Feed => self.tab_feed,
      Kind::Whispers => self.tab_whispers,
      Kind::Mentions => self.tab_mentions,
      Kind::ModLog => self.tab_modlog,
    }
  }

  pub fn show(&mut self, k: Kind, on: bool) {
    match k {
      Kind::Chat => {}
      Kind::Feed => self.tab_feed = on,
      Kind::Whispers => self.tab_whispers = on,
      Kind::Mentions => self.tab_mentions = on,
      Kind::ModLog => self.tab_modlog = on,
    }
  }

  // every look and behaviour back to its default. kept: which built-in tabs are on the bar (a layout choice,
  // like the tabs themselves). nothing here holds a login, a token, a list or anything you wrote
  pub fn reset(&self) -> Self {
    Settings {
      tab_feed: self.tab_feed,
      tab_whispers: self.tab_whispers,
      tab_mentions: self.tab_mentions,
      tab_modlog: self.tab_modlog,
      ..Self::default()
    }
  }

  pub fn parse(s: &str) -> Self {
    let mut out = Self::default();
    // the three badge switches from before the one: on if any of them was
    let (mut old_badges, mut one_badges) = (None, false);
    for l in s.lines() {
      let Some((k, v)) = l.split_once('=') else { continue };
      let on = v.trim() == "true";
      let k = k.trim();
      if matches!(k, "bttv_badges" | "ffz_badges" | "chatterino_badges") {
        old_badges = Some(old_badges.unwrap_or(false) || on);
        continue;
      }
      one_badges |= k == "third_party_badges";
      if schema::def(k).is_some_and(|d| d.put(&mut out, on) || d.pick(&mut out, v) || d.list_put(&mut out, v)) {
        continue;
      }
      if k == "zoom"
        && let Some(z) = v.trim().parse::<f32>().ok().filter(|z| z.is_finite())
      {
        out.zoom = clamp_zoom((z * 100.0).round() as i32);
      }
      if k == "tab_side"
        && let Some(d) = Side::parse(v)
      {
        out.tab_side = d;
      }
      if k == "tab_counter"
        && let Some(d) = Counter::parse(v)
      {
        out.tab_counter = d;
      }
    }
    // the one switch written by an earlier run of this version wins over the old three
    if let (false, Some(any)) = (one_badges, old_badges) {
      out.third_party_badges = any;
    }
    out
  }

  pub fn render(&self) -> String {
    let mut out = String::with_capacity(256);
    for d in &schema::DEFS {
      if let Some(on) = d.on(self) {
        out.push_str(&format!("{}={on}\n", d.id));
      } else if let Some(name) = d.picked(self) {
        out.push_str(&format!("{}={name}\n", d.id));
      } else if let Some(ids) = d.listed(self) {
        out.push_str(&format!("{}={ids}\n", d.id));
      }
    }
    out.push_str(&format!(
      "zoom={}\ntab_side={}\ntab_counter={}\n",
      self.scale(),
      self.tab_side.name(),
      self.tab_counter.name()
    ));
    out
  }

  // a file that names none of our settings is not ours (or is cut to nothing): it is set aside as
  // settings.bak and said so in chat, never quietly replaced
  pub fn load() -> Self {
    let Some(s) = store::read("settings") else { return Self::default() };
    if !Self::usable(&s) {
      store::set_aside("settings", None);
      return Self::default();
    }
    Self::fold_caps(&s);
    Self::parse(&s)
  }

  // the all-caps switch is a filter rule now. a file that still holds it is moved over once: if it was on, the
  // filters file gets a `caps` line (folded into the rule on its load), then the settings file is written
  // again without the line. the filters file goes first, so a crash between repeats the move and adds nothing
  fn fold_caps(s: &str) {
    let key = |l: &str| l.split_once('=').map(|(k, v)| (k.trim().to_owned(), v.trim() == "true"));
    let lines: Vec<_> = s.lines().filter_map(key).filter(|(k, _)| k == "automod_caps").collect();
    if lines.is_empty() {
      return;
    }
    if lines.iter().any(|(_, on)| *on) {
      let mut f = store::read("filters").unwrap_or_default();
      if !f.lines().any(|l| l == "caps") {
        f.push_str("caps\n");
        if let Err(e) = store::write("filters", &f) {
          eprintln!("save filters: {e}");
          return;
        }
      }
    }
    if let Err(e) = store::write("settings", &Self::parse(s).render()) {
      eprintln!("save settings: {e}");
    }
  }

  // empty (a fresh start), or at least one line that is a setting we know
  fn usable(s: &str) -> bool {
    let known = |k: &str| schema::def(k).is_some() || matches!(k, "zoom" | "tab_side" | "tab_counter");
    s.trim().is_empty() || s.lines().any(|l| l.split_once('=').is_some_and(|(k, _)| known(k.trim())))
  }

  // written half a second after the last change, off the ui thread (a zoom wheel is a change per notch);
  // `store::flush` on the way out writes it at once
  pub fn save(&self) {
    store::write_later("settings", &self.render());
  }
}

// slash commands handled here, never sent to a platform
#[derive(PartialEq, Eq, Debug)]
pub enum Local {
  // a switch from the schema table, by id (`/vi`, `/glow`, `/hidebots`...)
  Toggle(&'static str),
  // `/zoom 1.5`; None = no usable number
  Zoom(Option<u16>),
  // `/tabs left`; None = no such side
  Side(Option<Side>),
  // `/tabcounter heat`; None = no such mode
  Counter(Option<Counter>),
  // `/emotesize 2x`: a row by id and the value's place in its names; None = no such value
  Pick(&'static str, Option<usize>),
  // `/modbuttons x 1m ban`: a button row by id and what was typed after it
  Buttons(&'static str, String),
  // `/hide feed`, `/show mentions`; None = no such tab
  Tab(Option<Kind>, bool),
  // the settings screen, the key map
  Settings,
  // /reset: every look and behaviour back to its default (asks first)
  Reset,
  Keys,
  // /login [platform], /logout, /account: who you are
  Login(String),
  Logout,
  Account,
  // /following, /live: who you follow that is live
  Following,
  // /import: your twitch follows, picked into tabs
  Import,
  // /emotes: your heatsync emotes, managed
  Emotes,
  // /blocked: the twitch room's blocked terms
  Blocked,
  // /summary: the tab's stream stats
  Summary,
  // /status: the twitch channel's stream and chat modes
  Status,
  // /predictions: the twitch channel's prediction
  Pred,
  // /modtools: shield, automod, unban requests, chatters
  ModTools,
  // /lclear: empty this tab's lines (here only)
  Lclear,
  // /user <name>: their card
  User(String),
  // /tab <name>: go to a tab
  Goto(String),
  // /set <setting> <value>
  Set(String, String),
  // /follow <name> (true), /unfollow <name>
  Follow(String, bool),
}

pub fn local_cmd(text: &str) -> Option<Local> {
  let low = text.trim().to_ascii_lowercase();
  if let Some(rest) = low.strip_prefix("/zoom")
    && (rest.is_empty() || rest.starts_with(char::is_whitespace))
  {
    let z = rest.trim().trim_end_matches('%').parse::<f32>().ok().filter(|z| z.is_finite() && *z > 0.0);
    // 1.5 and 150 both mean 150%
    return Some(Local::Zoom(z.map(|z| clamp_zoom(if z < 10.0 { (z * 100.0).round() } else { z.round() } as i32))));
  }
  if let Some(rest) = low.strip_prefix("/tabs")
    && (rest.is_empty() || rest.starts_with(char::is_whitespace))
  {
    return Some(Local::Side(Side::parse(rest)));
  }
  if let Some(rest) = low.strip_prefix("/tabcounter")
    && (rest.is_empty() || rest.starts_with(char::is_whitespace))
  {
    return Some(Local::Counter(Counter::parse(rest)));
  }
  let (word, arg) = low.split_once(char::is_whitespace).unwrap_or((&low, ""));
  if let Some(d) = schema::by_cmd(word)
    && let Ctl::Pick(c, ..) = d.ctl
  {
    return Some(Local::Pick(d.id, c.names.iter().position(|n| *n == arg.trim())));
  }
  if let Some(d) = schema::by_cmd(word)
    && matches!(d.ctl, Ctl::Buttons(..))
  {
    return Some(Local::Buttons(d.id, arg.trim().to_owned()));
  }
  for (word, on) in [("/hide", false), ("/show", true)] {
    if let Some(rest) = low.strip_prefix(word)
      && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
      let kind = Kind::parse(rest);
      // `/hide bob` is the filter (hide a person in this tab), not a tab switch
      if word == "/hide" && kind.is_none() && !rest.trim().is_empty() {
        return None;
      }
      return Some(Local::Tab(kind, on));
    }
  }
  // the rest keep their case: names and values
  let (head, tail) = text.trim().split_once(char::is_whitespace).unwrap_or((text.trim(), ""));
  let tail = tail.trim();
  match head.to_ascii_lowercase().as_str() {
    "/user" => return Some(Local::User(tail.to_owned())),
    "/follow" => return Some(Local::Follow(tail.to_owned(), true)),
    "/unfollow" => return Some(Local::Follow(tail.to_owned(), false)),
    "/tab" => return Some(Local::Goto(tail.to_owned())),
    "/login" => return Some(Local::Login(tail.to_ascii_lowercase())),
    "/set" => {
      let (k, v) = tail.split_once(char::is_whitespace).unwrap_or((tail, ""));
      return Some(Local::Set(k.to_owned(), v.trim().to_owned()));
    }
    _ => {}
  }
  match low.as_str() {
    "/settings" => Some(Local::Settings),
    "/reset" => Some(Local::Reset),
    "/keys" | "/help" => Some(Local::Keys),
    "/logout" => Some(Local::Logout),
    "/account" => Some(Local::Account),
    "/following" | "/live" => Some(Local::Following),
    "/import" => Some(Local::Import),
    "/emotes" => Some(Local::Emotes),
    "/lclear" => Some(Local::Lclear),
    "/blocked" => Some(Local::Blocked),
    "/summary" => Some(Local::Summary),
    "/status" | "/modes" => Some(Local::Status),
    "/predictions" => Some(Local::Pred),
    "/modtools" | "/unbanrequests" => Some(Local::ModTools),
    w => schema::switch_for(w).map(|d| Local::Toggle(d.id)),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn defaults() {
    assert_eq!(
      Settings::default(),
      Settings {
        zoom: 100,
        tab_side: Side::Top,
        tab_counter: Counter::Unread,
        vi: false,
        auto_hide: false,
        timestamps: false,
        time_fmt: TimeFmt::H24,
        zebra: true,
        readable: true,
        platform_tags: true,
        avatars: false,
        emote_size: EmoteSize::X1,
        emoji_size: EmoteSize::X2,
        line_height: LineHeight::L20,
        animate_emotes: Animate::Always,
        dim_timeouts: true,
        first_glow: true,
        read_line: true,
        paints: true,
        hs_paints: true,
        third_party_badges: true,
        sub_flair: true,
        flame: true,
        heat_bars: true,
        text_badges: false,
        pronouns: true,
        stream_stats: true,
        paint_motion: true,
        ev_online: true,
        ev_offline: false,
        ev_game: true,
        ev_raid: true,
        ev_sub: true,
        ev_redeem: true,
        ev_pred: true,
        moments: true,
        inline_op: true,
        inline_mop: true,
        inline_re: true,
        whisper_toast: true,
        inline_dm: false,
        media: true,
        links: true,
        render_inventory: true,
        suggest_inventory: true,
        tab_feed: true,
        tab_whispers: true,
        tab_mentions: true,
        tab_modlog: false,
        notify: false,
        title_flash: true,
        mention_volume: Volume::V30,
        streamer: Streamer::Off,
        streamer_sounds: false,
        updates: true,
        chat_logs: false,
        hide_bots: false,
        hide_cmds: false,
        hide_dupes: false,
        confirm_ban: false,
        mod_buttons: 0,
        show_nsfw: false,
        show_violence: false,
        show_weapons: false,
        show_drugs: false,
        show_hate: false
      }
    );
    assert_eq!(Settings::parse(""), Settings::default());
    assert_eq!(Settings::parse("garbage\nvi\n=true\nvi=maybe"), Settings::default());
  }

  // the old "normal" preset left zebra, tags, paints, stats and emoji size as they were: a reset must not
  #[test]
  fn reset_puts_back_every_saved_setting_but_the_tabs_on_the_bar() {
    let mut s = Settings::default();
    for d in &schema::DEFS {
      match d.ctl {
        Ctl::Pill(get, set) => {
          let v = !get(&s);
          set(&mut s, v)
        }
        Ctl::Pick(c, get, set) => {
          let v = (get(&s) + 1) % c.names.len();
          set(&mut s, v)
        }
        Ctl::Buttons(_, set) => set(&mut s, 0b101),
        _ => {}
      }
    }
    (s.zoom, s.tab_side, s.tab_counter) = (150, Side::Left, Counter::Heat);
    // the test really moved every line the file keeps
    let (was, now) = (Settings::default().render(), s.render());
    for (a, b) in was.lines().zip(now.lines()) {
      assert_ne!(a, b, "the test left {a} alone");
    }
    let mut want = Settings::default();
    (want.tab_feed, want.tab_whispers, want.tab_mentions, want.tab_modlog) =
      (s.tab_feed, s.tab_whispers, s.tab_mentions, s.tab_modlog);
    assert_eq!(s.reset(), want);
    // and the same through the file
    assert_eq!(Settings::parse(&s.reset().render()), want);
  }

  // bttv, ffz and chatterino badges were three switches: the one that replaces them is on if any was, once, and
  // the old lines are never read again
  #[test]
  fn the_three_badge_switches_become_one_that_is_on_if_any_was() {
    let all = |a: bool, b: bool, c: bool| format!("vi=true\nbttv_badges={a}\nffz_badges={b}\nchatterino_badges={c}\n");
    assert!(Settings::parse(&all(false, true, false)).third_party_badges);
    assert!(Settings::parse(&all(false, false, true)).third_party_badges);
    let off = Settings::parse(&all(false, false, false));
    assert!(!off.third_party_badges && off.vi, "all three off: off, and the rest of the file is kept");
    // run twice: what is written back holds the one switch, and loading that changes nothing
    let back = Settings::parse(&off.render());
    assert_eq!(back, off);
    assert!(off.render().contains("third_party_badges=false") && !off.render().contains("bttv_badges"));
    // a file with the one switch wins over leftover old lines (an older and a newer run sharing a folder)
    assert!(!Settings::parse("third_party_badges=false\nbttv_badges=true\n").third_party_badges);
    assert!(Settings::parse("bttv_badges=false\nthird_party_badges=true\n").third_party_badges);
    // none of the four: on, the default
    assert!(Settings::parse("vi=true\n").third_party_badges);
  }

  #[test]
  fn roundtrip() {
    for (vi, auto_hide, first_glow, paints, zoom) in [
      (false, false, true, true, 100),
      (true, false, false, false, 130),
      (false, true, true, false, 90),
      (true, true, false, true, 300),
    ] {
      let s = Settings { zoom, vi, auto_hide, first_glow, paints, ev_offline: vi, ev_game: !vi, ..Settings::default() };
      assert_eq!(Settings::parse(&s.render()), s);
    }
    // unknown keys and spacing are tolerated
    assert_eq!(Settings::parse("future=1\n vi = true \n"), Settings { vi: true, ..Settings::default() });
  }

  // the all-caps switch is a filter rule now: on, it becomes one hide rule, once; off, it is just dropped
  #[test]
  fn the_old_caps_switch_moves_into_the_rules_once() {
    store::write("settings", "vi=true\nautomod_caps=true\n").unwrap();
    assert!(Settings::load().vi);
    assert!(!store::read("settings").unwrap().contains("automod_caps"));
    let rules = crate::model::filter::Filters::load();
    assert_eq!(rules.rules().len(), 1);
    // loaded again, nothing is left to move
    Settings::load();
    assert_eq!(crate::model::filter::Filters::load().rules().len(), 1);
    store::write("settings", "vi=true\nautomod_caps=false\n").unwrap();
    store::write("filters", "").unwrap();
    Settings::load();
    assert_eq!(crate::model::filter::Filters::load().rules().len(), 0);
  }

  // the store's scratch folder is per test thread, so this never sees the real settings
  #[test]
  fn a_file_that_is_not_settings_is_set_aside_not_dropped() {
    store::write("settings", "this is not\na settings file\n").unwrap();
    assert_eq!(Settings::load(), Settings::default());
    let said = store::take_notes();
    assert_eq!(said, ["your settings file was unreadable; saved a copy as settings.bak and started fresh"]);
    assert_eq!(store::read("settings.bak").as_deref(), Some("this is not\na settings file\n"));
    assert!(store::read("settings").is_none(), "moved, not copied");
    // bytes that are not text at all
    std::fs::write(store::path_of("settings").unwrap(), [0xde, 0xad, 0xbe, 0xef, b'\n', 0x00]).unwrap();
    assert_eq!(Settings::load(), Settings::default());
    assert_eq!(store::take_notes().len(), 1);
    // a real file, an empty one and a missing one are not news
    Settings { vi: true, ..Settings::default() }.save();
    assert!(Settings::load().vi);
    store::write("settings", "  \n").unwrap();
    assert_eq!(Settings::load(), Settings::default());
    store::remove("settings");
    assert_eq!(Settings::load(), Settings::default());
    assert!(store::take_notes().is_empty());
  }

  #[test]
  fn tab_side_parses_saves_and_steps() {
    assert_eq!(Settings::parse("tab_side=left").tab_side, Side::Left);
    assert_eq!(Settings::parse("tab_side=sideways").tab_side, Side::Top);
    let s = Settings { tab_side: Side::Bottom, ..Settings::default() };
    assert_eq!(Settings::parse(&s.render()), s);
    assert_eq!(Side::Top.step(false), Side::Left);
    assert_eq!(Side::Left.step(true), Side::Top);
    assert_eq!(local_cmd("/tabs Right"), Some(Local::Side(Some(Side::Right))));
    assert_eq!(local_cmd("/tabs"), Some(Local::Side(None)));
    assert_eq!(Settings::parse("tab_counter=heat").tab_counter, Counter::Heat);
    assert_eq!(Settings::default().tab_counter, Counter::Unread);
    let s = Settings { tab_counter: Counter::Off, ..Settings::default() };
    assert_eq!(Settings::parse(&s.render()).tab_counter, Counter::Off);
    assert_eq!(Counter::Off.step(false), Counter::Heat);
    assert_eq!(local_cmd("/tabcounter heat"), Some(Local::Counter(Some(Counter::Heat))));
    assert_eq!(local_cmd("/tabsy"), None);
  }

  #[test]
  fn chat_look_defaults_follow_the_ext() {
    let d = Settings::default();
    assert!(!d.timestamps && d.zebra && d.readable && d.platform_tags && d.dim_timeouts);
    assert_eq!(
      (d.time_fmt, d.emote_size, d.line_height, d.animate_emotes),
      (TimeFmt::H24, EmoteSize::X1, LineHeight::L20, Animate::Always)
    );
  }

  #[test]
  fn chat_look_switches_parse_and_save() {
    let s = Settings::parse(
      "timestamps=true\nzebra=false\nreadable=false\nplatform_tags=false\ndim_timeouts=false\n\
       time_fmt=12h\nemote_size=4x\nline_height=26\nanimate_emotes=never\n",
    );
    assert!(s.timestamps && !s.zebra && !s.readable && !s.platform_tags && !s.dim_timeouts);
    assert_eq!(
      (s.time_fmt, s.emote_size, s.line_height, s.animate_emotes),
      (TimeFmt::H12, EmoteSize::X4, LineHeight::L26, Animate::Never)
    );
    assert_eq!(Settings::parse(&s.render()), s);
    // a value that isn't one of the names leaves the default
    let bad = Settings::parse("time_fmt=9h\nemote_size=3x\nline_height=21\nanimate_emotes=hover");
    assert_eq!(bad, Settings::default());
    assert_eq!((EmoteSize::X4.mul(), EmoteSize::X1.mul()), (4, 1));
    assert!((LineHeight::L22.ratio() - 1.1).abs() < 1e-6 && LineHeight::L20.ratio() == 1.0);
    assert_eq!(TimeFmt::CHOICE.usage, "<24h|12h>");
    assert_eq!(Animate::CHOICE.usage, "<always|never>");
  }

  #[test]
  fn text_badges_round_trip() {
    assert!(!Settings::default().text_badges);
    let s = Settings::parse("text_badges=true\n");
    assert!(s.text_badges);
    assert_eq!(Settings::parse(&s.render()), s);
    assert_eq!(local_cmd("/textbadges"), Some(Local::Toggle("text_badges")));
  }

  // two settings are gone (the emote provider pick, partial links): a file from before still loads, keeps every
  // other setting, is written back without them, and their commands are no longer commands
  #[test]
  fn the_dropped_provider_and_partial_link_settings_are_ignored_quietly() {
    let old = "vi=true\nemote_provider=bttv\npartial_links=false\nzebra=false\n";
    let s = Settings::parse(old);
    assert!(s.vi && !s.zebra);
    assert_eq!(s, Settings { vi: true, zebra: false, ..Settings::default() });
    assert!(!s.render().contains("emote_provider") && !s.render().contains("partial_links"));
    assert_eq!(local_cmd("/emoteprovider bttv"), None);
    assert_eq!(local_cmd("/partiallinks"), None);
  }

  #[test]
  fn chat_look_commands() {
    assert_eq!(local_cmd("/timestamps"), Some(Local::Toggle("timestamps")));
    assert_eq!(local_cmd("/zebra"), Some(Local::Toggle("zebra")));
    assert_eq!(local_cmd("/readable"), Some(Local::Toggle("readable")));
    assert_eq!(local_cmd("/platformtags"), Some(Local::Toggle("platform_tags")));
    assert_eq!(local_cmd("/dimtimeouts"), Some(Local::Toggle("dim_timeouts")));
    assert_eq!(local_cmd("/timeformat 12h"), Some(Local::Pick("time_fmt", Some(1))));
    assert_eq!(local_cmd("/emotesize 4X"), Some(Local::Pick("emote_size", Some(2))));
    assert_eq!(local_cmd("/lineheight 22"), Some(Local::Pick("line_height", Some(1))));
    assert_eq!(local_cmd("/animateemotes never"), Some(Local::Pick("animate_emotes", Some(1))));
    assert_eq!(local_cmd("/animateemotes hover"), Some(Local::Pick("animate_emotes", None)));
    assert_eq!(local_cmd("/emotesize"), Some(Local::Pick("emote_size", None)));
  }

  #[test]
  fn zoom_clamps_steps_and_parses() {
    assert_eq!(clamp_zoom(0), 90);
    assert_eq!(clamp_zoom(1000), 300);
    assert_eq!(clamp_zoom(130), 130);
    assert_eq!(Settings::parse("zoom=9").zoom, 300);
    assert_eq!(Settings::parse("zoom=0.1").zoom, 90);
    assert_eq!(Settings::parse("zoom=0.5").zoom, 90);
    assert_eq!(Settings::parse("zoom=nan").zoom, 100);
    assert_eq!(Settings::parse("zoom=junk").zoom, 100);
    assert_eq!(Settings::parse("zoom=1.3").zoom, 130);
    assert!((Settings { zoom: 130, ..Settings::default() }.scale() - 1.3).abs() < 1e-6);
    // ten steps up from native never drifts off the 10% grid
    let mut z = 100;
    for _ in 0..10 {
      z = clamp_zoom(z as i32 + ZOOM_STEP as i32);
    }
    assert_eq!(z, 200);
    assert_eq!(local_cmd("/zoom 1.5"), Some(Local::Zoom(Some(150))));
    assert_eq!(local_cmd("/zoom 150%"), Some(Local::Zoom(Some(150))));
    assert_eq!(local_cmd("/zoom 99"), Some(Local::Zoom(Some(99))));
    assert_eq!(local_cmd("/zoom 9"), Some(Local::Zoom(Some(300))));
    assert_eq!(local_cmd("/zoom"), Some(Local::Zoom(None)));
    assert_eq!(local_cmd("/zoom abc"), Some(Local::Zoom(None)));
    assert_eq!(local_cmd("/zoomer"), None);
  }

  #[test]
  fn tab_and_notify_commands() {
    assert_eq!(local_cmd("/hide feed"), Some(Local::Tab(Some(Kind::Feed), false)));
    assert_eq!(local_cmd("/SHOW  Mentions"), Some(Local::Tab(Some(Kind::Mentions), true)));
    assert_eq!(local_cmd("/hide"), Some(Local::Tab(None, false)));
    // a name after /hide is the user filter, handled elsewhere
    assert_eq!(local_cmd("/hide bob"), None);
    assert_eq!(local_cmd("/show nope"), Some(Local::Tab(None, true)));
    assert_eq!(local_cmd("/hidden"), None);
    assert_eq!(local_cmd("/notify"), Some(Local::Toggle("notify")));
    let mut s = Settings::default();
    s.show(Kind::Whispers, false);
    s.notify = true;
    let back = Settings::parse(&s.render());
    assert!(!back.shown(Kind::Whispers) && back.shown(Kind::Feed) && back.notify);
    assert!(back.shown(Kind::Chat));
  }

  // the notifs tab is gone: a settings file from when it was there loads clean, keeps every other
  // setting, and is written back without it. /show notifs is no tab, just the usage line
  #[test]
  fn a_saved_notifs_tab_is_ignored() {
    let old = "vi=true\ntab_notifs=false\ntab_whispers=false\nzoom=1.5\nnotifs=true\n";
    assert!(Settings::usable(old));
    let s = Settings::parse(old);
    assert!(s.vi && !s.shown(Kind::Whispers) && s.shown(Kind::Mentions) && s.zoom > 100);
    assert!(!s.render().contains("notifs"));
    assert!(!Settings::usable("tab_notifs=true\n"), "a file of only that names nothing of ours");
    assert_eq!(Kind::parse("notifs"), None);
    assert_eq!(local_cmd("/show notifs"), Some(Local::Tab(None, true)));
    assert_eq!(local_cmd("/hide notifs"), None, "a name after /hide is the user filter");
  }

  #[test]
  fn slash_commands() {
    assert_eq!(local_cmd("/vi"), Some(Local::Toggle("vi")));
    assert_eq!(local_cmd("  /AutoHide "), Some(Local::Toggle("auto_hide")));
    assert_eq!(local_cmd("/vi now"), None);
    assert_eq!(local_cmd("vi"), None);
    assert_eq!(local_cmd("/me waves"), None);
    assert_eq!(local_cmd("/hidebots"), Some(Local::Toggle("hide_bots")));
    assert_eq!(local_cmd("/settings"), Some(Local::Settings));
    assert_eq!(local_cmd("/KEYS"), Some(Local::Keys));
    assert_eq!(local_cmd("/login"), Some(Local::Login(String::new())));
    assert_eq!(local_cmd("/login  Kick "), Some(Local::Login("kick".into())));
    assert_eq!(local_cmd("/logout"), Some(Local::Logout));
    assert_eq!(local_cmd("/ACCOUNT"), Some(Local::Account));
    assert_eq!(local_cmd("/Following"), Some(Local::Following));
    assert_eq!(local_cmd("/live"), Some(Local::Following));
    assert_eq!(local_cmd("/Import"), Some(Local::Import));
    // every switch in the table answers to its own command
    for d in &schema::DEFS {
      if let (Some(c), Ctl::Pill(..)) = (d.cmd, d.ctl) {
        assert_eq!(local_cmd(c), Some(Local::Toggle(d.id)));
      }
    }
  }
}
