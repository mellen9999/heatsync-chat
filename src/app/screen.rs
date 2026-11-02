// the settings screen (ctrl-, / /settings / the gear) and the key map (? / /keys): list screens over the
// chat built from model::schema and model::keys. j/k move, space or enter changes the row, / searches,
// esc closes. every change goes through `flip`, the same path the slash commands use
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::keys::KEYS;
use crate::model::line::Line;
use crate::model::schema::{self, Ctl, DEFS, Def, Fx, PRESETS, Preset};
use crate::model::settings::{Settings, ZOOM_MAX, ZOOM_STEP, clamp_zoom};
use crate::model::vi::Mode;
use crate::ui::canvas::Rect;
use crate::ui::sheet::{self, L, State};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
  Settings,
  Keys,
}

pub struct Screen {
  page: Page,
  st: State,
  q: Line,
  typing: bool,
  // the first g of gg
  chord: bool,
}

// what a row stands for
#[derive(Clone, Copy)]
enum Target {
  Def(&'static Def),
  Preset(&'static Preset),
  None,
}

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
    Screen { page, st: State::default(), q: Line::default(), typing: false, chord: false }
  }

  pub fn typing(&self) -> bool {
    self.typing
  }

  pub fn query_mut(&mut self) -> &mut Line {
    &mut self.q
  }

  fn lines(&self, set: &Settings) -> (Vec<L>, Vec<Target>) {
    match self.page {
      Page::Keys => keys_lines(),
      Page::Settings => settings_lines(self.q.text.trim(), set),
    }
  }
}

fn keys_lines() -> (Vec<L>, Vec<Target>) {
  let mut lines = Vec::new();
  for g in &KEYS {
    lines.push(L::Head(g.name.to_owned()));
    for (k, d) in g.rows {
      lines.push(L::Row { a: (*k).to_owned(), b: (*d).to_owned(), pill: None });
    }
  }
  let n = lines.len();
  (lines, vec![Target::None; n])
}

fn settings_lines(q: &str, set: &Settings) -> (Vec<L>, Vec<Target>) {
  let mut lines = Vec::new();
  let mut targets = Vec::new();
  let presets: Vec<&'static Preset> = PRESETS
    .iter()
    .filter(|p| {
      let hay = format!("preset {} {}", p.name, p.tip).to_lowercase();
      q.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
    })
    .collect();
  if !presets.is_empty() {
    lines.push(L::Head("presets".into()));
    targets.push(Target::None);
    for p in presets {
      lines.push(L::Row { a: p.name.to_owned(), b: String::new(), pill: Some(p.active(set)) });
      targets.push(Target::Preset(p));
    }
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
      Ctl::Words(marks) => {
        let n = crate::model::filter::with(|f| f.words(marks).len());
        (if n == 0 { "none".to_owned() } else { format!("{n} · enter adds") }, None)
      }
    };
    lines.push(L::Row { a: d.label.to_owned(), b, pill });
    targets.push(Target::Def(d));
  }
  (lines, targets)
}

impl App {
  // ---- changing a setting: the one path for slash commands, the screen and presets ----

  // what a flipped switch has to redo
  fn applied(&mut self, fx: Fx) {
    match fx {
      Fx::None => {}
      Fx::Vi => self.mode = Mode::Insert,
      Fx::Glow => {
        for v in self.tabs.iter_mut().flat_map(|t| t.views()) {
          v.glow = self.set.first_glow;
          v.invalidate();
        }
      }
      Fx::Paints => {
        self.res.cosmetics.on = self.set.paints;
        self.relayout_where(None);
      }
      Fx::PaintMotion => {
        self.res.arts.set_motion(self.set.paint_motion);
        self.relayout_where(None);
      }
      Fx::Links => {
        self.res.partial_links = self.set.partial_links;
        self.relayout_where(None);
      }
      Fx::Media => {
        self.res.media = self.set.media;
        self.relayout_where(None);
      }
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

  // flip a switch, save, say what it is now
  pub(super) fn flip(&mut self, d: &'static Def) {
    let Some(on) = d.on(&self.set).map(|v| !v) else { return };
    d.put(&mut self.set, on);
    self.applied(d.fx);
    self.set.save();
    self.say(format!("{} {}", d.label, if on { "on" } else { "off" }));
  }

  fn set_switch(&mut self, d: &'static Def, on: bool) {
    if d.on(&self.set).is_some_and(|v| v != on) {
      self.flip(d);
    }
  }

  fn apply_preset(&mut self, p: &'static Preset) {
    for (id, on) in p.diff {
      if let Some(d) = schema::def(id)
        && d.put(&mut self.set, *on)
      {
        self.applied(d.fx);
      }
    }
    self.set.save();
    self.say(format!("preset {}", p.name));
  }

  // ---- opening and closing ----

  pub(super) fn open_screen(&mut self, page: Page) {
    if self.picker.is_some() {
      self.close_picker();
    }
    self.close_prompt();
    self.menu = None;
    self.tab_menu = None;
    self.pop = None;
    self.card = None;
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

  pub(super) fn toggle_screen(&mut self, page: Page) {
    if self.screen.as_ref().is_some_and(|s| s.page == page) {
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
        Key::Character("/") if s.page == Page::Settings => s.typing = true,
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
        // a list lives in slash commands: leave the screen with the command ready to finish
        Ctl::Words(_) if adjust.is_none() => {
          self.close_screen();
          if let Some(c) = d.cmd {
            self.type_into_line(&format!("{c} "));
          }
        }
        Ctl::Words(_) => {}
      },
      (Some(Target::Preset(p)), None) => self.apply_preset(p),
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
    let area = self.chat_area();
    let s = self.screen.as_mut()?;
    let (lines, targets) = s.lines(&self.set);
    s.st.settle(&lines);
    let tip = match targets.get(s.st.at) {
      Some(Target::Def(d)) => d.tip,
      Some(Target::Preset(p)) => p.tip,
      _ => "",
    };
    let (title, search, keys) = match s.page {
      Page::Settings => (
        "settings",
        Some((s.q.text.as_str(), s.typing, "/ search")),
        if s.typing {
          "type to filter · enter or esc stops · up down moves"
        } else {
          "j k move · space change · / search · ? keys · esc"
        },
      ),
      Page::Keys => ("keys", None, "j k scroll · ? settings · esc closes"),
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
    assert!(matches!(&lines[0], L::Head(h) if h == "presets"));
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
    assert!(ids.contains(&"hide_bots") && ids.contains(&"hidden_words") && !ids.contains(&"vi"));
    assert!(matches!(&lines[0], L::Head(_)));
    let (none, _) = settings_lines("zzzz", &Settings::default());
    assert!(none.is_empty());
    // a preset answers its own name
    let (p, t) = settings_lines("normal", &Settings::default());
    assert!(t.iter().any(|t| matches!(t, Target::Preset(p) if p.name == "normal")) && p.len() == 2);
  }

  #[test]
  fn the_key_map_lists_every_group() {
    let (lines, targets) = keys_lines();
    assert_eq!(lines.iter().filter(|l| matches!(l, L::Head(_))).count(), KEYS.len());
    assert!(targets.iter().all(|t| matches!(t, Target::None)));
  }
}
