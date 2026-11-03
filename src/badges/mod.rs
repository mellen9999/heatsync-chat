// chat badges: twitch (global + per channel, keyed set/version) and third-party per-user sets
// (ffz, chatterino; 7tv lands with its event api). all images go through the url allowlist
pub mod fetch;

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
  Ffz,
  Chatterino,
}

#[derive(Default)]
pub struct Badges {
  global: TwitchSet,
  channels: HashMap<Box<str>, TwitchSet>,
  // a channel's own ffz mod / vip art, over twitch's for those two (kept apart: either fetch can land first)
  ffz_room: HashMap<Box<str>, TwitchSet>,
  ffz: UserBadges,
  chatterino: UserBadges,
}

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

  pub fn drop_channel(&mut self, channel: &str) {
    self.channels.remove(channel);
    self.ffz_room.remove(channel);
  }

  pub fn set_users(&mut self, src: Source, b: UserBadges) {
    match src {
      Source::Ffz => self.ffz = b,
      Source::Chatterino => self.chatterino = b,
    }
  }

  // display order: twitch's own (tag order), then chatterino, then ffz
  pub fn for_message<'a>(&'a self, channel: &str, m: &Message) -> Vec<&'a Badge> {
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
    if let Ok(uid) = m.user_id.parse::<u64>() {
      for ub in [&self.chatterino, &self.ffz] {
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
    let urls: Vec<&str> = bs.for_message("c", &m).iter().map(|b| &*b.url).collect();
    assert_eq!(urls, ["g-mod", "c-sub", "ffz"]);
    bs.set_ffz_room("c", [(key("moderator", "1"), b("ffz-mod"))].into());
    let urls: Vec<&str> = bs.for_message("c", &m).iter().map(|b| &*b.url).collect();
    assert_eq!(urls, ["ffz-mod", "c-sub", "ffz"], "the channel's ffz mod art wins");
    assert_eq!(&*bs.for_message("other", &m)[0].url, "g-mod", "only in its own channel");
  }
}
