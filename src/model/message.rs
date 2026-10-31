// one chat line, platform-neutral. body is the user's text; emote spans index into it
use std::ops::Range;

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
  // our own line, echoed from USERSTATE (no emote positions: resolve twitch globals by name)
  pub const OWN: u8 = 1 << 4;
}

// image url for a byte range of the body
pub type Placed = (Range<u32>, Box<str>);
// (word, url, zero-width)
pub type HsEmote = (Box<str>, Box<str>, bool);

#[derive(Clone, Debug)]
pub struct Message {
  pub id: Box<str>,
  pub ts: i64,
  pub platform: Platform,
  pub kind: Kind,
  pub flags: u8,
  pub login: Box<str>,
  pub user_id: Box<str>,
  pub nick: Box<str>,
  pub color: u32,
  pub body: Box<str>,
  // platform-native emotes by byte range of body → image url (twitch tag, kick tokens, yt emoji)
  pub emotes: Box<[Placed]>,
  // heatsync emotes the server resolved from the SENDER's own inventory: word → (url, zero-width)
  pub hs: Box<[HsEmote]>,
  // (set, version) pairs from the twitch `badges=` tag
  pub badges: Box<[(Box<str>, Box<str>)]>,
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
      user_id: "".into(),
      nick: "".into(),
      color: 0,
      body: text.into(),
      emotes: Box::new([]),
      hs: Box::new([]),
      badges: Box::new([]),
      reply_to: None,
      system: None,
    }
  }

  pub fn has(&self, f: u8) -> bool {
    self.flags & f != 0
  }
}
