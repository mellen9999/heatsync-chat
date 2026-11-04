// chat badges: twitch (global + per channel, keyed set/version) and third-party per-user sets
// (ffz, chatterino; 7tv lands with its event api). all images go through the url allowlist
pub mod fetch;
pub mod relay;

use std::collections::HashMap;

use crate::model::message::Message;

#[derive(Clone, Debug)]
pub struct Badge {
  pub url: Box<str>,
  // hover tooltip (lands with mouse hover)
  #[allow(dead_code)]
  pub title: Box<str>,
  // ffz badges are glyphs drawn over a colored tile
  pub bg: Option<u32>,
}

pub type TwitchSet = HashMap<Box<str>, Badge>;

#[derive(Default, Debug)]
pub struct UserBadges {
  pub list: Vec<Badge>,
  pub users: HashMap<u64, Vec<u16>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
  Bttv,
  Ffz,
  Chatterino,
}

// which per-chatter sets show (the ext's bttvBadges, ffzBadges, chatterinoBadges)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Third {
  pub bttv: bool,
  pub ffz: bool,
  pub chatterino: bool,
}

#[derive(Default)]
pub struct Badges {
  global: TwitchSet,
  channels: HashMap<Box<str>, TwitchSet>,
  // a channel's own ffz mod / vip art, over twitch's for those two (kept apart: either fetch can land first)
  ffz_room: HashMap<Box<str>, TwitchSet>,
  bttv: UserBadges,
  ffz: UserBadges,
  chatterino: UserBadges,
  // a kick channel's subscriber art by months, per tab (sorted by months). kick never falls back to
  // twitch's art: twitch versions are tiers, kick's are months
  kick: HashMap<Box<str>, Vec<(u32, Box<str>)>>,
}

// tabs whose kick art is kept: the ext's 20
const KICK_TABS: usize = 20;

pub fn key(set: &str, version: &str) -> Box<str> {
  format!("{set}/{version}").into()
}

impl Badges {
  pub fn set_twitch(&mut self, channel: Option<&str>, set: TwitchSet) {
    match channel {
      Some(c) => {
        self.channels.insert(c.into(), set);
      }
      None => self.global = set,
    }
  }

  pub fn set_ffz_room(&mut self, channel: &str, set: TwitchSet) {
    self.ffz_room.insert(channel.into(), set);
  }

  pub fn set_kick(&mut self, tab: &str, mut art: Vec<(u32, Box<str>)>) {
    art.sort_by_key(|a| a.0);
    if self.kick.len() >= KICK_TABS
      && !self.kick.contains_key(tab)
      && let Some(old) = self.kick.keys().next().cloned()
    {
      self.kick.remove(&old);
    }
    self.kick.insert(tab.into(), art);
  }

  // the picture for a sub of `months`: the exact tier, else the nearest lower one (the ext's
  // findNearestKickBadgeVersion); none when the channel has no art that low
  pub fn kick_sub(&self, tab: &str, months: u32) -> Option<&str> {
    let art = self.kick.get(tab)?;
    let i = art.partition_point(|a| a.0 <= months);
    art[..i].last().map(|a| &*a.1)
  }

  pub fn drop_channel(&mut self, channel: &str) {
    self.kick.remove(channel);
    self.channels.remove(channel);
    self.ffz_room.remove(channel);
  }

  pub fn set_users(&mut self, src: Source, b: UserBadges) {
    match src {
      Source::Bttv => self.bttv = b,
      Source::Ffz => self.ffz = b,
      Source::Chatterino => self.chatterino = b,
    }
  }

  // display order: twitch's own (tag order), then bttv, chatterino, ffz; each third-party set behind its
  // switch (`bttv`, `ffz`, `chatterino`)
  pub fn for_message<'a>(&'a self, channel: &str, m: &Message, on: Third) -> Vec<&'a Badge> {
    let ch = self.channels.get(channel);
    let room = self.ffz_room.get(channel);
    let mut out: Vec<&Badge> = m
      .badges
      .iter()
      .filter_map(|(s, v)| {
        let k = key(s, v);
        room.and_then(|r| r.get(&k)).or_else(|| ch.and_then(|c| c.get(&k))).or_else(|| self.global.get(&k))
      })
      .collect();
    out.extend(self.third_party(m, on));
    out
  }

  // the bttv, chatterino then ffz badges one chatter holds, each behind its switch
  pub fn third_party<'a>(&'a self, m: &Message, on: Third) -> Vec<&'a Badge> {
    let mut out = Vec::new();
    if let Ok(uid) = m.user_id.parse::<u64>() {
      let sets = [(on.bttv, &self.bttv), (on.chatterino, &self.chatterino), (on.ffz, &self.ffz)];
      for ub in sets.into_iter().filter(|s| s.0).map(|s| s.1) {
        if let Some(ix) = ub.users.get(&uid) {
          out.extend(ix.iter().filter_map(|&i| ub.list.get(i as usize)));
        }
      }
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::message::{Kind, Platform};

  fn b(url: &str) -> Badge {
    Badge { url: url.into(), title: "".into(), bg: None }
  }

  #[test]
  fn channel_overrides_global_then_third_party() {
    let mut bs = Badges::default();
    bs.set_twitch(None, [(key("subscriber", "0"), b("g-sub")), (key("moderator", "1"), b("g-mod"))].into());
    bs.set_twitch(Some("c"), [(key("subscriber", "0"), b("c-sub"))].into());
    bs.set_users(Source::Ffz, UserBadges { list: vec![b("ffz")], users: [(42, vec![0])].into() });
    let mut m = Message::system(0, Platform::Twitch, "x");
    m.kind = Kind::Chat;
    m.user_id = "42".into();
    m.badges =
      vec![("moderator".into(), "1".into()), ("subscriber".into(), "0".into()), ("nope".into(), "1".into())].into();
    let all = Third { bttv: true, ffz: true, chatterino: true };
    let urls: Vec<&str> = bs.for_message("c", &m, all).iter().map(|b| &*b.url).collect();
    assert_eq!(urls, ["g-mod", "c-sub", "ffz"]);
    bs.set_ffz_room("c", [(key("moderator", "1"), b("ffz-mod"))].into());
    let urls: Vec<&str> = bs.for_message("c", &m, all).iter().map(|b| &*b.url).collect();
    assert_eq!(urls, ["ffz-mod", "c-sub", "ffz"], "the channel's ffz mod art wins");
    assert_eq!(&*bs.for_message("other", &m, all)[0].url, "g-mod", "only in its own channel");
    let urls: Vec<&str> = bs.for_message("c", &m, Third { ffz: false, ..all }).iter().map(|b| &*b.url).collect();
    assert_eq!(urls, ["ffz-mod", "c-sub"], "ffz badges off: the channel's own ffz mod art stays");
  }

  #[test]
  fn bttv_badges_come_first_and_have_their_own_switch() {
    let mut bs = Badges::default();
    bs.set_users(Source::Bttv, UserBadges { list: vec![b("bttv")], users: [(42, vec![0])].into() });
    bs.set_users(Source::Chatterino, UserBadges { list: vec![b("chat")], users: [(42, vec![0])].into() });
    bs.set_users(Source::Ffz, UserBadges { list: vec![b("ffz")], users: [(42, vec![0])].into() });
    let mut m = Message::system(0, Platform::Twitch, "x");
    m.kind = Kind::Chat;
    m.user_id = "42".into();
    let all = Third { bttv: true, ffz: true, chatterino: true };
    let urls = |m: &Message, on| bs.third_party(m, on).iter().map(|b| b.url.to_string()).collect::<Vec<_>>();
    assert_eq!(urls(&m, all), ["bttv", "chat", "ffz"]);
    assert_eq!(urls(&m, Third { bttv: false, ..all }), ["chat", "ffz"], "bttv badges off");
    m.user_id = "43".into();
    assert!(urls(&m, all).is_empty(), "someone else holds none");
    m.user_id = "x".into();
    assert!(urls(&m, all).is_empty(), "no twitch id, no badge");
  }

  #[test]
  fn kick_sub_art_is_the_nearest_lower_month() {
    let mut bs = Badges::default();
    bs.set_kick("c", vec![(6, "six".into()), (1, "one".into()), (12, "twelve".into())]);
    let at = |m| bs.kick_sub("c", m);
    assert_eq!((at(1), at(5), at(6)), (Some("one"), Some("one"), Some("six")));
    assert_eq!((at(11), at(999)), (Some("six"), Some("twelve")));
    assert_eq!((at(0), bs.kick_sub("other", 3)), (None, None));
  }

  #[test]
  fn kick_art_is_kept_for_at_most_twenty_tabs() {
    let mut bs = Badges::default();
    for i in 0..40 {
      bs.set_kick(&format!("t{i}"), vec![(1, "x".into())]);
    }
    assert_eq!(bs.kick.len(), KICK_TABS);
    assert!(bs.kick_sub("t39", 1).is_some(), "the newest is kept");
  }
}
