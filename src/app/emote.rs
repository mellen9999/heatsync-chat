// emote clicks: left inserts the name; right acts on the exact emote under the pointer (rule 10) as
// `right-click an emote` says: a menu, or at once (a blocked one unblocks, yours is removed, any other is
// blocked), or nothing; shift+right always opens the small menu with the rest
use crate::i18n::{f, t};
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::overlay::Overlay;
use crate::emotes::block::{Click, Scope, emote_hash, right_click};
use crate::model::settings::RightClick;
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

fn label(a: Act) -> &'static str {
  match a {
    Act::Block => t("app.emote.block"),
    Act::Unblock => t("app.emote.unblock"),
    Act::Add => t("app.emote.add"),
    Act::Remove => t("app.emote.remove"),
    Act::Copy => t("app.emote.copy"),
  }
}

// what a plain right-click does to this exact emote (rule 10)
fn primary(owned: bool, blocked: bool) -> Act {
  match right_click(owned, blocked) {
    Click::Remove => Act::Remove,
    Click::Block => Act::Block,
    Click::Unblock => Act::Unblock,
  }
}

// the menu rows for an emote, given what it is to you: what a right-click would do leads
fn rows(owned: bool, blocked: bool, logged_in: bool) -> Vec<(Act, &'static str)> {
  let lead = primary(owned, blocked);
  let mut v = vec![(lead, label(lead))];
  let rest = [if blocked { Act::Unblock } else { Act::Block }, if owned { Act::Remove } else { Act::Add }];
  for a in rest {
    if a != lead && (logged_in || matches!(a, Act::Block | Act::Unblock)) {
      v.push((a, label(a)));
    }
  }
  v.push((Act::Copy, label(Act::Copy)));
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
    } else {
      let mode = self.set.right_click;
      // off leaves the emote alone, and the press falls through to the line under it
      if mode == RightClick::Off {
        return false;
      }
      if self.mods.shift_key() || mode == RightClick::Menu {
        let rows = rows(owned, blocked, self.me.is_some());
        self.overlay = Some(Overlay::Emote(Open { menu: Menu::new(rows, self.pointer), hit, owned }));
      } else {
        self.run_act(primary(owned, blocked), &hit, owned);
      }
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
      Act::Copy => self.copy(name, f("app.emote.copied", &[("name", name)])),
      Act::Block | Act::Unblock => {
        let block = act == Act::Block;
        let hashes = if block {
          self.res.blocked.block(name, &hit.url, scope);
          vec![hash]
        } else {
          self.res.blocked.unblock(name, &hit.url, scope)
        };
        self.res.blocked.save();
        self.say(f(if block { "app.emote.blocked" } else { "app.emote.unblocked" }, &[("name", name)]));
        if self.me.is_some() {
          for hash in hashes {
            self.net.send(Cmd::Emote(if block { EmoteCmd::Block { hash } } else { EmoteCmd::Unblock { hash } }));
          }
        }
        self.relayout_where(None);
      }
      Act::Add | Act::Remove if self.me.is_none() => self.say(t("app.login_first_ctrl_l")),
      Act::Add => {
        self.say(f("app.emote.adding", &[("name", name)]));
        self.net.send(Cmd::Emote(EmoteCmd::Add {
          name: name.clone(),
          url: hit.url.clone(),
          hash: Some(hash),
          zero_width: hit.zero_width,
        }));
      }
      Act::Remove => {
        self.say(f("app.emote.removing", &[("name", name)]));
        self.net.send(Cmd::Emote(EmoteCmd::Remove { name: name.clone() }));
      }
    }
    self.bar_dirty = true;
  }

  pub fn menu_key(&mut self, key: Key<&str>) {
    let Some(Overlay::Emote(o)) = &mut self.overlay else { return };
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => o.menu.step(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => o.menu.step(-1),
      Key::Named(NamedKey::Enter) => return self.menu_pick(),
      Key::Named(NamedKey::Escape) => self.close_menu(),
      _ => {}
    }
    self.menu_changed();
  }

  fn menu_pick(&mut self) {
    if let Some(o) = self.take_menu()
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
    let row = self.menu().and_then(|o| o.menu.row_at(x, y, line));
    match (row, &mut self.overlay) {
      (Some(i), Some(Overlay::Emote(o))) => {
        o.menu.at = i;
        self.menu_pick();
      }
      _ => {
        self.close_menu();
        self.menu_changed();
      }
    }
  }

  // hover moves the highlight
  pub fn menu_hover(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    if let Some(Overlay::Emote(o)) = &mut self.overlay
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
    let Some(Overlay::Emote(o)) = &mut self.overlay else { return None };
    Some(o.menu.draw(&mut self.canvas, &mut self.res.text, bounds))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn menu_rows_lead_with_what_a_right_click_does() {
    let copy = (Act::Copy, "copy name");
    assert_eq!(rows(false, false, false), vec![(Act::Block, "block"), copy]);
    assert_eq!(rows(false, true, true), vec![(Act::Unblock, "unblock"), (Act::Add, "add to my emotes"), copy]);
    assert_eq!(rows(true, false, true), vec![(Act::Remove, "remove from my emotes"), (Act::Block, "block"), copy]);
    assert_eq!(rows(true, true, true)[0], (Act::Unblock, "unblock"));
  }

  fn hit(url: &str) -> EmoteHit {
    EmoteHit { name: "W".into(), url: url.into(), zero_width: false, scope: Scope::Channel, over: Vec::new() }
  }

  // rules 8 + 10: yours means the exact emote you own under that name. the room's W, a friend's W and a
  // bttv W are all someone else's, so a right-click on them blocks and never removes yours
  #[test]
  fn only_the_exact_emote_you_own_is_yours() {
    let (net, _rx) = crate::net::Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    let mine = "https://cdn.7tv.app/emote/MINE/1x.webp";
    let row = serde_json::json!({"custom_name": "W", "url": mine, "width": 32, "height": 32,
      "added_at": "2026-10-30T22:26:51.099Z"});
    app.me = Some(crate::net::auth::Me {
      username: "m".into(),
      platform: "twitch".into(),
      twitch: Some("m".into()),
      twitch_id: Some(1),
      names: vec!["m".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
    app.res.senders.lookup(1, "W", 0);
    app.res.senders.put(1, crate::heatsync::senders::parse_set(&serde_json::json!([row])));
    let yours = hit(mine);
    let room = hit("https://cdn.7tv.app/emote/ROOM/1x.webp");
    let bttv = hit("https://cdn.betterttv.net/emote/zz/1x.webp");
    assert!(app.owned(&yours));
    assert!(!app.owned(&room) && !app.owned(&bttv));
    assert_eq!(primary(app.owned(&yours), false), Act::Remove);
    assert_eq!(primary(app.owned(&room), false), Act::Block);
    // blocking the room's W leaves mine and the bttv one showing
    app.res.blocked.block("W", &room.url, Scope::Channel);
    let blocked = |app: &App, h: &EmoteHit| app.res.blocked.contains(&h.name, &h.url);
    assert_eq!([blocked(&app, &room), blocked(&app, &yours), blocked(&app, &bttv)], [true, false, false]);
    assert_eq!(primary(app.owned(&room), blocked(&app, &room)), Act::Unblock);
    // removing mine is a live change: from then on it is no longer yours, in every tab
    assert_eq!(app.res.senders.stale(1, Some("W")), Some(true));
    assert!(!app.owned(&yours));
  }

  // rule 10: honoured on every surface: menu is the default, instant and off are the other two answers,
  // and the setting answers to the site's own key name
  #[test]
  fn right_click_mode_is_a_setting_with_three_answers() {
    use crate::model::settings::{RightClick, Settings};
    assert_eq!(Settings::default().right_click, RightClick::Menu);
    assert_eq!(RightClick::CHOICE.usage, "<menu|instant|off>");
    assert_eq!(Settings::parse("right_click=off\n").right_click, RightClick::Off);
    assert_eq!(Settings::parse("right_click=bogus\n").right_click, RightClick::Menu);
    assert!(crate::model::schema::def("right_click").unwrap().matches("rightClickBlockMode"));
  }
}
