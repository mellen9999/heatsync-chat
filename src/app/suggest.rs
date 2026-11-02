// the `:` `@` `/` lists over the message box: kept in step with the text, driven by keys and clicks,
// and drawn with the tab tooltip (whichever is up) over the bottom of the chat
use winit::keyboard::{Key, NamedKey};

use super::App;
use crate::model::complete::Complete;
use crate::model::message::Platform;
use crate::model::slash;
use crate::model::suggest::{Kind, Suggest, trigger};
use crate::model::vi::Mode;
use crate::ui::canvas::Rect;
use crate::ui::res::EMOTE_PX;
use crate::ui::suggest::{self as ui, Row};
use crate::ui::theme;

impl App {
  // the list for the text as it stands; esc keeps it shut until the text or cursor moves
  pub(super) fn sync_suggest(&mut self) {
    // only while typing in the box itself, and not mid tab-cycle; looked at afresh when that resumes
    if self.prompt.is_some() || self.mode != Mode::Insert || self.complete.is_some() {
      self.suggest = None;
      self.suggest_at.1 = usize::MAX;
      return;
    }
    let (text, cur) = (&self.line.text, self.line.cursor);
    if self.suggest_at.1 == cur && *self.suggest_at.0 == **text {
      return;
    }
    self.suggest_at = (text.as_str().into(), cur);
    let old = self.suggest.take();
    let Some((kind, range)) = trigger(text, cur) else { return };
    let mut items = self.candidates(&text[range.clone()], kind == Kind::Command);
    items.truncate(kind.max());
    if items.is_empty() {
      return;
    }
    // the same name stays picked as the list narrows
    let was = old.filter(|o| o.kind == kind).and_then(|o| o.items.get(o.sel).map(|p| p.name.clone()));
    let sel = was.and_then(|n| items.iter().position(|p| p.name == n)).unwrap_or(0);
    self.suggest = Some(Suggest { kind, range, items, sel });
  }

  // keys an open list takes first. true = handled
  pub(super) fn suggest_key(&mut self, key: &Key<&str>) -> bool {
    let Some(s) = &mut self.suggest else { return false };
    let plain = !self.mods.shift_key() && !self.mods.control_key();
    match key {
      Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) if plain => s.step(*key == Key::Named(NamedKey::ArrowDown)),
      Key::Named(NamedKey::Tab | NamedKey::Enter) => {
        let sel = s.sel;
        self.suggest_pick(sel);
      }
      Key::Named(NamedKey::Escape) => self.suggest = None,
      _ => return false,
    }
    self.input_dirty = true;
    self.wake();
    true
  }

  // a pick replaces what was typed; the list stays on as the tab cycle
  fn suggest_pick(&mut self, i: usize) {
    let Some(s) = self.suggest.take() else { return };
    self.line.checkpoint(false);
    self.complete = Some(Complete { start: s.range.start, end: s.range.end, matches: s.items, at: i, chipped: None });
    self.show_pick();
  }

  // a click on a row picks it. true = the press was the list's
  pub(super) fn suggest_click(&mut self) -> bool {
    let (x, y) = self.pointer;
    let Some(i) = self.suggest.as_ref().and_then(|_| self.suggest_rows.iter().position(|r| r.1.contains(x, y))) else {
      return false;
    };
    let at = self.suggest_rows[i].0;
    self.suggest_pick(at);
    true
  }

  // the list, or else the cycle's tooltip, over the bottom of the chat; remembers that it covered it
  pub(super) fn draw_over_input(&mut self) -> Option<Rect> {
    self.suggest_rows.clear();
    let tip = self.complete.as_ref().and_then(Complete::tip);
    self.over_input = self.input_h > 0 && (self.suggest.is_some() || tip.is_some());
    if !self.over_input {
      return None;
    }
    let input = self.input_area();
    let Some(s) = &self.suggest else {
      return Some(ui::tip(&mut self.canvas, &mut self.res.text, input, &tip?));
    };
    let (kind, sel) = (s.kind, s.sel);
    let names: Vec<Box<str>> = s.items.iter().map(|p| p.name.clone()).collect();
    let platform = self.target().map_or(Platform::Twitch, |t| t.0);
    let cmds = if kind == Kind::Command { slash::all() } else { Vec::new() };
    let mut rows = Vec::with_capacity(names.len());
    for name in &names {
      let mut row = Row { img: None, name, ink: theme::WHITE, bold: false, args: "", desc: "" };
      match kind {
        Kind::Emote => {
          let url = self.box_url(name);
          row.img = url.map(|u| self.res.images.id(&u, EMOTE_PX));
        }
        Kind::People => {
          let who = self.res.chatters.person(platform, name.trim_start_matches('@'), true);
          row.ink = who.map_or(theme::WHITE, |w| w.color);
          row.bold = true;
        }
        Kind::Command => {
          if let Some(c) = cmds.iter().find(|c| *c.name == **name) {
            (row.args, row.desc) = (c.args, c.desc);
          }
        }
      }
      rows.push(row);
    }
    let top = self.chat_area().y;
    let (r, hits) = ui::list(&mut self.canvas, &mut self.res.text, &mut self.res.images, input, top, &rows, sel);
    self.suggest_rows = hits;
    Some(r)
  }
}
