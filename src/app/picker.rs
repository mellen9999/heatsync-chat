// the emote picker: ctrl-; or the logo button. filter box, then a grid of local emotes
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::picker::{Focus, Item, Picker, Sec, Tab};
use crate::ui::picker::{Geo, draw, geo_of};
use crate::util::time::now_ms;

impl App {
  pub fn toggle_picker(&mut self) {
    if self.picker.is_some() {
      return self.close_picker();
    }
    let chan: Box<str> = self.tabs.get(self.active).map_or("".into(), |t| t.view.channel.clone());
    let item = |sec, name: &str, url: &str, w, h| Item { sec, name: name.into(), url: url.into(), w, h };
    // yours: your heatsync emotes, then your twitch sub emotes
    let mut mine: Vec<Item> = match self.me.as_ref().and_then(|m| m.twitch_id) {
      Some(id) => {
        self.res.senders.entries(id, now_ms()).into_iter().map(|(n, e)| item(Sec::Mine, n, &e.url, e.w, e.h)).collect()
      }
      None => Vec::new(),
    };
    let subs = self.res.emotes.sub_entries().into_iter().filter(|(n, _)| !mine.iter().any(|m| &*m.name == *n));
    let subs: Vec<Item> = subs.map(|(n, e)| item(Sec::Mine, n, &e.url, e.w, e.h)).collect();
    mine.extend(subs);
    let channel = self
      .res
      .emotes
      .channel_entries(&chan)
      .into_iter()
      .map(|(n, e)| item(Sec::Channel, n, &e.url, e.w, e.h))
      .collect();
    let global =
      self.res.emotes.global_entries().into_iter().map(|(n, e)| item(Sec::Global, n, &e.url, e.w, e.h)).collect();
    self.close_menu();
    let mut p = Picker::new(mine, channel, global, &self.recents);
    p.prefs = crate::model::picker_search::Prefs::load();
    self.picker = Some(p);
    self.picker_changed();
  }

  pub fn close_picker(&mut self) {
    self.hide_tip();
    self.picker = None;
    self.pick_due = None;
    self.gifs_leave();
    self.ime.clear();
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.input_dirty = true;
    self.trim_soon();
    self.wake();
  }

  pub(super) fn picker_changed(&mut self) {
    self.gifs_sync();
    self.picker_search_sync();
    self.input_dirty = true;
    self.wake();
  }

  pub(super) fn picker_geo(&self) -> Geo {
    geo_of(self.chat_area(), &self.res, self.picker.as_ref().map_or(Tab::Emotes, |p| p.tab))
  }

  // go to the other tab (or `to`): each keeps its own text and place
  pub(super) fn picker_tab(&mut self, to: Option<Tab>) {
    let Some(p) = &mut self.picker else { return };
    let to = to.unwrap_or(p.tab.next());
    if to == p.tab {
      return;
    }
    p.switch(to);
    if to == Tab::Gifs {
      self.gifs_enter();
    } else {
      self.gifs_leave();
    }
    self.picker_reveal();
    self.picker_changed();
  }

  pub fn picker_reveal(&mut self) {
    let g = self.picker_geo();
    if let Some(p) = &mut self.picker {
      p.reveal(g.m, g.grid.h);
    }
    self.wake();
  }

  // insert the selected emote; shift keeps the picker open for another
  fn picker_pick(&mut self, keep: bool) {
    if self.picker.as_ref().is_some_and(|p| p.tab == Tab::Gifs) {
      return self.gif_pick(keep);
    }
    if self.picker.as_ref().is_some_and(|p| p.tab == Tab::Emoji) {
      return self.emoji_pick(keep);
    }
    let Some(it) = self.picker.as_ref().and_then(|p| p.selected()) else { return };
    let (name, url) = (it.name.clone(), it.url.clone());
    // a catalog tile is a candidate in the box, remembered like a tab-completed one: sending the line is
    // what adds it to your inventory. anything local is the emote you mean by that name
    let remote = self.picker.as_ref().and_then(|p| p.selected_hit()).cloned();
    if self.res.blocked.contains(&name, &url) {
      self.say(format!("{name} is blocked (right-click it in chat to unblock)"));
      self.bar_dirty = true;
      return self.wake();
    }
    match &remote {
      Some(h) => self.catalog_reg.track(h),
      None => self.catalog_reg.forget(&name),
    }
    self.line.insert_chip(&name);
    self.used_emote(&name);
    self.complete = None;
    if !keep {
      self.close_picker();
      self.enter_insert();
    } else {
      self.input_dirty = true;
      self.wake();
    }
  }

  // the emoji tab's pick: the glyph itself goes in the box (the ext types :name:, which turns into this on send)
  fn emoji_pick(&mut self, keep: bool) {
    let Some(glyph) = self.picker.as_ref().and_then(|p| p.selected()).map(|i| i.url.clone()) else { return };
    self.line.insert_word(&glyph);
    self.complete = None;
    if keep {
      self.input_dirty = true;
      return self.wake();
    }
    self.close_picker();
    self.enter_insert();
  }

  pub fn picker_wheel(&mut self, px: i32) {
    let g = self.picker_geo();
    if let Some(p) = &mut self.picker {
      p.scroll_by(-px, g.m, g.grid.h);
    }
    self.picker_search_scrolled();
    self.wake();
  }

  pub fn picker_hover(&mut self) {
    let g = self.picker_geo();
    let (x, y) = self.pointer;
    let Some(p) = &mut self.picker else { return };
    if g.grid.contains(x, y)
      && let Some(i) = p.cell_at(x - g.grid.x, y - g.grid.y, g.m)
      && i != p.sel
    {
      p.sel = i;
      self.gifs_sync();
      self.wake();
    }
  }

  // a click: on a cell inserts it, on the filter row focuses it, anywhere else closes
  pub fn picker_click(&mut self) {
    let g = self.picker_geo();
    let (x, y) = self.pointer;
    let tab = self.picker.as_ref().and_then(|p| Tab::ALL.into_iter().zip(p.view.tabs).find(|(_, r)| r.contains(x, y)));
    if let Some((t, _)) = tab {
      return self.picker_tab(Some(t));
    }
    let chip = self.picker.as_ref().and_then(|p| p.view.chips.iter().position(|r| r.w > 0 && r.contains(x, y)));
    if let Some(i) = chip {
      return self.picker_chip(i);
    }
    let hit = self
      .picker
      .as_ref()
      .and_then(|p| g.grid.contains(x, y).then(|| p.cell_at(x - g.grid.x, y - g.grid.y, g.m)).flatten());
    if let Some(i) = hit {
      if let Some(p) = &mut self.picker {
        p.sel = i;
      }
      return self.picker_pick(self.mods.shift_key());
    }
    if g.search.contains(x, y) || g.chips.contains(x, y) || g.grid.contains(x, y) {
      if let Some(p) = &mut self.picker {
        p.focus = Focus::Search;
      }
      return self.wake();
    }
    self.close_picker();
  }

  pub fn picker_key(&mut self, key: Key<&str>, text: Option<&str>) {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    let half = self.picker_geo().grid.h / 2;
    let g = self.picker_geo();
    let Some(p) = &mut self.picker else { return };
    let focus = p.focus;
    match key {
      Key::Character(";") if ctrl => return self.close_picker(),
      Key::Named(NamedKey::Enter) => return self.picker_pick(self.mods.shift_key()),
      Key::Named(NamedKey::ArrowLeft) => p.step(-1, 0, g.m),
      Key::Named(NamedKey::ArrowRight) => p.step(1, 0, g.m),
      Key::Named(NamedKey::ArrowUp) => p.step(0, -1, g.m),
      Key::Named(NamedKey::ArrowDown) => p.step(0, 1, g.m),
      Key::Named(NamedKey::PageUp) => p.scroll_by(-half * 2, g.m, g.grid.h),
      Key::Named(NamedKey::PageDown) => p.scroll_by(half * 2, g.m, g.grid.h),
      Key::Named(NamedKey::Tab) if ctrl => return self.picker_tab(None),
      Key::Character(c @ ("1" | "2" | "3" | "4")) if alt && !ctrl => {
        return self.picker_chip(c.parse::<usize>().map_or(0, |n| n - 1));
      }
      Key::Named(NamedKey::Tab) => p.focus = if focus == Focus::Search { Focus::Grid } else { Focus::Search },
      Key::Named(NamedKey::Escape) if focus == Focus::Search && !p.query.is_empty() => p.clear_query(),
      Key::Named(NamedKey::Escape) => return self.close_picker(),
      _ if focus == Focus::Search => match key {
        Key::Named(NamedKey::Backspace) => p.backspace(),
        Key::Character("u" | "w") if ctrl => p.clear_query(),
        Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
          // mid-composition the input method owns the text; its commit arrives as an ime event
          if let Some(t) = text.filter(|_| self.ime.accepts_key_text()) {
            p.type_str(t);
          }
        }
        _ => return,
      },
      Key::Character("h") if !ctrl => p.step(-1, 0, g.m),
      Key::Character("l") if !ctrl => p.step(1, 0, g.m),
      Key::Character("k") if !ctrl => p.step(0, -1, g.m),
      Key::Character("j") if !ctrl => p.step(0, 1, g.m),
      Key::Character("]") if !ctrl => return self.picker_tab(None),
      Key::Character("[") if !ctrl => {
        let to = p.tab.prev();
        return self.picker_tab(Some(to));
      }
      Key::Character("g") => p.home(),
      Key::Character("G") => p.end(),
      Key::Character("d") if ctrl => p.scroll_by(half, g.m, g.grid.h),
      Key::Character("u") if ctrl => p.scroll_by(-half, g.m, g.grid.h),
      Key::Character("/" | "i") => p.focus = Focus::Search,
      Key::Character("q") => return self.close_picker(),
      _ => return,
    }
    // filtering or stepping changes what's selected; keep it in view
    let moved = !matches!(key, Key::Named(NamedKey::PageUp | NamedKey::PageDown)) && !ctrl;
    if moved {
      self.picker_reveal();
    }
    self.picker_changed();
  }

  // paints the picker over the whole chat area; animated cells repaint alone on their own clock
  pub fn draw_picker(&mut self) -> Option<crate::ui::canvas::Rect> {
    let area = self.chat_area();
    let p = self.picker.as_mut()?;
    let dirty = draw(&mut self.canvas, &mut self.res, area, p, self.ime.preedit());
    self.next_anim = p.view.next_anim();
    dirty
  }
}
