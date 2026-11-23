// one chat line, platform-neutral. body is the user's text; emote spans index into it
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
  Twitch,
  Kick,
  Youtube,
}

impl Platform {
  // the most characters one line may carry there (ext send-targets.js CHAT_MAX_LEN)
  pub const fn max_chars(self) -> usize {
    match self {
      Platform::Twitch | Platform::Kick => 500,
      Platform::Youtube => 200,
    }
  }

  // "twitch" | "kick" | "youtube" ("yt" too): how the server and the lists spell a platform
  pub fn from_name(s: &str) -> Option<Self> {
    match s {
      "twitch" => Some(Platform::Twitch),
      "kick" => Some(Platform::Kick),
      "youtube" | "yt" => Some(Platform::Youtube),
      _ => None,
    }
  }

  pub fn name(self) -> &'static str {
    match self {
      Platform::Twitch => "twitch",
      Platform::Kick => "kick",
      Platform::Youtube => "youtube",
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
  Chat,
  // /me — body takes the name color
  Action,
  // server/system line (timeouts, subs, raids, notices)
  System,
  // a stream line (went live, switched game, offline): drawn under the streamer's name like a /me,
  // but nobody said it, so it is never picked, replied to, moderated, logged or counted as chat
  Event,
}

impl Kind {
  // a person's line, as opposed to a notice or a stream line
  pub fn said(self) -> bool {
    matches!(self, Kind::Chat | Kind::Action)
  }
}

pub mod flag {
  pub const CLEARED: u16 = 1;
  pub const FIRST: u16 = 1 << 1;
  pub const HIGHLIGHTED: u16 = 1 << 2;
  pub const HISTORY: u16 = 1 << 3;
  // our own line, echoed from USERSTATE (no emote positions: resolve twitch globals by name)
  pub const OWN: u16 = 1 << 4;
  // first line from this person in this tab this session (the glow)
  pub const NEW: u16 = 1 << 5;
  // red edge: the line mentions you (the ext's mention tint, as an edge) or a send that failed
  pub const ALERT: u16 = 1 << 6;
  // heatsync-orange edge: something you sent (whispers)
  pub const BRAND: u16 = 1 << 7;
  // with CLEARED: the grey is a deleted line (stays grey), not a ban or timeout (an unban lifts it)
  pub const DELETED: u16 = 1 << 8;
}

// what kind of event a notice row is (ext main.js noticeKind): its words take the kind's color, bold,
// and the row the same color down its left (the ext's ring). the ui owns the colors
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Notice {
  #[default]
  Plain,
  // sub, resub, gifts (twitch and kick)
  Sub,
  Raid,
  // a bits badge tier, a charity gift, kick's KICKs
  Bits,
  // a mod's anniversary (twitch modiversary)
  ModAnniv,
  Announce,
  Streak,
  Milestone,
  // someone's line sent with a channel-point reward
  Redeem,
}

// a run of text with its own ink, drawn around the name (feed ids/heat, whisper arrows)
#[derive(Clone, Debug)]
pub struct Seg {
  pub text: Box<str>,
  pub ink: u32,
  pub bold: bool,
}

// what only some lines carry, boxed so ordinary chat pays one pointer
#[derive(Clone, Debug, Default)]
pub struct Extra {
  // the room the line came from (mentions tab): resolves its emotes/badges and shows as a tag
  pub chan: Box<str>,
  // drawn before the name / between the name and the colon
  pub head: Box<[Seg]>,
  pub tail: Box<[Seg]>,
  // left edge color when no flag asks for one (feed posts: the heat tier); 0 = none
  pub edge: u32,
  // whose paint the name wears when the line's own platform/user_id can't say (a feed post by a kick author)
  pub paint: Option<crate::cosmetics::id::PaintId>,
  // a feed post's media (model::post::pic)
  pub pic: Option<Box<Pic>>,
  // a feed post's reaction chips, drawn on a line of their own under it (model::reaction)
  pub react: Box<[crate::model::reaction::Reaction]>,
  // a stream line whose name color the frame didn't give: it follows what the app learns of them later
  pub late: bool,
  // the same line's streamer id (paint, `+`, badges) still unknown: it is filled in when a lookup lands
  pub late_id: bool,
  // an automod hold's row: its allow / deny, or how it ended
  pub held: Option<Box<crate::model::held::Row>>,
  // the event a notice row tells of (its system line's color)
  pub notice: Notice,
  // a cheer: the `N bits` chip before the badges
  pub bits: u32,
  // what the sender's own app said a word of this line is (their pick: tab, the picker). it draws over the
  // room's emote of that name. an empty url is a pick that was taken down: the word stays a word
  pub picks: Box<[HsEmote]>,
  // your own line sent through heatsync.org (model::origin): tagged [H] in heatsync orange
  pub origin: bool,
  // the send id heatsync gave the line (`chat:origin`): rows of one send on several feeds share it
  pub send: Box<str>,
  // the head already wears the row's one mark (a whisper's [T]/[H], a notification's [H]): no platform tag over it
  pub branded: bool,
  // the line it answers and the first line of its chain (model::chain); empty = none
  pub parent: Box<str>,
  pub root: Box<str>,
  // a went-live line: a click or enter on it opens its channel (its tab, else the temp tab)
  pub opens: bool,
  // twitch shared chat: the room the line was said in when that is not this one (its numeric room id),
  // and the line's id there. every room in a session gets the same line under the same source id
  pub shared: Box<str>,
  pub source: Box<str>,
  // a power-up: the message effect's name (`simmer`), and a gigantified last emote
  pub fx: Box<str>,
  pub giant: bool,
  // msg-id=user-intro (someone introducing themselves: the `new here` chip) and returning-chatter=1
  pub intro: bool,
  pub returning: bool,
  // a person timed out or banned: the row joins the run of them just above it (model::runs)
  pub run: Option<Box<crate::model::runs::Run>>,
  // a feed post's bold and underlined runs, in bytes of the body (model::markup)
  pub marks: Box<[crate::model::markup::Mark]>,
}

// one picture (a video's still) and the video it plays
pub type PicItem = (Box<str>, Option<Box<str>>);

// a feed post's media as its row shows it: a tag after the words, and in an open thread the pictures
// under them, or the held box with its button
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pic {
  // `[img]` `[gif]` `[vid]` `[3 img]`
  pub tag: Box<str>,
  // what the tag peeks on hover; None = nothing to peek (still in moderation, or withheld)
  pub peek: Option<Box<str>>,
  // drawn under the words (an open thread); else the tag alone
  pub inline: bool,
  pub items: Box<[PicItem]>,
  // the held box's words (`[media hidden — nsfw]`); None = in the clear
  pub hidden: Option<Box<str>>,
  // its button: show, hide, retry, …; None = no button (csam)
  pub button: Option<Box<str>>,
}

// a reply keeps this much of its parent's words (one line of it shows)
pub const REPLY_TEXT: usize = 300;

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
  pub flags: u16,
  pub login: Box<str>,
  pub user_id: Box<str>,
  pub nick: Box<str>,
  pub color: u32,
  pub body: Box<str>,
  // platform-native emotes by byte range of body → image url (twitch tag, kick tokens, yt emoji)
  pub emotes: Box<[Placed]>,
  // heatsync emotes the server matched to the SENDER's words by name, from their own inventory: word →
  // (url, zero-width). a name-match is not a pick: it draws only where the room has no emote of that name
  // (a pick is `Extra::picks`). a feed post's refs are here too: a post has no room, so the two read alike
  pub hs: Box<[HsEmote]>,
  // (set, version) pairs from the twitch `badges=` tag
  pub badges: Box<[(Box<str>, Box<str>)]>,
  // `@name` the message replies to, and the start of that line's words
  pub reply_to: Option<Box<str>>,
  pub reply_text: Option<Box<str>>,
  // usernotice system text shown above the user's part (sub/raid/announcement)
  pub system: Option<Box<str>>,
  pub x: Option<Box<Extra>>,
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
      reply_text: None,
      system: None,
      x: None,
    }
  }

  pub fn has(&self, f: u16) -> bool {
    self.flags & f != 0
  }

  // the Extra, made when a line first needs one
  pub fn extra(&mut self) -> &mut Extra {
    self.x.get_or_insert_with(Box::default)
  }

  pub fn notice(&self) -> Notice {
    self.x.as_ref().map_or(Notice::Plain, |x| x.notice)
  }
}
