// the name hover popup (ext tooltips.js user tooltip): the pointer resting on a sender's name, or on
// a name inside a line's words, opens a compact profile; it follows the pointer onto the next name
// and closes when the pointer leaves, the chat scrolls or a key goes down. a name already looked up
// shows at once; a cold one shows its name first and asks heatsync after 50ms of resting (scrolling
// past names costs nothing). answers are cached 60s and shared with the full card (ext _cardCache)
use crate::i18n::t;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::App;
use super::card::{Load, note_of, tag};
use crate::model::card::Info;
use crate::model::color;
use crate::model::message::{Message, Platform};
use crate::model::tabs::Kind;
use crate::net::{Cmd, social};
use crate::ui::canvas::Rect;
use crate::ui::card::View;
use crate::ui::chat_view::WhoHit;
use crate::ui::{peek, theme};

// how long the pointer rests on a cold name before heatsync is asked (ext: 50ms)
const REST: Duration = Duration::from_millis(50);
const TTL: Duration = Duration::from_secs(60);
// a failed answer is not asked again at once; a request with no answer yet is given this long
const FAILED_TTL: Duration = Duration::from_secs(10);
const LOADING_TTL: Duration = Duration::from_secs(15);
const CAP: usize = 300;

struct Entry {
  at: Instant,
  load: Load,
}

// what heatsync said about people, per platform, by lowercase login
#[derive(Default)]
pub struct Cards {
  by: [HashMap<Box<str>, Entry>; 3],
}

fn slot(p: Platform) -> usize {
  match p {
    Platform::Twitch => 0,
    Platform::Kick => 1,
    Platform::Youtube => 2,
  }
}

impl Cards {
  // the answer (or the wait for one) while it is fresh
  pub fn get(&self, p: Platform, login: &str) -> Option<&Load> {
    let e = self.by[slot(p)].get(login)?;
    let ttl = match e.load {
      Load::Done(_) => TTL,
      Load::Failed(_) => FAILED_TTL,
      Load::Loading => LOADING_TTL,
    };
    (e.at.elapsed() < ttl).then_some(&e.load)
  }

  // how long ago this person's entry was made (asked or answered)
  pub fn age(&self, p: Platform, login: &str) -> Option<Duration> {
    Some(self.by[slot(p)].get(login)?.at.elapsed())
  }

  // tests: make an entry older than it is
  #[cfg(test)]
  pub fn backdate(&mut self, p: Platform, login: &str, by: Duration) {
    if let Some(e) = self.by[slot(p)].get_mut(login) {
      e.at = e.at.checked_sub(by).unwrap_or(e.at);
    }
  }

  // drop what we hold on them, so the next `want` asks again
  pub fn forget(&mut self, p: Platform, login: &str) {
    self.by[slot(p)].remove(login);
  }

  // true = nobody has asked lately: the caller sends the request (and it is marked as waiting, so
  // a second hover or a color lookup for the same name does not ask again)
  pub fn want(&mut self, p: Platform, login: &str) -> bool {
    if matches!(self.get(p, login), Some(Load::Done(_) | Load::Loading)) {
      return false;
    }
    let map = &mut self.by[slot(p)];
    if map.len() >= CAP && !map.contains_key(login) {
      let old = map.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| k.clone());
      if let Some(k) = old {
        map.remove(&k);
      }
    }
    map.insert(login.into(), Entry { at: Instant::now(), load: Load::Loading });
    true
  }

  // the answer landed
  pub fn put(&mut self, p: Platform, login: &str, res: Result<Info, u16>) -> &Load {
    let load = match res {
      Ok(i) => Load::Done(i),
      Err(429) => Load::Failed(t("app.peek.slow")),
      Err(0) => Load::Failed(t("app.peek.no_answer")),
      Err(_) => Load::Failed(t("app.peek.failed")),
    };
    let map = &mut self.by[slot(p)];
    if map.len() >= CAP && !map.contains_key(login) {
      let old = map.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| k.clone());
      if let Some(k) = old {
        map.remove(&k);
      }
    }
    map.insert(login.into(), Entry { at: Instant::now(), load });
    &map[login].load
  }
}

// the open popup
// the picture beside the name (ext .hs-card-avatar)
const AVATAR_PX: u16 = 40;

pub struct Peek {
  platform: Platform,
  login: Box<str>,
  chan: Option<Box<str>>,
  nick: Box<str>,
  color: u32,
  // which name on which line (a second copy of the same person is a new spot: the popup moves)
  spot: (i64, usize),
  // the pointer when it appeared; the popup keeps still while the pointer stays on the name
  at: (i32, i32),
  // a mention's color may still arrive
  mention: bool,
  // the line's own twitch id (a sender's name only): who pronouns are asked about
  uid: Box<str>,
}

impl App {
  // the room a line belongs to, for "since when do they follow this channel"
  pub(super) fn room_of(&self, m: &Message) -> Option<Box<str>> {
    match m.x.as_deref().map(|x| &x.chan).filter(|c| !c.is_empty()) {
      Some(c) => Some(c.clone()),
      None => self.tabs.get(self.active)?.sources.iter().find(|s| s.0 == m.platform).map(|s| s.1.clone()),
    }
  }

  // the room of the tab on screen for a platform (twitch only: followage is per channel)
  pub(super) fn chan_for(&self, p: Platform) -> Option<Box<str>> {
    let t = self.tabs.get(self.active)?;
    t.sources.iter().find(|s| s.0 == p).map(|s| s.1.clone()).filter(|_| p == Platform::Twitch)
  }

  fn who_under_pointer(&mut self) -> Option<WhoHit> {
    if !self.tip_free() || self.picker.is_some() {
      return None;
    }
    let (x, y) = self.pointer;
    let chat = self.chat_area();
    if !chat.contains(x, y) {
      return None;
    }
    let res = &mut self.res;
    let t = self.tabs.get_mut(self.active).filter(|t| t.kind == Kind::Chat)?;
    t.view.who_at(chat, x, y, res)
  }

  // the pointer moved. true = it is on a name (the popup is up or coming), so no emote popup
  pub(super) fn peek_hover(&mut self) -> bool {
    let Some(hit) = self.who_under_pointer() else {
      if self.peek.is_some() {
        self.hide_peek();
      }
      return false;
    };
    let spot = (hit.seq, hit.start);
    if self.peek.as_ref().is_some_and(|p| p.spot == spot) {
      return true;
    }
    let Some(m) = self.tabs.get(self.active).and_then(|t| t.view.message(hit.seq)) else { return false };
    if m.login.is_empty() {
      return false;
    }
    let (platform, chan) = (m.platform, self.room_of(m).filter(|_| m.platform == Platform::Twitch));
    let (login, nick, color, mention, uid) = match hit.mention {
      Some(n) => (n.login.clone(), n.login, color::readable(n.color), true, Box::default()),
      None => (m.login.clone(), m.nick.clone(), color::readable(m.color), false, m.user_id.clone()),
    };
    if let Some(mut t) = self.tip.take() {
      t.release(&mut self.res);
      self.tip_next = None;
    }
    // cached: nothing to wait for. cold: rest 50ms first, then ask
    self.peek_due = match self.cards.get(platform, &login) {
      Some(Load::Done(_) | Load::Loading) => None,
      _ => Some(Instant::now() + REST),
    };
    self.peek = Some(Peek { platform, login, chan, nick, color, spot, at: self.pointer, mention, uid });
    if self.peek_due.is_none() {
      self.want_pronouns(self.peek_twitch_id());
    }
    self.tip_repaint();
    true
  }

  pub(super) fn peek_twitch_id(&self) -> Option<u64> {
    let p = self.peek.as_ref()?;
    let info = match self.cards.get(p.platform, &p.login) {
      Some(Load::Done(i)) => Some(i),
      _ => None,
    };
    super::pronouns::twitch_id(p.platform, &p.uid, info)
  }

  // the pointer rested long enough: ask
  pub(super) fn peek_fire(&mut self) {
    self.peek_due = None;
    let Some(p) = &self.peek else { return };
    let (platform, login, channel) = (p.platform, p.login.clone(), p.chan.clone());
    if self.cards.want(platform, &login) {
      self.net.send(Cmd::Social(social::Cmd::Card { platform, login, channel }));
    }
    self.want_pronouns(self.peek_twitch_id());
  }

  pub(super) fn hide_peek(&mut self) {
    self.peek_due = None;
    if self.peek.take().is_some() {
      self.tip_repaint();
    }
  }

  // an answer landed for someone: a popup waiting on them fills in; a mention's color may have come
  pub(super) fn peek_changed(&mut self, platform: Platform, login: &str) {
    let Some(p) = self.peek.as_mut().filter(|p| p.platform == platform && *p.login == *login) else { return };
    if p.mention
      && let Some(c) = self.res.chatters.person(platform, login, true)
    {
      p.color = color::readable(c.color);
    }
    self.want_pronouns(self.peek_twitch_id());
    self.tip_repaint();
  }

  // painted over everything, with the emote popup
  pub(super) fn draw_peek(&mut self) -> Option<Rect> {
    let bounds = self.chat_area();
    let pronouns = self.pronouns_of(self.peek_twitch_id()).map(str::to_owned);
    let plus = self.peek.as_ref().map(|p| (p.platform, p.uid.clone())).and_then(|(p, uid)| {
      let twitch = self.peek_twitch_id();
      self.plus_ink(p, &uid, twitch)
    });
    let p = self.peek.as_ref()?;
    let load = self.cards.get(p.platform, &p.login);
    let (rows, avatar): (&[(&'static str, String)], _) = match load {
      Some(Load::Done(i)) => (&i.rows, i.avatar.as_deref()),
      _ => (&[], None),
    };
    let note = load.map_or(Some(t("app.card.looking_up")), note_of);
    let v = View {
      hint: "",
      nick: &p.nick,
      color: if p.color == 0 { theme::WHITE } else { p.color },
      pronouns: pronouns.as_deref(),
      plus,
      tag: tag(p.platform),
      rows,
      note,
      mine: None,
      recent: &[],
    };
    let crate::ui::res::Res { text, images, .. } = &mut self.res;
    let side = images.px(AVATAR_PX) as i32;
    let id = avatar.map(|u| images.id(u, AVATAR_PX));
    let pic = id.and_then(|id| images.frame(id)).map(|(f, w, h, _)| (&f.rgba[..], w, h));
    Some(peek::draw(&mut self.canvas, text, bounds, p.at, &v, pic, side))
  }

  // a lookup answered: share it (popup, full card, mention colors)
  pub(super) fn learn_card(&mut self, platform: Platform, login: &str, res: Result<Info, u16>) {
    if let Ok(i) = &res {
      // their heatsync color, else twitch's hash palette (ext hsResolveUserColor step 3)
      let c = i.color.unwrap_or_else(|| color::default_for(login));
      use crate::cosmetics::id::PaintId;
      let paint = match platform {
        Platform::Twitch => i.twitch_id.map(PaintId::Twitch),
        Platform::Kick => i.kick_id.map(PaintId::Kick),
        Platform::Youtube => i.youtube_id.clone().map(PaintId::Youtube),
      };
      self.res.chatters.learned(platform, login, c, paint);
      for v in self.tabs.iter_mut().flat_map(|t| t.views()) {
        v.touch_person(platform, login);
      }
      self.restyle_stream_lines();
    }
    self.cards.put(platform, login, res);
    self.peek_changed(platform, login);
  }

  // names @-mentioned in lines that nobody has spoken for: one batch of lookups per frame, each
  // name once (deduped with hovers by the cache)
  pub(super) fn ask_people(&mut self) {
    for (platform, login) in self.res.chatters.take_wanted() {
      if self.cards.want(platform, &login) {
        let channel = self.chan_for(platform);
        self.net.send(Cmd::Social(social::Cmd::Card { platform, login, channel }));
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn one_request_per_person_until_it_answers_or_fails() {
    let mut c = Cards::default();
    assert!(c.want(Platform::Twitch, "bob"), "cold: ask");
    assert!(!c.want(Platform::Twitch, "bob"), "waiting: a second hover or a color lookup does not ask again");
    assert!(c.want(Platform::Kick, "bob"), "another platform is another person");
    c.put(Platform::Twitch, "bob", Ok(Info::default()));
    assert!(matches!(c.get(Platform::Twitch, "bob"), Some(Load::Done(_))));
    assert!(!c.want(Platform::Twitch, "bob"), "fresh answers are reused");
    // a failure shows its reason and may be asked again
    c.put(Platform::Twitch, "amy", Err(429));
    assert!(matches!(c.get(Platform::Twitch, "amy"), Some(Load::Failed("slow down, try again in a moment"))));
    assert!(c.want(Platform::Twitch, "amy"));
  }

  #[test]
  fn the_cache_stays_bounded() {
    let mut c = Cards::default();
    for i in 0..CAP + 50 {
      c.want(Platform::Twitch, &format!("user{i}"));
      c.put(Platform::Twitch, &format!("p{i}"), Ok(Info::default()));
    }
    assert!(c.by[0].len() <= CAP);
  }
}
