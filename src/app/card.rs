// the user card (ext profile-card.js): click a name, or `c` on a selected line. name, what heatsync
// knows (one request, shown when it lands), what they said here this session, and actions (mention,
// whisper, mute, block, and the mod actions when you moderate the room). esc closes
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::pop::{Act, Subject};
use crate::model::card::Info;
use crate::model::message::Platform;
use crate::model::tabs::Kind;
use crate::net::{Cmd, social};
use crate::ui::canvas::Rect;
use crate::ui::card as view;
use crate::ui::menu::Menu;
use crate::util::time::hhmm;

const RECENT: usize = 6;

pub(super) enum Load {
  Loading,
  Done(Info),
  Failed(&'static str),
}

// what a card says in place of rows while there are none to show
pub(super) fn note_of(l: &Load) -> Option<&'static str> {
  match l {
    Load::Loading => Some("looking them up…"),
    Load::Failed(why) => Some(why),
    Load::Done(i) if !i.registered => Some("not on heatsync"),
    Load::Done(_) => None,
  }
}

pub struct Card {
  who: Subject,
  info: Load,
  recent: Vec<(i64, Box<str>)>,
  menu: Menu<Act>,
  // where it was last drawn: a click outside it closes it
  rect: Rect,
}

impl Card {
  fn twitch_id(&self) -> Option<u64> {
    let info = match &self.info {
      Load::Done(i) => Some(i),
      _ => None,
    };
    super::pronouns::twitch_id(self.who.platform, &self.who.user_id, info)
  }
}

pub(super) fn tag(p: Platform) -> (&'static str, u32) {
  match p {
    Platform::Twitch => ("[T] ", crate::ui::theme::TWITCH),
    Platform::Kick => ("[K] ", crate::ui::theme::GREEN),
    Platform::Youtube => ("[Y] ", crate::ui::theme::RED),
  }
}

impl App {
  pub(super) fn open_card(&mut self, who: Subject) {
    let Some(t) = self.tabs.get(self.active) else { return };
    let recent = t.view.recent_by(&who.login, RECENT);
    // the room it was opened from: followage is "since when do they follow this channel". a fresh
    // answer from a hover is reused (ext _cardCache), else ask
    let channel = (who.platform == Platform::Twitch).then(|| who.chan.clone());
    let info = match self.cards.get(who.platform, &who.login) {
      Some(Load::Done(i)) => Load::Done(i.clone()),
      Some(Load::Loading) => Load::Loading,
      _ => {
        if self.cards.want(who.platform, &who.login) {
          self.net.send(Cmd::Social(social::Cmd::Card { platform: who.platform, login: who.login.clone(), channel }));
        }
        Load::Loading
      }
    };
    let menu = Menu::new(self.card_rows(&who, &info), (0, 0));
    self.hide_peek();
    self.card = Some(Card { who, info, recent, menu, rect: Rect::new(0, 0, 0, 0) });
    self.pop = None;
    self.want_pronouns(self.card_twitch_id());
    self.card_changed();
  }

  // the ext's card has its logs cell first, then follow / unfollow once heatsync says which
  fn card_rows(&self, who: &Subject, info: &Load) -> Vec<(Act, String)> {
    let mut rows = vec![(Act::Logs, "chat logs".to_owned())];
    if let Load::Done(i) = info
      && let Some(f) = i.follow.filter(|_| self.me.is_some() && !who.own)
    {
      rows.push((Act::Follow(!f), if f { "unfollow" } else { "follow" }.into()));
    }
    rows.extend(self.person_rows(who));
    rows
  }

  // the open card read again from the cache (a follow changed it): its facts and its rows
  pub(super) fn card_refresh(&mut self, platform: Platform, login: &str) {
    let Some(c) = self.card.as_ref().filter(|c| c.who.platform == platform && *c.who.login == *login) else { return };
    let info = match self.cards.get(platform, login) {
      Some(Load::Done(i)) => Load::Done(i.clone()),
      Some(Load::Failed(why)) => Load::Failed(why),
      _ => Load::Loading,
    };
    let rows = self.card_rows(&c.who, &info);
    if let Some(c) = self.card.as_mut() {
      c.info = info;
      c.menu.at = c.menu.at.min(rows.len().saturating_sub(1));
      c.menu.items = rows;
    }
    self.card_changed();
  }

  pub(super) fn card_twitch_id(&self) -> Option<u64> {
    self.card.as_ref().and_then(Card::twitch_id)
  }

  pub(super) fn on_card(&mut self, platform: Platform, login: &str, res: Result<Info, u16>) {
    if let Ok(i) = &res {
      self.note_sync(platform, login, i);
    }
    self.learn_card(platform, login, res);
    self.card_refresh(platform, login);
    self.want_pronouns(self.card_twitch_id());
  }

  pub(super) fn card_changed(&mut self) {
    if let Some(t) = self.tabs.get_mut(self.active) {
      t.view.invalidate();
    }
    self.wake();
  }

  // a click on a name opens the card. true = it was a name
  pub(super) fn name_press(&mut self) -> bool {
    if !matches!(self.tabs.get(self.active).map(|t| t.kind), Some(Kind::Chat)) {
      return false;
    }
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    if !chat.contains(x, y) {
      return false;
    }
    let res = &mut self.res;
    let Some(t) = self.tabs.get_mut(self.active) else { return false };
    let Some(hit) = t.view.who_at(chat, x, y, res) else { return false };
    t.view.select_at(chat, y, res);
    let Some(m) = self.tabs.get(self.active).and_then(|t| t.view.message(hit.seq)) else { return false };
    let who = match &hit.mention {
      None => self.subject_of(m),
      Some(n) => self.subject_for(m, &n.login, &n.login, "", n.color, ""),
    };
    match who {
      Some(who) => {
        self.open_card(who);
        true
      }
      None => false,
    }
  }

  // `c` on a selected line
  pub(super) fn card_selected(&mut self) {
    let who = self.tabs.get(self.active).and_then(|t| t.view.selected()).and_then(|m| self.subject_of(m));
    if let Some(w) = who {
      self.open_card(w);
    }
  }

  pub(super) fn card_key(&mut self, key: Key<&str>) {
    let Some(c) = &mut self.card else { return };
    match key {
      Key::Character("j") | Key::Named(NamedKey::ArrowDown) => c.menu.step(1),
      Key::Character("k") | Key::Named(NamedKey::ArrowUp) => c.menu.step(-1),
      Key::Named(NamedKey::Enter) => return self.card_pick(),
      Key::Named(NamedKey::Escape) | Key::Character("q") => self.card = None,
      _ => return,
    }
    self.card_changed();
  }

  fn card_pick(&mut self) {
    let Some(c) = self.card.take() else { return };
    self.card_changed();
    if let Some(act) = c.menu.chosen() {
      self.act(act, Some(c.who), None);
    }
  }

  pub(super) fn card_click(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    let Some(c) = &mut self.card else { return };
    if let Some(i) = c.menu.row_at(x, y, line) {
      c.menu.at = i;
      return self.card_pick();
    }
    // inside the box but not on a row: stay; outside: close
    if !c.rect.contains(x, y) {
      self.card = None;
    }
    self.card_changed();
  }

  pub(super) fn card_hover(&mut self) {
    let line = self.res.text.line_px as i32;
    let (x, y) = self.pointer;
    if let Some(c) = &mut self.card
      && let Some(i) = c.menu.row_at(x, y, line)
      && i != c.menu.at
    {
      c.menu.at = i;
      self.wake();
    }
  }

  pub(super) fn draw_card(&mut self) -> Option<Rect> {
    let area = self.chat_area();
    let mine = self.card.as_ref().and_then(|c| self.note_of(c.who.platform, &c.who.login));
    let pronouns = self.pronouns_of(self.card_twitch_id()).map(str::to_owned);
    let c = self.card.as_mut()?;
    let note = note_of(&c.info);
    let rows: &[(&'static str, String)] = match &c.info {
      Load::Done(i) => &i.rows,
      _ => &[],
    };
    let recent: Vec<(String, &str)> = c.recent.iter().map(|(ts, body)| (hhmm(*ts), &**body)).collect();
    let v = view::View {
      hint: "esc closes",
      nick: &c.who.nick,
      color: c.who.color,
      pronouns: pronouns.as_deref(),
      tag: tag(c.who.platform),
      rows,
      note,
      mine: mine.as_deref(),
      recent: &recent,
    };
    let r = view::draw(&mut self.canvas, &mut self.res.text, area, &v, &mut c.menu);
    c.rect = r;
    Some(r)
  }
}
