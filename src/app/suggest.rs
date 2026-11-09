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

// the open `:` `@` `/` list, the text and cursor it was made for, its rows on screen (index, rect)
#[derive(Default)]
pub(super) struct SuggestUi {
  pub(super) list: Option<Suggest>,
  pub(super) at: (Box<str>, usize),
  pub(super) rows: Vec<(usize, Rect)>,
  // the row the pointer is on (reversed in place of the keyboard's pick until a key moves)
  pub(super) hot: Option<usize>,
}

impl App {
  // the list for the text as it stands; esc keeps it shut until the text or cursor moves
  pub(super) fn sync_suggest(&mut self) {
    // only while typing in the box itself, and not mid tab-cycle; looked at afresh when that resumes
    if self.prompt.is_some() || self.mode != Mode::Insert || self.complete.is_some() {
      self.sug.list = None;
      self.sug.at.1 = usize::MAX;
      return;
    }
    let (text, cur) = (&self.line.text, self.line.cursor);
    if self.sug.at.1 == cur && *self.sug.at.0 == **text {
      return;
    }
    self.sug.at = (text.as_str().into(), cur);
    let old = self.sug.list.take();
    let Some((kind, range)) = trigger(text, cur) else { return };
    let mut items = self.candidates(&text[range.clone()], kind == Kind::Command);
    items.truncate(kind.max());
    if items.is_empty() {
      return;
    }
    // the same name stays picked as the list narrows
    let was = old.filter(|o| o.kind == kind).and_then(|o| o.items.get(o.sel).map(|p| p.name.clone()));
    let sel = was.and_then(|n| items.iter().position(|p| p.name == n)).unwrap_or(0);
    self.sug.hot = None;
    self.sug.list = Some(Suggest { kind, range, items, sel });
  }

  // keys an open list takes first. true = handled
  pub(super) fn suggest_key(&mut self, key: &Key<&str>) -> bool {
    let Some(s) = &mut self.sug.list else { return false };
    let plain = !self.mods.shift_key() && !self.mods.control_key();
    match key {
      Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) if plain => s.step(*key == Key::Named(NamedKey::ArrowDown)),
      // shift-tab goes the other way: the one above, the last from the top
      Key::Named(NamedKey::Tab) if self.mods.shift_key() => {
        let n = s.items.len().max(1);
        let sel = (s.sel + n - 1) % n;
        self.suggest_pick(sel);
      }
      Key::Named(NamedKey::Tab | NamedKey::Enter) => {
        let sel = s.sel;
        self.suggest_pick(sel);
      }
      Key::Named(NamedKey::Escape) => self.sug.list = None,
      _ => return false,
    }
    self.sug.hot = None;
    self.input_dirty = true;
    self.wake();
    true
  }

  // the pointer moved: the row under it reverses (a key takes the pick back)
  pub(super) fn suggest_hover(&mut self) {
    let (x, y) = self.pointer;
    let hot = self.sug.list.as_ref().and_then(|_| self.sug.rows.iter().find(|r| r.1.contains(x, y)).map(|r| r.0));
    if hot != self.sug.hot {
      self.sug.hot = hot;
      self.input_dirty = true;
      self.wake();
    }
  }

  // a pick replaces what was typed; the list stays on as the tab cycle
  fn suggest_pick(&mut self, i: usize) {
    let Some(s) = self.sug.list.take() else { return };
    self.line.checkpoint(false);
    self.complete =
      Some(Complete { start: s.range.start, end: s.range.end, matches: s.items, at: i, chipped: None, remote: None });
    self.show_pick();
  }

  // a click on a row picks it. true = the press was the list's
  pub(super) fn suggest_click(&mut self) -> bool {
    let (x, y) = self.pointer;
    let Some(i) = self.sug.list.as_ref().and_then(|_| self.sug.rows.iter().position(|r| r.1.contains(x, y))) else {
      return false;
    };
    let at = self.sug.rows[i].0;
    self.suggest_pick(at);
    true
  }

  // the list, or else the cycle's tooltip, over the bottom of the chat; remembers that it covered it
  pub(super) fn draw_over_input(&mut self) -> Option<Rect> {
    self.sug.rows.clear();
    let tip = self.complete.as_ref().and_then(Complete::tip);
    self.over_input = self.input_h > 0 && (self.sug.list.is_some() || tip.is_some());
    if !self.over_input {
      return None;
    }
    let input = self.input_area();
    let Some(s) = &self.sug.list else {
      return Some(ui::tip(&mut self.canvas, &mut self.res.text, input, &tip?));
    };
    let (kind, sel) = (s.kind, s.sel);
    let names: Vec<Box<str>> = s.items.iter().map(|p| p.shown().into()).collect();
    let platform = self.target().map_or(Platform::Twitch, |t| t.0);
    let cmds = if kind == Kind::Command { slash::all() } else { Vec::new() };
    let own: Vec<(String, String)> = if kind == Kind::Command {
      self.ucmds.iter().map(|(n, e)| (format!("/{n}"), e.to_owned())).collect()
    } else {
      Vec::new()
    };
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
          if let Some((_, e)) = own.iter().find(|(n, _)| **n == **name) {
            row.desc = e;
          } else if let Some(c) = cmds.iter().find(|c| *c.name == **name) {
            (row.args, row.desc) = (c.args, crate::i18n::t(c.desc));
          }
        }
      }
      rows.push(row);
    }
    let top = self.chat_area().y;
    let (r, hits) = ui::list(
      &mut self.canvas,
      &mut self.res.text,
      &mut self.res.images,
      input,
      top,
      &rows,
      (sel, self.sug.hot.filter(|&h| h < rows.len())),
    );
    self.sug.rows = hits;
    Some(r)
  }
}
