// user prefs, persisted as `settings` (key=value lines). vi + auto_hide default off: a plain chat box
use crate::model::tabs::Kind;
use crate::store;

pub const ZOOM_MIN: u16 = 50;
pub const ZOOM_MAX: u16 = 300;
pub const ZOOM_STEP: u16 = 10;

// percent, kept whole so repeated steps never drift
pub fn clamp_zoom(pct: i32) -> u16 {
  pct.clamp(ZOOM_MIN as i32, ZOOM_MAX as i32) as u16
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
  // everything's size, percent of the display scale (100 = native)
  pub zoom: u16,
  // vim keys in the message box + chat-driving mode
  pub vi: bool,
  // the input bar takes no room while empty and unused
  pub auto_hide: bool,
  // orange edge on someone's first line of the session. default on, like the ext
  pub first_glow: bool,
  // 7tv paints on names and 7tv badges. default on, like the ext (sevenTvPaints)
  pub paints: bool,
  // stream lines: went live (on), went offline (off), game switch (on), chat-spike line (on)
  pub ev_online: bool,
  pub ev_offline: bool,
  pub ev_game: bool,
  pub moments: bool,
  // inline pictures under chat lines. default on, like the ext (mediaEmbedsEnabled)
  pub media: bool,
  // built-in tabs on the bar (all on, like the ext once you are signed in; /hide and /show)
  pub tab_feed: bool,
  pub tab_whispers: bool,
  pub tab_mentions: bool,
  // a desktop notification for an @mention while the window is not focused. default off, like the
  // ext's hs_notifications: nothing leaves the window unless asked
  pub notify: bool,
  // chat filters (ext hideBots, hideCommands, hideDuplicates): all off by default
  pub hide_bots: bool,
  pub hide_cmds: bool,
  pub hide_dupes: bool,
  // ask before a ban goes out (the ext's modConfirmBan, off by default like the ext)
  pub confirm_ban: bool,
}

impl Default for Settings {
  fn default() -> Self {
    Settings {
      zoom: 100,
      vi: false,
      auto_hide: false,
      first_glow: true,
      paints: true,
      ev_online: true,
      ev_offline: false,
      ev_game: true,
      moments: true,
      media: true,
      tab_feed: true,
      tab_whispers: true,
      tab_mentions: true,
      notify: false,
      hide_bots: false,
      hide_cmds: false,
      hide_dupes: false,
      confirm_ban: false,
    }
  }
}

impl Settings {
  // the multiplier on the window's scale factor
  pub fn scale(&self) -> f32 {
    self.zoom as f32 / 100.0
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
      match k.trim() {
        "vi" => out.vi = on,
        "auto_hide" => out.auto_hide = on,
        "first_glow" => out.first_glow = on,
        "paints" => out.paints = on,
        "ev_online" => out.ev_online = on,
        "ev_offline" => out.ev_offline = on,
        "ev_game" => out.ev_game = on,
        "moments" => out.moments = on,
        "media" => out.media = on,
        "tab_feed" => out.tab_feed = on,
        "tab_whispers" => out.tab_whispers = on,
        "tab_mentions" => out.tab_mentions = on,
        "notify" => out.notify = on,
        "confirm_ban" => out.confirm_ban = on,
        "hide_bots" => out.hide_bots = on,
        "hide_cmds" => out.hide_cmds = on,
        "hide_dupes" => out.hide_dupes = on,
        "zoom" => {
          if let Some(z) = v.trim().parse::<f32>().ok().filter(|z| z.is_finite()) {
            out.zoom = clamp_zoom((z * 100.0).round() as i32);
          }
        }
        _ => {}
      }
    }
    out
  }

  pub fn render(&self) -> String {
    format!(
      "vi={}\nauto_hide={}\nfirst_glow={}\npaints={}\nev_online={}\nev_offline={}\nev_game={}\nmoments={}\nmedia={}\ntab_feed={}\ntab_whispers={}\ntab_mentions={}\nnotify={}\nconfirm_ban={}\nhide_bots={}\nhide_cmds={}\nhide_dupes={}\nzoom={}\n",
      self.vi,
      self.auto_hide,
      self.first_glow,
      self.paints,
      self.ev_online,
      self.ev_offline,
      self.ev_game,
      self.moments,
      self.media,
      self.tab_feed,
      self.tab_whispers,
      self.tab_mentions,
      self.notify,
      self.confirm_ban,
      self.hide_bots,
      self.hide_cmds,
      self.hide_dupes,
      self.scale()
    )
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
  Vi,
  AutoHide,
  Glow,
  Paints,
  // stream lines: went live, went offline, game switch; the chat-spike line
  Online,
  Offline,
  Game,
  Moments,
  // inline pictures
  Media,
  // `/zoom 1.5`; None = no usable number
  Zoom(Option<u16>),
  // desktop notification on an @mention
  Notify,
  // ask before banning
  ConfirmBan,
  // `/hide feed`, `/show mentions`; None = no such tab
  Tab(Option<Kind>, bool),
}

pub const LOCAL: [&str; 14] = [
  "/vi",
  "/autohide",
  "/glow",
  "/paints",
  "/online",
  "/offline",
  "/game",
  "/moments",
  "/media",
  "/zoom",
  "/notify",
  "/confirmban",
  "/hide",
  "/show",
];

pub fn local_cmd(text: &str) -> Option<Local> {
  let low = text.trim().to_ascii_lowercase();
  if let Some(rest) = low.strip_prefix("/zoom")
    && (rest.is_empty() || rest.starts_with(char::is_whitespace))
  {
    let z = rest.trim().trim_end_matches('%').parse::<f32>().ok().filter(|z| z.is_finite() && *z > 0.0);
    // 1.5 and 150 both mean 150%
    return Some(Local::Zoom(z.map(|z| clamp_zoom(if z < 10.0 { (z * 100.0).round() } else { z.round() } as i32))));
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
    "/notify" => Some(Local::Notify),
    "/confirmban" => Some(Local::ConfirmBan),
    "/vi" => Some(Local::Vi),
    "/autohide" => Some(Local::AutoHide),
    "/glow" => Some(Local::Glow),
    "/paints" => Some(Local::Paints),
    "/online" => Some(Local::Online),
    "/offline" => Some(Local::Offline),
    "/game" => Some(Local::Game),
    "/moments" => Some(Local::Moments),
    "/media" => Some(Local::Media),
    _ => None,
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
        vi: false,
        auto_hide: false,
        first_glow: true,
        paints: true,
        ev_online: true,
        ev_offline: false,
        ev_game: true,
        moments: true,
        media: true,
        tab_feed: true,
        tab_whispers: true,
        tab_mentions: true,
        notify: false,
        hide_bots: false,
        hide_cmds: false,
        hide_dupes: false,
        confirm_ban: false
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
    assert_eq!(local_cmd("/notify"), Some(Local::Notify));
    let mut s = Settings::default();
    s.show(Kind::Whispers, false);
    s.notify = true;
    let back = Settings::parse(&s.render());
    assert!(!back.shown(Kind::Whispers) && back.shown(Kind::Feed) && back.notify);
    assert!(back.shown(Kind::Chat));
  }

  #[test]
  fn slash_commands() {
    assert_eq!(local_cmd("/vi"), Some(Local::Vi));
    assert_eq!(local_cmd("  /AutoHide "), Some(Local::AutoHide));
    assert_eq!(local_cmd("/vi now"), None);
    assert_eq!(local_cmd("vi"), None);
    assert_eq!(local_cmd("/me waves"), None);
  }
}
