// user prefs, persisted as `settings` (key=value lines). vi + auto_hide default off: a plain chat box
use crate::heatsync::content::Cat;
use crate::model::schema;
use crate::model::tabs::Kind;
use crate::store;

pub const ZOOM_MIN: u16 = 50;
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
  const ALL: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

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
  const ALL: [Counter; 3] = [Counter::Off, Counter::Unread, Counter::Heat];

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
  // orange edge on someone's first line of the session. default on, like the ext
  pub first_glow: bool,
  // 7tv paints on names and 7tv badges. default on, like the ext (sevenTvPaints)
  pub paints: bool,
  // name paints move (scenes + effects). off freezes each to its first frame, like the ext's animatePaints
  pub paint_motion: bool,
  // stream lines: went live (on), went offline (off), game switch (on), chat-spike line (on)
  pub ev_online: bool,
  pub ev_offline: bool,
  pub ev_game: bool,
  pub moments: bool,
  // heatsync feed posts as lines in every chat tab (ext inlineNotifs): new threads, the OP answering in
  // their own thread, replies. all on, like the ext
  pub inline_op: bool,
  pub inline_mop: bool,
  pub inline_re: bool,
  // inline pictures under chat lines. default on, like the ext (mediaEmbedsEnabled)
  pub media: bool,
  // host-less and defanged links (watch?v=…, r/sub/comments/…, site (dot) com) are clickable. default on, like the ext (partialLinksEnabled)
  pub partial_links: bool,
  // built-in tabs on the bar (all on, like the ext once you are signed in; /hide and /show)
  pub tab_feed: bool,
  pub tab_whispers: bool,
  pub tab_mentions: bool,
  // a desktop notification for an @mention while the window is not focused. default off, like the
  // ext's hs_notifications: nothing leaves the window unless asked
  pub notify: bool,
  // look for a newer release now and then (github sees the ask). default on
  pub updates: bool,
  // chat filters (ext hideBots, hideCommands, hideDuplicates): all off by default
  pub hide_bots: bool,
  pub hide_cmds: bool,
  pub hide_dupes: bool,
  // ask before a ban goes out (the ext's modConfirmBan, off by default like the ext)
  pub confirm_ban: bool,
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
      first_glow: true,
      paints: true,
      paint_motion: true,
      ev_online: true,
      ev_offline: false,
      ev_game: true,
      moments: true,
      inline_op: true,
      inline_mop: true,
      inline_re: true,
      media: true,
      partial_links: true,
      tab_feed: true,
      tab_whispers: true,
      tab_mentions: true,
      notify: false,
      updates: true,
      hide_bots: false,
      hide_cmds: false,
      hide_dupes: false,
      confirm_ban: false,
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

  pub fn shown(&self, k: Kind) -> bool {
    match k {
      Kind::Chat => true,
      Kind::Feed => self.tab_feed,
      Kind::Whispers => self.tab_whispers,
      Kind::Mentions => self.tab_mentions,
    }
  }

  pub fn show(&mut self, k: Kind, on: bool) {
    match k {
      Kind::Chat => {}
      Kind::Feed => self.tab_feed = on,
      Kind::Whispers => self.tab_whispers = on,
      Kind::Mentions => self.tab_mentions = on,
    }
  }

  pub fn parse(s: &str) -> Self {
    let mut out = Self::default();
    for l in s.lines() {
      let Some((k, v)) = l.split_once('=') else { continue };
      let on = v.trim() == "true";
      let k = k.trim();
      if schema::def(k).is_some_and(|d| d.put(&mut out, on)) {
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
    out
  }

  pub fn render(&self) -> String {
    let mut out = String::with_capacity(256);
    for d in &schema::DEFS {
      if let Some(on) = d.on(self) {
        out.push_str(&format!("{}={on}\n", d.id));
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

  pub fn load() -> Self {
    store::read("settings").map(|s| Self::parse(&s)).unwrap_or_default()
  }

  pub fn save(&self) {
    if let Err(e) = store::write("settings", &self.render()) {
      eprintln!("save settings: {e}");
    }
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
  // `/hide feed`, `/show mentions`; None = no such tab
  Tab(Option<Kind>, bool),
  // the settings screen, the key map
  Settings,
  Keys,
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
  match low.as_str() {
    "/settings" => Some(Local::Settings),
    "/keys" => Some(Local::Keys),
    w => schema::switch_for(w).map(|d| Local::Toggle(d.id)),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::schema::Ctl;

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
        first_glow: true,
        paints: true,
        paint_motion: true,
        ev_online: true,
        ev_offline: false,
        ev_game: true,
        moments: true,
        inline_op: true,
        inline_mop: true,
        inline_re: true,
        media: true,
        partial_links: true,
        tab_feed: true,
        tab_whispers: true,
        tab_mentions: true,
        notify: false,
        updates: true,
        hide_bots: false,
        hide_cmds: false,
        hide_dupes: false,
        confirm_ban: false,
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

  #[test]
  fn roundtrip() {
    for (vi, auto_hide, first_glow, paints, zoom) in [
      (false, false, true, true, 100),
      (true, false, false, false, 130),
      (false, true, true, false, 50),
      (true, true, false, true, 300),
    ] {
      let s = Settings { zoom, vi, auto_hide, first_glow, paints, ev_offline: vi, ev_game: !vi, ..Settings::default() };
      assert_eq!(Settings::parse(&s.render()), s);
    }
    // unknown keys and spacing are tolerated
    assert_eq!(Settings::parse("future=1\n vi = true \n"), Settings { vi: true, ..Settings::default() });
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
  fn zoom_clamps_steps_and_parses() {
    assert_eq!(clamp_zoom(0), 50);
    assert_eq!(clamp_zoom(1000), 300);
    assert_eq!(clamp_zoom(130), 130);
    assert_eq!(Settings::parse("zoom=9").zoom, 300);
    assert_eq!(Settings::parse("zoom=0.1").zoom, 50);
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
    // every switch in the table answers to its own command
    for d in &schema::DEFS {
      if let (Some(c), Ctl::Pill(..)) = (d.cmd, d.ctl) {
        assert_eq!(local_cmd(c), Some(Local::Toggle(d.id)));
      }
    }
  }
}
