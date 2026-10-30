// one chat line, platform-neutral. body is the user's text; emote spans index into it
use std::ops::Range;

// kick + youtube arrive with the heatsync relay (m3)
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
  Twitch,
  Kick,
  Youtube,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
  Chat,
  // /me — body takes the name color
  Action,
  // server/system line (timeouts, subs, raids, notices)
  System,
}

pub mod flag {
  pub const CLEARED: u8 = 1;
  pub const FIRST: u8 = 1 << 1;
  pub const HIGHLIGHTED: u8 = 1 << 2;
  pub const HISTORY: u8 = 1 << 3;
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // platform + emotes feed the emote renderer (m2b)
pub struct Message {
  pub id: Box<str>,
  pub ts: i64,
  pub platform: Platform,
  pub kind: Kind,
  pub flags: u8,
  pub login: Box<str>,
  pub nick: Box<str>,
  pub color: u32,
  pub body: Box<str>,
  // platform-native emotes by byte range of body (twitch `emotes=` tag)
  pub emotes: Box<[(Range<u32>, Box<str>)]>,
  // `@name` the message replies to
  pub reply_to: Option<Box<str>>,
  // usernotice system text shown above the user's part (sub/raid/announcement)
  pub system: Option<Box<str>>,
}

impl Message {
  pub fn system(ts: i64, platform: Platform, text: impl Into<Box<str>>) -> Self {
    Message {
      id: "".into(),
      ts,
      platform,
      kind: Kind::System,
      flags: 0,
      login: "".into(),
      nick: "".into(),
      color: 0,
      body: text.into(),
      emotes: Box::new([]),
      reply_to: None,
      system: None,
    }
  }

  pub fn has(&self, f: u8) -> bool {
    self.flags & f != 0
  }
}
