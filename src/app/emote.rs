// emote clicks: left inserts the name, right toggles the block (or removes it from your own
// heatsync emotes), shift+right opens a small menu with the rest
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::emotes::block::{Click, Scope, emote_hash, right_click};
use crate::net::Cmd;
use crate::net::emote_ops::EmoteCmd;
use crate::ui::canvas::Rect;
use crate::ui::chat_view::EmoteHit;
use crate::ui::menu::Menu;
use crate::util::time::now_ms;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
  Block,
  Unblock,
  Add,
  Remove,
  Copy,
}

pub struct Open {
  menu: Menu<Act>,
  hit: EmoteHit,
  owned: bool,
}

// the menu rows for an emote, given what it is to you
fn rows(owned: bool, blocked: bool, logged_in: bool) -> Vec<(Act, &'static str)> {
  let mut v = vec![(Act::Copy, "copy name")];
  v.push(if blocked { (Act::Unblock, "unblock") } else { (Act::Block, "block") });
  if logged_in {
    v.push(if owned { (Act::Remove, "remove from my emotes") } else { (Act::Add, "add to my emotes") });
  }
  v
}

impl App {
  // is this exact emote (same name, same image) in your own heatsync inventory?
  pub(super) fn owned(&mut self, hit: &EmoteHit) -> bool {
    let Some(id) = self.me.as_ref().and_then(|m| m.twitch_id) else { return false };
    self.res.senders.lookup(id, &hit.name, now_ms()).is_some_and(|x| x.url == hit.url)
  }

  pub(super) fn emote_under_pointer(&mut self) -> Option<EmoteHit> {
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    if !chat.contains(x, y) {
      return None;
    }
    let res = &mut self.res;
    self.tabs.get_mut(self.active)?.view.emote_at(chat, x, y, res)
  }

  // true when the press landed on an emote and was handled
  pub fn emote_press(&mut self, right: bool) -> bool {
    let Some(hit) = self.emote_under_pointer() else { return false };
    let blocked = self.res.blocked.contains(&hit.name, &hit.url);
    let owned = self.owned(&hit);
    if !right {
      // a blocked emote unblocks on click (as in the ext); anything else goes into the box
      if blocked {
        self.run_act(Act::Unblock, &hit, owned);
      } else {
        self.line.insert_chip(&hit.name);
        self.used_emote(&hit.name);
        self.complete = None;
        self.enter_insert();
        self.input_dirty = true;
      }
    } else if self.mods.shift_key() {
      let rows = rows(owned, blocked, self.me.is_some());
      self.menu = Some(Open { menu: Menu::new(rows, self.pointer), hit, owned });
    } else {
      let act = match right_click(owned, blocked) {
        Click::Remove => Act::Remove,
        Click::Block => Act::Block,
        Click::Unblock => Act::Unblock,
      };
      self.run_act(act, &hit, owned);
    }
    self.wake();
    true
  }

  fn run_act(&mut self, act: Act, hit: &EmoteHit, owned: bool) {
    // an emote in your inventory is filed the way the ext files inventory rows
    let scope = if owned { Scope::Hs } else { hit.scope };
    let hash = emote_hash(&hit.url, scope);
    let name = &hit.name;
    match act {
      Act::Copy => {
        self.status.clear();
        self.copy(name);
        if self.status.is_empty() {
          self.status = format!("copied {name}");
        }
      }
      Act::Block | Act::Unblock => {
        let block = act == Act::Block;
        let hashes = if block {
          self.res.blocked.block(name, &hit.url, scope);
          vec![hash]
        } else {
          self.res.blocked.unblock(name, &hit.url, scope)
        };
        self.res.blocked.save();
        self.status = format!("{} {name}", if block { "blocked" } else { "unblocked" });
        if self.me.is_some() {
          for hash in hashes {
            self.net.send(Cmd::Emote(if block { EmoteCmd::Block { hash } } else { EmoteCmd::Unblock { hash } }));
          }
        }
        self.relayout_where(None);
      }
      Act::Add | Act::Remove if self.me.is_none() => self.status = "log in first (ctrl-l)".into(),
      Act::Add => {
        self.status = format!("adding {name}…");
        self.net.send(Cmd::Emote(EmoteCmd::Add {
          name: name.clone(),
          url: hit.url.clone(),
          hash: Some(hash),
          zero_width: hit.zero_width,
        }));
      }
      Act::Remove => {
        self.status = format!("removing {name}…");
        self.net.send(Cmd::Emote(EmoteCmd::Remove { name: name.clone() }));
      }
    }
    self.bar_dirty = true;
  }

  pub fn menu_key(&mut self, key: Key<&str>) {
    let Some(o) = &mut self.menu else { return };
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => o.menu.step(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => o.menu.step(-1),
      Key::Named(NamedKey::Enter) => return self.menu_pick(),
      Key::Named(NamedKey::Escape) => self.menu = None,
      _ => {}
    }
    self.menu_changed();
  }

  fn menu_pick(&mut self) {
    if let Some(o) = self.menu.take()
      && let Some(act) = o.menu.chosen()
    {
      self.run_act(act, &o.hit, o.owned);
    }
    self.menu_changed();
  }

  // a click while the menu is up: on a row picks it, anywhere else closes it
  pub fn menu_click(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    let row = self.menu.as_ref().and_then(|o| o.menu.row_at(x, y, line));
    match (row, &mut self.menu) {
      (Some(i), Some(o)) => {
        o.menu.at = i;
        self.menu_pick();
      }
      _ => {
        self.menu = None;
        self.menu_changed();
      }
    }
  }

  // hover moves the highlight
  pub fn menu_hover(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    if let Some(o) = &mut self.menu
      && let Some(i) = o.menu.row_at(x, y, line)
      && i != o.menu.at
    {
      o.menu.at = i;
      self.wake();
    }
  }

  pub fn menu_changed(&mut self) {
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.wake();
  }

  // drawn last so it sits over everything; returns its rect for damage
  pub fn draw_menu(&mut self) -> Option<Rect> {
    let bounds = self.chat_area();
    let o = self.menu.as_mut()?;
    Some(o.menu.draw(&mut self.canvas, &mut self.res.text, bounds))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn menu_rows_follow_state() {
    let copy = (Act::Copy, "copy name");
    assert_eq!(rows(false, false, false), vec![copy, (Act::Block, "block")]);
    assert_eq!(rows(false, true, true), vec![copy, (Act::Unblock, "unblock"), (Act::Add, "add to my emotes")]);
    assert_eq!(rows(true, false, true), vec![copy, (Act::Block, "block"), (Act::Remove, "remove from my emotes")]);
  }
}
