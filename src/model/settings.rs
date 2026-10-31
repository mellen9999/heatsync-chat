// user prefs, persisted as `settings` (key=value lines). vi + auto_hide default off: a plain chat box
use crate::model::schema::{self, Ctl};
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
  // host-less and defanged links (watch?v=…, r/sub/comments/…, site (dot) com) are clickable. default on, like the ext (partialLinksEnabled)
  pub partial_links: bool,
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
      partial_links: true,
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
      let k = k.trim();
      if schema::def(k).is_some_and(|d| d.put(&mut out, on)) {
        continue;
      }
      if k == "zoom"
        && let Some(z) = v.trim().parse::<f32>().ok().filter(|z| z.is_finite())
      {
        out.zoom = clamp_zoom((z * 100.0).round() as i32);
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
    out.push_str(&format!("zoom={}\n", self.scale()));
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
  // `/hide feed`, `/show mentions`; None = no such tab
  Tab(Option<Kind>, bool),
  // the settings screen, the key map
  Settings,
  Keys,
}

// every command this file answers, for tab completion
pub fn commands() -> Vec<&'static str> {
  let mut v: Vec<&'static str> =
    schema::DEFS.iter().filter(|d| matches!(d.ctl, Ctl::Pill(..) | Ctl::Zoom)).filter_map(|d| d.cmd).collect();
  v.extend(["/hide", "/show", "/settings", "/keys"]);
  v
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
        partial_links: true,
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
    assert!(commands().contains(&"/vi") && commands().contains(&"/keys") && !commands().contains(&"/hideword"));
  }
}
