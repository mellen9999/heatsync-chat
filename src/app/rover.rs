// tab mode (ext tab-mode.js, same keys as the site and the ext): `space t` puts a cursor on the tab
// bar. j k move it (counts work: 3j), g G first and last, enter or l opens the tab under it, J K carry
// a channel down or up the bar, d or x closes a channel, a or o adds one, esc q t leave. moving never
// switches tabs: only enter does, which is what makes `d` safe on one key. the cursor is underlined;
// the input bar says where it is and what the keys do
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::tabs::Kind;

#[derive(Default)]
pub(super) struct Rover {
  // the tab under the cursor (an index into tabs: it follows a tab carried by J K)
  pub(super) at: usize,
  count: String,
}

impl App {
  fn shown_tabs(&self) -> Vec<usize> {
    (0..self.tabs.len()).filter(|&i| self.visible(i)).collect()
  }

  pub(super) fn rover_open(&mut self) {
    self.rover = Some(Rover { at: self.active, count: String::new() });
    self.rover_changed();
  }

  fn rover_close(&mut self) {
    self.rover = None;
    self.rover_changed();
  }

  fn rover_changed(&mut self) {
    self.bar_dirty = true;
    self.input_dirty = true;
    self.wake();
  }

  // what the input bar says while the mode is on
  pub(super) fn rover_note(&self) -> Option<String> {
    let r = self.rover.as_ref()?;
    let shown = self.shown_tabs();
    let n = shown.iter().position(|&i| i == r.at)? + 1;
    let name = &self.tabs[r.at].name;
    Some(format!("{n}/{} {name} · jk move · enter open · JK reorder · d close · a add · esc", shown.len()))
  }

  fn rover_take_count(&mut self) -> usize {
    let r = self.rover.as_mut().expect("rover open");
    let n = r.count.parse::<usize>().unwrap_or(1).clamp(1, 999);
    r.count.clear();
    n
  }

  // the cursor `by` steps along the bar, clamped to its ends
  fn rover_step(&mut self, by: isize) {
    let shown = self.shown_tabs();
    let Some(r) = &mut self.rover else { return };
    let p = shown.iter().position(|&i| i == r.at).unwrap_or(0) as isize;
    r.at = shown[(p + by).clamp(0, shown.len() as isize - 1) as usize];
  }

  // carry the channel under the cursor one place along the channels; built-in tabs stay put
  fn rover_carry(&mut self, down: bool) {
    let Some(at) = self.rover.as_ref().map(|r| r.at) else { return };
    if self.tabs[at].kind != Kind::Chat {
      self.say("that tab is fixed");
      return;
    }
    let shown = self.shown_tabs();
    let p = shown.iter().position(|&i| i == at).unwrap_or(0);
    let next = if down { shown.get(p + 1..).and_then(|s| s.first()) } else { p.checked_sub(1).map(|q| &shown[q]) };
    let Some(&to) = next.filter(|&&j| self.tabs[j].kind == Kind::Chat) else { return };
    self.tabs.swap(at, to);
    if self.active == at {
      self.active = to;
    } else if self.active == to {
      self.active = at;
    }
    if let Some(r) = &mut self.rover {
      r.at = to;
    }
    self.save_tabs();
  }

  fn rover_remove(&mut self) {
    let Some(at) = self.rover.as_ref().map(|r| r.at) else { return };
    if self.tabs[at].kind != Kind::Chat {
      self.say("that tab can't be closed");
      return;
    }
    let p = self.shown_tabs().iter().position(|&i| i == at).unwrap_or(0);
    self.remove_tab(at);
    // land on whatever slid into the gap
    let shown = self.shown_tabs();
    match shown.get(p.min(shown.len().saturating_sub(1))) {
      Some(&i) => {
        if let Some(r) = &mut self.rover {
          r.at = i;
        }
      }
      None => self.rover = None,
    }
  }

  // every key goes here while the mode is on: the ones it doesn't know are swallowed
  pub(super) fn rover_key(&mut self, k: Key<&str>) {
    match k {
      Key::Named(NamedKey::Escape) | Key::Character("q" | "t") => return self.rover_close(),
      Key::Character(d) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
        let r = self.rover.as_mut().expect("rover open");
        if d != "0" || !r.count.is_empty() {
          r.count.push_str(d);
        }
      }
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => {
        let n = self.rover_take_count();
        self.rover_step(n as isize);
      }
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => {
        let n = self.rover_take_count();
        self.rover_step(-(n as isize));
      }
      Key::Character("g") | Key::Named(NamedKey::Home) => self.rover_step(isize::MIN / 2),
      Key::Character("G") | Key::Named(NamedKey::End) => self.rover_step(isize::MAX / 2),
      Key::Named(NamedKey::Enter) | Key::Character("l") | Key::Named(NamedKey::ArrowRight) => {
        let at = self.rover.take().map(|r| r.at);
        if let Some(i) = at {
          self.select(i);
        }
      }
      Key::Character("J") => self.rover_carry(true),
      Key::Character("K") => self.rover_carry(false),
      Key::Character("d" | "x") => self.rover_remove(),
      Key::Character("a" | "o") => {
        self.rover = None;
        self.open_form(None);
      }
      _ => return,
    }
    self.rover_changed();
  }
}
