// the pinned-line bar under the tabs (ext pin-bar.js): newest unhidden pin of the active tab's feeds.
// click opens/folds it, the × (or space x in vi normal mode) hides that pin for this session, or, for the
// owner of the room / a twitch mod, asks to unpin it for everyone. a line's menu pins / unpins it
use super::App;
use super::overlay::Overlay;
use super::pop::{Act, Pop};
use crate::i18n::t;
use crate::model::message::Platform;
use crate::model::pin::{Pin, Pins};
use crate::net::Cmd;
use crate::net::pinact::{self, Fail, Op};
use crate::ui::canvas::Rect;
use crate::ui::{pin_bar, theme};

// the pinned-lines bar: the pins per room, its height (0 = none), whether it is open, and what it last drew
#[derive(Default)]
pub(super) struct PinBar {
  pub(super) pins: Pins,
  pub(super) h: i32,
  pub(super) open: bool,
  pub(super) sig: (Option<Box<str>>, bool, i32, i32),
  // the room "unpin for everyone?" is asking about
  pub(super) ask: Option<(Platform, Box<str>)>,
}

impl App {
  pub(super) fn pin_area(&self) -> Rect {
    let b = self.body();
    Rect::new(b.x, b.y, b.w, self.pin_bar.h)
  }

  fn cur_pin(&self) -> Option<&Pin> {
    self.pin_bar.pins.current(&self.tabs.get(self.active)?.sources)
  }

  pub(super) fn on_pin(&mut self, p: Platform, key: &str, pin: Option<Pin>) {
    // pin frames are broadcast for every room: only rooms we are in may take a slot
    if !self.tabs.iter().any(|t| t.sources.iter().any(|s| s.0 == p && *s.1 == *key)) {
      return;
    }
    self.pin_bar.pins.set(p, key, pin);
    if self.tabs.get(self.active).is_some_and(|t| t.sources.iter().any(|s| s.0 == p && *s.1 == *key)) {
      self.wake();
    }
  }

  // repaint the bar if what it shows (or the room it gets) changed; the chat below re-lays out
  pub(super) fn sync_pin(&mut self) -> Option<Rect> {
    let pin = self.cur_pin().cloned();
    let sig = (pin.as_ref().map(|p| p.id.clone()), self.pin_bar.open, self.body().w, self.res.text.line_px as i32);
    if sig == self.pin_bar.sig {
      return None;
    }
    self.pin_bar.sig = sig;
    let old = self.pin_bar.h;
    let w = self.body().w;
    self.pin_bar.h = pin.as_ref().map_or(0, |p| pin_bar::height(&mut self.res.text, p, w, self.pin_bar.open));
    let area = Rect { h: self.pin_bar.h.max(old), ..self.pin_area() };
    self.canvas.fill(area, theme::BLACK);
    let bar = self.pin_area();
    if let Some(p) = &pin {
      pin_bar::draw(&mut self.canvas, &mut self.res.text, bar, p, self.pin_bar.open);
    }
    if self.pin_bar.h != old
      && let Some(t) = self.tabs.get_mut(self.active)
    {
      t.view.invalidate();
    }
    Some(area)
  }

  // true when the click landed on the bar
  pub(super) fn pin_click(&mut self) -> bool {
    let (x, y) = self.pointer;
    let area = self.pin_area();
    if self.pin_bar.h == 0 || !area.contains(x, y) {
      return false;
    }
    if pin_bar::hide_rect(&mut self.res.text, area).contains(x, y) {
      self.pin_x();
    } else {
      self.pin_bar.open = !self.pin_bar.open;
      self.wake();
    }
    true
  }

  // the ×: the room's owner or a twitch mod unpins for everyone (asked first), anyone else hides it here
  pub(super) fn pin_x(&mut self) {
    let Some(t) = self.tabs.get(self.active) else { return };
    let Some((p, room, _)) = self.pin_bar.pins.current_in(&t.sources) else { return };
    if !self.can_pin(p, &room) {
      return self.pin_hide();
    }
    self.pin_bar.ask = Some((p, room));
    let area = self.pin_area();
    let rows = vec![
      (Act::UnpinYes, crate::i18n::t("app.pin.yes").to_owned()),
      (Act::No, crate::i18n::t("app.cancel").to_owned()),
    ];
    let head = crate::i18n::t("app.pin.ask").to_owned();
    self.overlay = Some(Overlay::Pop(Pop::ask(head, (area.x + 24, area.y + area.h), rows)));
    self.pop_changed();
  }

  pub(super) fn pin_hide(&mut self) {
    if let Some(id) = self.cur_pin().map(|p| p.id.clone()) {
      self.pin_bar.pins.hide(&id);
      self.pin_bar.open = false;
      self.wake();
    }
  }
}

impl App {
  // may we pin in this room? our own channel on that platform, or a twitch mod. kick mods are refused
  // by the server, so on kick only the owner
  pub(super) fn can_pin(&self, p: Platform, chan: &str) -> bool {
    let Some(me) = &self.me else { return false };
    let own = me.name_on(p).is_some_and(|n| match p {
      Platform::Kick => crate::model::modrole::kick_slug(n) == chan,
      _ => n.eq_ignore_ascii_case(chan),
    });
    match p {
      Platform::Twitch => own || self.mod_in.contains(chan),
      Platform::Kick => own,
      Platform::Youtube => false,
    }
  }

  // the pin row a line's menu offers: None = none, Some(true) = this line is the room's pin
  pub(super) fn pin_menu(&self, m: &crate::model::message::Message) -> Option<bool> {
    let t = self.tabs.get(self.active).filter(|t| t.kind == crate::model::tabs::Kind::Chat)?;
    if !m.kind.said() || m.id.is_empty() || !matches!(m.platform, Platform::Twitch | Platform::Kick) {
      return None;
    }
    let chan = self.room_of(m).filter(|c| t.sources.iter().any(|s| s.0 == m.platform && s.1 == *c))?;
    if !self.can_pin(m.platform, &chan) {
      return None;
    }
    let pinned = self.pin_bar.pins.in_room(m.platform, &chan).is_some_and(|p| *p.msg_id == *m.id);
    Some(pinned)
  }

  // the line the menu was opened on: pin it, or (it is the pin) clear the room's pin
  pub(super) fn pin_selected(&mut self, set: bool) {
    let Some(m) = self.tabs.get(self.active).and_then(|t| t.view.selected()) else { return };
    let Some(want) = self.pin_menu(m) else { return };
    let (platform, msg_id) = (m.platform, m.id.clone());
    let Some(chan) = self.room_of(m) else { return };
    // the menu is stale if the pin moved under it: do what the line is now
    let op = if set && !want { Op::Set { platform, chan, msg_id } } else { Op::Clear { platform, chan } };
    self.net.send(Cmd::Pin(op));
  }

  // "yes" to unpin for everyone
  pub(super) fn pin_clear_go(&mut self) {
    if let Some((platform, chan)) = self.pin_bar.ask.take()
      && self.can_pin(platform, &chan)
    {
      self.net.send(Cmd::Pin(Op::Clear { platform, chan }));
    }
  }

  pub(super) fn on_pin_done(&mut self, d: pinact::Done) {
    let pinact::Done { platform, chan, set, res } = d;
    match res {
      Ok(pin) => {
        // the ws echo repaints too; this is the sure path for a socket that is down
        self.on_pin(platform, &chan, if set { pin } else { None });
        self.say(if set { t("app.pin.pinned") } else { t("app.pin.unpinned") });
      }
      Err(Fail::Auth) => self.say(t("app.pin.login")),
      Err(Fail::Relink) => {
        self.say(t("app.pin.link"));
        self.net.send(Cmd::Relink("mod"));
      }
      Err(Fail::Denied) => self.say(t("app.pin.denied")),
      Err(Fail::NotArchived) => self.say(t("app.pin.not_archived")),
      Err(Fail::Other) => self.say(t("app.pin.failed")),
    }
    self.wake();
  }
}
