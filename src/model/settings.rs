// user prefs, persisted as `settings` (key=value lines). vi + auto_hide default off: a plain chat box
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
}

impl Default for Settings {
  fn default() -> Self {
    Settings { zoom: 100, vi: false, auto_hide: false, first_glow: true, paints: true }
  }
}

impl Settings {
  // the multiplier on the window's scale factor
  pub fn scale(&self) -> f32 {
    self.zoom as f32 / 100.0
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
      "vi={}\nauto_hide={}\nfirst_glow={}\npaints={}\nzoom={}\n",
      self.vi,
      self.auto_hide,
      self.first_glow,
      self.paints,
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
  // `/zoom 1.5`; None = no usable number
  Zoom(Option<u16>),
}

pub const LOCAL: [&str; 5] = ["/vi", "/autohide", "/glow", "/paints", "/zoom"];

pub fn local_cmd(text: &str) -> Option<Local> {
  let low = text.trim().to_ascii_lowercase();
  if let Some(rest) = low.strip_prefix("/zoom")
    && (rest.is_empty() || rest.starts_with(char::is_whitespace))
  {
    let z = rest.trim().trim_end_matches('%').parse::<f32>().ok().filter(|z| z.is_finite() && *z > 0.0);
    // 1.5 and 150 both mean 150%
    return Some(Local::Zoom(z.map(|z| clamp_zoom(if z < 10.0 { (z * 100.0).round() } else { z.round() } as i32))));
  }
  match low.as_str() {
    "/vi" => Some(Local::Vi),
    "/autohide" => Some(Local::AutoHide),
    "/glow" => Some(Local::Glow),
    "/paints" => Some(Local::Paints),
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
      Settings { zoom: 100, vi: false, auto_hide: false, first_glow: true, paints: true }
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
      let s = Settings { zoom, vi, auto_hide, first_glow, paints };
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
  fn slash_commands() {
    assert_eq!(local_cmd("/vi"), Some(Local::Vi));
    assert_eq!(local_cmd("  /AutoHide "), Some(Local::AutoHide));
    assert_eq!(local_cmd("/vi now"), None);
    assert_eq!(local_cmd("vi"), None);
    assert_eq!(local_cmd("/me waves"), None);
  }
}
