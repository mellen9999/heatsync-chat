// all network io lives on one background thread running a current-thread tokio runtime.
// the ui talks to it through `Net` and hears back through winit user events
pub mod auth;
mod avatars;
pub mod blocked;
mod catalog;
pub mod chantools;
mod clip;
mod cosmetics;
pub mod dmdel;
pub mod dupe;
pub mod emote_ops;
pub mod following;
mod font;
pub mod held;
pub mod hs_ws;
pub mod http;
pub mod import;
pub mod inventory;
pub mod irc;
mod lanes;
mod links;
mod live_dots;
pub mod marks;
pub mod modact;
pub mod modes;
pub mod modsuite;
pub mod modtools;
pub mod mrules;
pub mod mutes;
mod origin;
mod personal;
pub mod pin;
pub mod pinact;
pub mod pred;
pub mod pushed;
mod recent;
pub mod relay;
mod retry;
mod roomname;
mod senders;
mod session;
pub mod seventv;
pub mod social;
pub mod twclip;
pub mod update;
pub mod upload;
mod youtube;
pub(crate) mod ytdel;

use crate::model::ident;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, mpsc};
use winit::event_loop::EventLoopProxy;

use crate::badges::{self, Source, TwitchSet, UserBadges};
use crate::emotes::{Provider, Set, providers};
use crate::heatsync::senders::Set as SenderSet;
use crate::images::{ImgId, cache, decode, url};
use crate::model::message::{Message, Platform};
use irc::Chat;

pub const USER_AGENT: &str = concat!("heatsync-chat/", env!("CARGO_PKG_VERSION"));
pub const ORIGIN: &str = "https://heatsync.org";

pub enum NetEvent {
  Chat(Box<str>, Chat),
  // recent history for a freshly joined channel, oldest first
  Backfill(Box<str>, Vec<Chat>),
  Status(String),
  // lines that landed while a feed was down, fetched once it's back (twitch irc dropped, a youtube
  // subscription resumed): the app slots them in by time among what's shown and skips what it has
  CatchUp {
    platform: Platform,
    key: Box<str>,
    msgs: Vec<Message>,
  },
  // channel = None for globals
  Emotes(Option<Box<str>>, Provider, Set),
  Image(ImgId, u32, Result<decode::Decoded, String>),
  TwitchBadges(Option<Box<str>>, TwitchSet),
  // a channel's own ffz mod / vip badge art
  FfzRoomBadges(Box<str>, TwitchSet),
  // a kick channel's subscriber art by months, keyed by kick slug
  KickBadges(Box<str>, Vec<(u32, Box<str>)>),
  // kick/youtube lines relayed by heatsync, keyed by kick slug / youtube handle
  Relay {
    platform: Platform,
    key: Box<str>,
    msgs: Vec<Message>,
    history: bool,
  },
  // a person removed from a relayed room: timed out for `secs`, or banned (None)
  RelayClear {
    platform: Platform,
    key: Box<str>,
    login: Box<str>,
    secs: Option<u32>,
  },
  // youtube lines a mod deleted (by id), or all a banned author said (by channel id)
  YtGone {
    key: Box<str>,
    gone: ytdel::Gone,
  },
  // a send on kick was refused for a reason that lasts, or went through (None): the box keeps saying why
  Lock(Platform, Box<str>, Option<crate::model::lock::Cause>),
  YtStatus {
    key: Box<str>,
    status: Box<str>,
  },
  // a twitch tab's publicly linked accounts: kick slug, youtube (handle, UC id)
  Links {
    tab: Box<str>,
    kick: Option<Box<str>>,
    yt: Option<(Box<str>, Option<Box<str>>)>,
  },
  UserBadges(Source, UserBadges),
  // heatsync inventories for twitch user ids
  Senders(Vec<(u64, SenderSet)>),
  // heatsync's refs for a twitch line (by its id), from the server's own copy of it
  HsRefs {
    chan: Box<str>,
    id: Box<str>,
    refs: Box<[crate::model::message::HsEmote]>,
  },
  // a platform was linked or unlinked (or the socket came back): read the account again
  AccountChanged,
  SendersStale {
    ids: Vec<u64>,
    ver: Option<Box<str>>,
    removed: Option<Box<str>>,
  },
  // logged in as / logged out
  Me(Option<auth::Me>),
  // a sign-in is waiting on the browser (esc gives it up) / is not any more
  LoginWait(bool),
  // the server's blocked-emote hashes for this login
  BlockedHashes(Vec<Box<str>>),
  // your twitch sub emotes (empty: none, or logged out)
  SubEmotes(crate::emotes::Set),
  // our own heatsync inventory changed (an add, or a removal of `removed`): refetch it
  Inventory {
    removed: Option<Box<str>>,
  },
  // the inventory screen's ask answered, or why not
  Inv {
    op: inventory::Op,
    res: Result<inventory::Got, String>,
  },
  Upload(upload::Upload),
  // what the clipboard held when asked (the key's target: the picker's search box or the line), or why
  // it couldn't say
  Pasted {
    to_picker: bool,
    got: Result<crate::util::clip::Paste, String>,
  },
  // the font a family name means, or why there is none (as a sentence); `asked` is the name it answers
  Font {
    asked: Box<str>,
    found: Result<crate::util::font::Found, String>,
  },
  // a second launch asked this window to come forward, with any channels it was given
  Raise(Vec<Box<str>>),
  // a chatter's picture url (None = none found)
  Avatar(Platform, Box<str>, Option<Box<str>>),
  // a shared-chat room's channel name by its number (None = twitch knows no such room)
  RoomLogin(Box<str>, Option<Box<str>>),
  // 7tv paints + badges for twitch ids (None = wears nothing)
  Cosmetics(Vec<(crate::cosmetics::id::PaintId, Option<crate::cosmetics::Cosmetic>)>),
  // 7tv says this twitch user's cosmetics changed: ask again
  CosmeticsStale(crate::cosmetics::id::PaintId),
  // chatters' heat by login (an answer, or the server's push)
  Heat(Vec<(Box<str>, u32)>),
  // chatters' sub flair by twitch id (None = wears none)
  Flair(Vec<(u64, Option<crate::badges::Badge>)>),
  // heatsync says these twitch ids changed their paint: ask again
  CosmeticsChanged(Vec<crate::cosmetics::id::PaintId>),
  // a live edit of a channel's 7tv emote set
  EmoteDelta(Box<str>, crate::emotes::live::Delta),
  // 7tv personal emotes: who owns which set, a set read whole, a live edit of one
  Personal(crate::emotes::personal::Ev),
  // a channel's pinned line was set or cleared (None)
  Pin {
    platform: Platform,
    key: Box<str>,
    pin: Option<crate::model::pin::Pin>,
  },
  // went live / offline / switched game, already paired and de-duplicated
  Stream(crate::model::stream_events::Line),
  // `follow:history`: switched / offline lines of the last 15 min with their own times, oldest first
  StreamHistory(Vec<(i64, crate::model::stream_events::Line)>),
  // something you sent through heatsync.org, by its words (`chat:origin_broadcast`)
  Origin(Box<str>),
  // a chat-rate spike in a channel (text ready to show)
  Moment {
    platform: Platform,
    key: Box<str>,
    text: Box<str>,
  },
  // twitch whispers: one pushed live, or the history read when the tab first opened
  // `epoch` tags a history read with the login it was asked under (live pushes carry none)
  Whispers {
    items: Vec<crate::model::whisper::Whisper>,
    live: bool,
    epoch: Option<u32>,
  },
  // a send settled: who it went to (resolved), or why not
  // the feed: a page, why it failed (http status, 0 = no answer), a thread, a vote or a post settling,
  // and a post pushed live
  Feed(social::Page),
  FeedFailed {
    status: u16,
  },
  Thread {
    id: Box<str>,
    res: Result<(crate::model::post::Post, Vec<crate::model::post::Post>), u16>,
  },
  Media {
    id: Box<str>,
    res: Result<Box<[crate::model::post::Media]>, u16>,
  },
  // a release newer than this one is out
  Update(update::Release),
  // the install finished (restart now) or said why not
  Updated(Result<(), String>),
  Voted {
    id: Box<str>,
    res: Result<(bool, Option<u32>), u16>,
  },
  // a reaction press settled (`on` = what was asked); ok = whether the server added it
  Reacted {
    id: Box<str>,
    emote: u64,
    on: bool,
    res: Result<bool, u16>,
  },
  // a live reaction on a post of the thread open (ws `reaction:added` / `reaction:removed`), checked
  Reaction(crate::model::reaction::Frame),
  // a live vote on one (ws `vote:updated`): the post's heat, absolute
  PostHeat {
    id: Box<str>,
    heat: u32,
  },
  // who this socket is signed in as (ws `authenticated`): our own reaction frames are told apart by it
  HsUser(Box<str>),
  // a bookmark settled (`on` = what was asked), the check of which posts are bookmarked, an edit and a
  // delete of your own post settling
  Bookmarked {
    id: Box<str>,
    on: bool,
    res: Result<(), u16>,
  },
  Marks {
    ids: Vec<Box<str>>,
    res: Result<Vec<(Box<str>, bool)>, u16>,
  },
  Edited {
    id: Box<str>,
    res: Result<Box<str>, u16>,
  },
  Deleted {
    id: Box<str>,
    res: Result<(), u16>,
  },
  // you last looked at a tab at `at` (here at login, or on another client since)
  Seen {
    surface: crate::model::unseen::Surface,
    at: i64,
  },
  // a heatsync notification for you (ws `notification:new`), checked
  Notif(crate::model::inbox::Item),
  // your settings as the account holds them (only the keys the window maps)
  UiState(crate::model::sync::Inbound),
  // the heatsync socket is signed in (or is not, again)
  HsAuth(bool),
  // the account's tab list (ws `multichat:config`), checked; the app applies the rev gate
  TabSync(crate::model::tabsync::Frame),
  Posted {
    res: Result<crate::model::post::Post, u16>,
  },
  // a page of the chat-log viewer, or the http status that said why not (0 = no answer); `epoch` names the ask
  Logs {
    epoch: u32,
    res: Result<crate::model::logs::Page, u16>,
  },
  // the gifs tab's answer to the search `q` asked under `epoch`, or the http status that said why not
  Gifs {
    epoch: u32,
    q: Box<str>,
    res: Result<crate::model::gifs::Page, crate::model::gifs::Fail>,
  },
  FeedLive(crate::model::post::Post),
  // tab-complete's catalog search: one page from 7tv and ffz (None = that one failed or wasn't asked), tagged
  // with the ask it answers
  Catalog {
    epoch: u32,
    stv: Option<crate::model::catalog::Page>,
    ffz: Option<crate::model::catalog::Page>,
  },
  // the emote picker's catalog search: one page from 7tv, ffz and heatsync (None = failed or not asked)
  PickerSearch {
    epoch: u32,
    pages: [Option<crate::model::catalog::Page>; 3],
  },
  // followed streamers' twitch colors (lowercase login, rgb)
  FollowColors(Vec<(Box<str>, u32)>),
  // your mute / block lists on heatsync, read after a login
  Lists(mutes::Lists),
  // a mute changed on another device
  MuteSync(mutes::Sync),
  // heatsync pushes ("this changed"), already checked (net/pushed.rs)
  // a name's heatsync colour
  ProfileColor(pushed::ProfileColor),
  // a feed post another person's action took away / rewrote / renumbered / moderated
  PostHidden(Box<str>),
  PostEdited(pushed::Edit),
  PostMeta(pushed::Meta),
  PostModerated(Box<str>, pushed::Mod),
  // a post bookmarked / un-bookmarked on another device
  Bookmark(Box<str>, bool),
  // the account's unread notification count after a look elsewhere
  Unread(u32),
  // your plus moved: ask again how you look
  PlusMoved,
  // the site's flagged-content switches (ws `user_settings:update`, or the read at login), checked
  ContentPrefs(Vec<(crate::heatsync::content::Cat, bool)>),
  // the follows / blocked emotes / blocks changed on another device: read them again
  Refetch(crate::model::livegate::Ask),
  // your mention rules answered: the list, or a change made / why not
  MentionRules(Result<mrules::Got, mrules::Fail>),
  // the server saw a line match one of your mention rules
  RuleHit(crate::model::mrules::Hit),
  // a user card's facts (or why not: http status, 0 = no answer)
  Card {
    platform: Platform,
    login: Box<str>,
    res: Result<crate::model::card::Info, u16>,
  },
  // a note upload to heatsync settled: the text sent, and whether it took
  NoteSaved {
    platform: Platform,
    login: Box<str>,
    text: String,
    ok: bool,
  },
  // a mod action settled
  ModDone(Box<modact::Done>),
  // a twitch clip from the card's `clip` row settled (the card's person)
  Clip {
    login: Box<str>,
    res: Result<twclip::Made, twclip::Fail>,
  },
  // a /nuke settled
  NukeDone(Box<modact::NukeDone>),
  // a pin / unpin for everyone settled
  PinDone(Box<pinact::Done>),
  // a link's preview (None: nothing to show)
  Preview(Box<str>, Option<crate::model::preview::Preview>),
  // someone's pronouns (by twitch id): Ok(words or none set), Err = no answer
  Pronouns(u64, Result<Option<Box<str>>, ()>),
  // automod held a message in a room you moderate / a hold settled (any mod, or twitch's hour ran out)
  Held(Box<crate::model::held::Hold>),
  HeldEnd {
    chan: Box<str>,
    id: Box<str>,
    end: crate::model::held::End,
  },
  // the room's automod watch answered: what twitch still holds there
  HeldWatched {
    chan: Box<str>,
    res: Result<Vec<crate::model::held::Hold>, held::Fail>,
  },
  // an allow / deny settled
  HeldActed {
    chan: Box<str>,
    id: Box<str>,
    allow: bool,
    res: Result<(), held::Fail>,
  },
  // a #tag's hover card: Ok(None) = heatsync doesn't know it, Err = no answer
  TagCard(Box<str>, Result<Option<crate::model::tagcard::Card>, ()>),
  // who you follow is live: the full answer from heatsync, the socket's catch-up on sign-in (adds only),
  // and one go-live / game switch / sign-off of a followed channel
  Following(Result<Vec<crate::model::following::Live>, following::Fail>),
  // your twitch follows for /import
  Import(Result<import::Got, import::Fail>),
  // one live-status answer: per platform, the channels asked and who of them is live (None: no answer)
  LiveDots {
    twitch: (crate::model::live_dots::Names, Option<crate::model::live_dots::Names>),
    kick: (crate::model::live_dots::Names, Option<crate::model::live_dots::Names>),
  },
  FollowSnapshot(Vec<crate::model::following::Live>),
  FollowChange(crate::model::following::Change),
  // a room's blocked terms answered (by twitch room id)
  Blocked {
    room: Box<str>,
    res: Result<blocked::Got, blocked::Fail>,
  },
  // a chat mode change settled (twitch login)
  ModesDone {
    chan: Box<str>,
    set: crate::model::modes::Set,
    res: Result<(), modact::Fail>,
  },
  // a twitch channel's stream for /status (None = no answer)
  StreamInfo {
    chan: Box<str>,
    info: Option<crate::model::status::Info>,
  },
  // a twitch channel's prediction (Err = no sensible answer)
  Pred {
    chan: Box<str>,
    res: Result<Option<crate::model::pred::Pred>, ()>,
  },
  // a /warn, /shoutout or /shield settled (Ok(Some(on)) = shield mode now)
  Suite {
    chan: Box<str>,
    cmd: crate::model::modsuite::Cmd,
    res: Result<Option<bool>, blocked::Fail>,
  },
  // an /announce, /vip, /mod, /color, /poll or /prediction settled
  Chan {
    chan: Box<str>,
    cmd: crate::model::chantools::Cmd,
    res: Result<chantools::Done, chantools::Oops>,
  },
  // the mod tools screen's answer (by twitch login)
  Tools {
    chan: Box<str>,
    res: Result<modtools::Got, blocked::Fail>,
  },
  // a heatsync follow / unfollow settled
  Followed {
    platform: crate::model::message::Platform,
    login: Box<str>,
    want: bool,
    res: Result<(), social::FollowFail>,
  },
  WhisperSent {
    key: u64,
    res: Result<crate::model::whisper::Who, crate::model::whisper::Fail>,
  },
  // heatsync stored a dm of ours: the row `key` is that dm (digits), so it can be taken back
  DmStored {
    key: u64,
    id: Box<str>,
  },
  // our delete of a dm (digits) settled
  DmDeleted {
    id: Box<str>,
    res: Result<(), dmdel::Fail>,
  },
  // a dm was taken down by one of its two people, on any client (`dm:message-deleted`): its key in the list
  DmGone(Box<str>),
}

// event sink back to the ui. false = the ui is gone
#[derive(Clone)]
pub enum Out {
  Win(EventLoopProxy<NetEvent>),
  // the live probes read events off a plain channel: no event loop, no display
  #[cfg(test)]
  Pipe(std::sync::mpsc::Sender<NetEvent>),
}

impl Out {
  pub fn chat(&self, ch: Box<str>, ev: Chat) -> bool {
    self.send(NetEvent::Chat(ch, ev))
  }

  pub fn send(&self, ev: NetEvent) -> bool {
    match self {
      Out::Win(p) => p.send_event(ev).is_ok(),
      #[cfg(test)]
      Out::Pipe(p) => p.send(ev).is_ok(),
    }
  }

  pub fn status(&self, s: impl Into<String>) {
    self.send(NetEvent::Status(s.into()));
  }
}

// exponential, capped at 60s, ±20% jitter so a fleet of clients doesn't stampede
pub fn backoff(attempt: u32) -> Duration {
  let base = 2f64.powi(attempt.min(6) as i32).min(60.0);
  let jitter = 0.8 + 0.4 * (crate::util::time::now_ms() % 1000) as f64 / 1000.0;
  Duration::from_secs_f64(base * jitter)
}

// a wait in whole seconds for a status line ("trying again in 8s"), never "0s"
pub fn secs(d: Duration) -> u64 {
  d.as_secs_f64().round().max(1.0) as u64
}

// say a connection is down and when it's tried again: the first few times, then every fifth, so an
// outage is a few lines in the chat, not a wall of them
pub fn retrying(out: &Out, why: &str, attempt: u32, wait: Duration) {
  if attempt <= 3 || attempt.is_multiple_of(5) {
    out.status(format!("{why}, trying again in {}s", secs(wait)));
  }
}

// the line being answered: its platform, its message id, and @nick + its words for our own echo
pub struct Reply {
  // the platform the line came from: only its own leg threads the reply
  pub platform: Platform,
  pub id: Box<str>,
  pub nick: Box<str>,
  pub text: Box<str>,
}

pub enum Cmd {
  Join(Box<str>),
  Part(Box<str>),
  // roomstate told us the channel's twitch id: fetch its 7tv/bttv/ffz sets
  Room(Box<str>, Box<str>),
  Image(ImgId, Box<str>, u32),
  // heatsync relay rooms
  Hs(hs_ws::Cmd),
  // recent history for a relayed source: (platform, routing key, archive key)
  Backfill(Platform, Box<str>, Box<str>),
  // twitch user id → their heatsync inventory (version set after a ws invalidation)
  Sender(u64, Option<Box<str>>),
  // sign in with this platform
  Login(auth::Provider),
  Link(auth::Provider),
  // give up the sign-in that's waiting on the browser
  CancelLogin,
  // link youtube again, this time also asking to post chat as the user (the opt-in)
  AllowYoutubePosting,
  Logout,
  // a video or a stream to mpv (or the browser); mpv's failure comes back as a status line
  Play(Box<str>),
  // a link's preview, from heatsync's resolver
  Preview(Box<str>),
  // someone's pronouns through heatsync's proxy, by twitch user id
  Pronouns(u64),
  // automod holds: watch a room you moderate, allow / deny one
  Held(held::Cmd),
  // a #tag's hover card (signed in: whether you follow it)
  TagCard(Box<str>),
  // who you follow that is live (heatsync)
  Following,
  // your twitch follows, the live ones marked (heatsync): /import
  Import,
  // is each of these open twitch / kick channels live (the tab's red dot)
  LiveDots(Vec<Box<str>>, Vec<Box<str>>),
  // a room's blocked terms (by twitch room id): list, add, remove
  Blocked(Box<str>, blocked::Op),
  // set a twitch room's chat mode (by login)
  Modes(Box<str>, crate::model::modes::Set),
  // a twitch channel's stream, for /status
  StreamInfo(Box<str>),
  // a twitch channel's prediction, for the banner and /predictions
  Pred(Box<str>),
  // twitch's mod suite in a room (by login): warn, shoutout, shield mode
  Suite(Box<str>, crate::model::modsuite::Cmd),
  // announce, roles, color, polls and predictions (the room by login; empty for your color)
  Chan(Box<str>, crate::model::chantools::Cmd),
  // the mod tools screen: load, shield, an unban request, an automod level
  Tools(Box<str>, modtools::Op),
  Emote(emote_ops::EmoteCmd),
  // the inventory screen: list, rename, move, remove, undo, restore, sets, share, import
  Inventory(inventory::Op),
  // search the 7tv / ffz catalogs for tab-complete: the word, the ask's number, which page, from whom
  Catalog { q: Box<str>, epoch: u32, page: u32, stv: bool, ffz: bool },
  // the picker's: the same catalogs plus heatsync's directory ([7tv, ffz, hs] = which to ask)
  PickerSearch { q: Box<str>, epoch: u32, page: u32, ask: [bool; 3] },
  // upload this file / ask a file dialog for one, then upload it
  Upload(std::path::PathBuf),
  // tab-complete a path in the upload prompt, off the ui thread; `seq` tells the answer from an old one
  Complete { seq: u64, input: String, home: Option<String> },
  // a pasted picture: its own bytes (png, jpeg, webp or gif), checked like any upload
  Picture(Vec<u8>),
  // copy this to the clipboard and say `done` (empty = nothing); paste: read it, and say where for
  Copy { text: String, done: String },
  Paste { to_picker: bool },
  // which installed font a family name means (fc-match, off the ui thread)
  Font(Box<str>),
  Pick,
  // a chatter's picture
  Avatar(Platform, Box<str>),
  // a shared-chat room's channel name, by its number
  RoomLogin(Box<str>),
  // chatters' 7tv paints + badges
  Cosmetics(crate::cosmetics::id::PaintId),
  // a chatter's heat bar or sub flair
  Mark(marks::Ask),
  // our own twitch id, for watching our own 7tv cosmetics
  SevenTvMe(Option<u64>),
  // 7tv personal emotes: None = off, else the twitch room ids of the open channels. no id of ours goes
  SevenTvPersonal(Option<Vec<u64>>),
  // personal sets to read whole
  PersonalSets(Vec<Box<str>>),
  Send { platform: Platform, chan: Box<str>, text: String, reply: Option<Reply> },
  // read the account again (heatsync said it changed)
  Recheck,
  // whispers and feed calls bound to the login
  Social(social::Cmd),
  // a newer release: look, or install it
  Update(update::Cmd),
  // log in again asking for one more permission pack (twitch whispers, mod)
  Relink(&'static str),
  // a ban, timeout, delete… in a room we moderate
  Mod(modact::Req),
  // a clip of this twitch channel (the card's person, then the channel)
  Clip(Box<str>, Box<str>),
  // /nuke's deletes (paced), with the term for the summary
  Nuke(Vec<modact::Req>, String),
  // pin a line for everyone / clear the room's pin
  Pin(pinact::Op),
  // your mute list changed (synced while logged in)
  Mute(mutes::Op),
  // read the mute and block lists again (`user:blocked` / `user:unblocked`)
  Lists,
}

const IMAGE_FETCHES: usize = 6;
const DISK_CACHE_MAX: u64 = 256 << 20;
const MAX_IMAGE_BYTES: usize = 8 << 20;
const MAX_PREVIEW: usize = 64 << 10;
// the proxy's answer for one id is a few dozen bytes
const MAX_PRONOUNS: usize = 4 << 10;

struct DecodeJob {
  id: ImgId,
  url: Box<str>,
  h: u32,
  bytes: Vec<u8>,
  // fresh from the network: persist once it decodes
  store: bool,
}

// decode one job, and keep the disk cache honest: bytes are stored only once they decode, and a cached
// file that no longer decodes is dropped, so a retry fetches again instead of reading the same junk
fn run_job(j: &DecodeJob) -> Result<decode::Decoded, String> {
  // a decoder bug on one hostile file must not end decoding for the session
  let res =
    std::panic::catch_unwind(|| decode::decode(&j.bytes, j.h)).unwrap_or_else(|_| Err("decoder crashed".into()));
  match (&res, j.store) {
    (Ok(_), true) => cache::write(&j.url, &j.bytes),
    (Err(_), false) => cache::remove(&j.url),
    _ => {}
  }
  res
}

// decoding is cpu work; keep it off the network loop
fn spawn_decoder(out: Out) -> Option<std::sync::mpsc::Sender<DecodeJob>> {
  let (tx, rx) = std::sync::mpsc::channel::<DecodeJob>();
  let spawned = std::thread::Builder::new().name("decode".into()).spawn(move || {
    cache::prune(DISK_CACHE_MAX);
    for j in rx {
      let res = run_job(&j);
      if !out.send(NetEvent::Image(j.id, j.h, res)) {
        return;
      }
    }
  });
  spawned.ok().map(|_| tx)
}

// the decoder thread is gone: the picture fails (so it isn't "loading" forever) and, once, the app says so
static DECODER_GONE_SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn hand_to_decoder(dec: &std::sync::mpsc::Sender<DecodeJob>, job: DecodeJob, out: &Out) {
  let (id, h) = (job.id, job.h);
  if dec.send(job).is_err() {
    out.send(NetEvent::Image(id, h, Err("decoder stopped".into())));
    if !DECODER_GONE_SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
      out.status("pictures stopped loading: restart heatsync-chat");
    }
  }
}

async fn image(
  http: reqwest::Client,
  gate: Arc<Semaphore>,
  dec: std::sync::mpsc::Sender<DecodeJob>,
  id: ImgId,
  url: Box<str>,
  h: u32,
  out: Out,
) {
  if !url::allowed(&url) {
    out.send(NetEvent::Image(id, h, Err("host not allowed".into())));
    return;
  }
  // a disk read must not stall the one network thread
  let file = cache::path(&url);
  if let Ok(Some(bytes)) = tokio::task::spawn_blocking(move || file.and_then(|f| cache::read_file(&f))).await {
    return hand_to_decoder(&dec, DecodeJob { id, url, h, bytes, store: false }, &out);
  }
  let Ok(_permit) = gate.acquire().await else { return };
  match http::capped(http.get(&*url).send().await, MAX_IMAGE_BYTES).await {
    Ok(bytes) => hand_to_decoder(&dec, DecodeJob { id, url, h, bytes, store: true }, &out),
    Err(e) => {
      out.send(NetEvent::Image(id, h, Err(e)));
    }
  }
}

// ask heatsync what a link is: its resolver, or its archive for a chat-log permalink (ext
// fetch_embed_resolve / logEmbed: no login, 6s, a small answer)
async fn preview(http: reqwest::Client, url: Box<str>, out: Out) {
  let res = http.get(crate::model::preview::ask(&url)).timeout(Duration::from_secs(6)).send().await;
  let p = http::capped(res, MAX_PREVIEW)
    .await
    .ok()
    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    .and_then(|v| crate::model::preview::parse(&url, &v));
  out.send(NetEvent::Preview(url, p));
}

const MAX_TAG_CARD: usize = 4 * 1024;

// a #tag's hover card (ext tag-card.js): GET /api/tags/:tag/card, with the token when signed in (the
// answer then says whether you follow it). 404 = an unknown tag, an answer too
pub(crate) async fn tag_card(http: reqwest::Client, token: Option<String>, tag: Box<str>, out: Out) {
  let ok = (2..=30).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
  let res = if !ok {
    Ok(None)
  } else {
    let mut req =
      http.get(format!("{ORIGIN}/api/tags/{}/card", tag.to_ascii_lowercase())).timeout(Duration::from_secs(6));
    if let Some(t) = &token {
      req = req.bearer_auth(t);
    }
    match req.send().await {
      Ok(r) if r.status().as_u16() == 404 => Ok(None),
      r => http::capped(r, MAX_TAG_CARD)
        .await
        .map_err(drop)
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).map_err(drop))
        .map(|v| crate::model::tagcard::card(&v, crate::util::time::now_ms())),
    }
  };
  out.send(NetEvent::TagCard(tag, res));
}

// pronouns (ext fetch_pronouns) via heatsync's proxy: public, by twitch id, no token goes with it. 5s at most
async fn pronouns(http: reqwest::Client, uid: u64, out: Out) {
  let r = http
    .get(crate::model::pronouns::url(uid))
    .header("accept", "application/json")
    .timeout(Duration::from_secs(5))
    .send()
    .await;
  let res = http::capped(r, MAX_PRONOUNS)
    .await
    .ok()
    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    .ok_or(())
    .and_then(|v| crate::model::pronouns::parse(&v, uid));
  out.send(NetEvent::Pronouns(uid, res));
}

// image fetches follow redirects only between allowlisted hosts (`/api/img` answers with a 302)
fn image_client() -> Option<reqwest::Client> {
  use reqwest::redirect::Policy;
  reqwest::Client::builder()
    .user_agent(USER_AGENT)
    .connect_timeout(Duration::from_secs(8))
    .timeout(Duration::from_secs(12))
    .redirect(Policy::custom(|a| {
      if a.previous().len() >= 3 || !url::allowed(a.url().as_str()) { a.stop() } else { a.follow() }
    }))
    .build()
    .ok()
}

fn provider_name(p: Provider) -> &'static str {
  match p {
    Provider::SevenTv => "7tv",
    Provider::Bttv => "bttv",
    Provider::Ffz => "ffz",
    Provider::Twitch => "twitch",
  }
}

// a channel's emote sets, one retrying task per provider so a down one doesn't hold up the others.
// they stop when the tab closes
async fn channel_emotes(
  http: reqwest::Client,
  ch: Box<str>,
  room: Box<str>,
  stv: mpsc::UnboundedSender<seventv::Cmd>,
  open: retry::Open,
  out: Out,
) {
  if !room.bytes().all(|b| b.is_ascii_digit()) {
    return;
  }
  for (p, u) in providers::channel_urls(&room) {
    let Some(tab) = open.watch(&ch) else { return };
    let (http, ch, room, stv, out) = (http.clone(), ch.clone(), room.clone(), stv.clone(), out.clone());
    tokio::spawn(async move {
      let what = format!("{ch}'s {} emotes", provider_name(p));
      let Some(v) = retry::until(&out, &what, || tab.alive(), || providers::fetch(&http, &u)).await else { return };
      if p == Provider::SevenTv
        && let Some(set) = providers::seventv_set_id(&v)
      {
        // live edits for this channel's set; the broadcaster's own cosmetics too
        let user = providers::seventv_user_id(&v).zip(room.parse().ok());
        let _ = stv.send(seventv::Cmd::Watch { ch: ch.clone(), set, user });
      }
      if p == Provider::Ffz {
        out.send(NetEvent::FfzRoomBadges(ch.clone(), badges::fetch::ffz_room(&v)));
      }
      out.send(NetEvent::Emotes(Some(ch), p, providers::parse_channel(p, &v)));
    });
  }
}

// the global badge sources, each tried until it answers
async fn global_badges(http: reqwest::Client, out: Out) {
  let (h, o) = (http.clone(), out.clone());
  tokio::spawn(async move {
    if let Some(s) = retry::until(&o, "twitch badges", || true, || badges::fetch::twitch_global(&h)).await {
      o.send(NetEvent::TwitchBadges(None, s));
    }
  });
  type Parse = fn(&serde_json::Value) -> UserBadges;
  let third: [(Source, &str, &'static str, Parse); 3] = [
    (Source::Bttv, badges::fetch::BTTV, "bttv badges", badges::fetch::bttv),
    (Source::Chatterino, badges::fetch::CHATTERINO, "chatterino badges", badges::fetch::chatterino),
    (Source::Ffz, badges::fetch::FFZ_IDS, "ffz badges", badges::fetch::ffz),
  ];
  for (src, u, what, parse) in third {
    let (http, out) = (http.clone(), out.clone());
    tokio::spawn(async move {
      if let Some(v) = retry::until(&out, what, || true, || providers::fetch(&http, u)).await {
        out.send(NetEvent::UserBadges(src, parse(&v)));
      }
    });
  }
}

// a channel's own twitch badges, until they arrive or the tab closes
async fn channel_badges(http: reqwest::Client, ch: Box<str>, tab: retry::Watch, out: Out) {
  let what = format!("{ch}'s badges");
  if let Some(s) = retry::until(&out, &what, || tab.alive(), || badges::fetch::twitch_channel(&http, &ch)).await {
    out.send(NetEvent::TwitchBadges(Some(ch), s));
  }
}

// a kick channel's subscriber art, a few tries (a channel with no art of its own answers fine and
// sends nothing; lines then wear the "sub" word, like the ext)
async fn kick_badges(http: reqwest::Client, slug: Box<str>, out: Out) {
  let what = format!("{slug}'s kick badges");
  if let Some(art) = retry::until_n(&out, &what, 3, || true, || badges::fetch::kick_channel(&http, &slug)).await
    && !art.is_empty()
  {
    out.send(NetEvent::KickBadges(slug, art));
  }
}

// every global emote source, each its own retrying task: one down doesn't hold up the rest
async fn global_emotes(http: reqwest::Client, out: Out) {
  type Parse = fn(&serde_json::Value) -> Set;
  // twitch's: the ext's list (every helix global: LUL, :P) laid over the catalog's twitch rows (the
  // event unlocks, which that list lacks). either alone is short; one failing still leaves the other,
  // so each landing sends what the two have so far
  let twitch: Arc<std::sync::Mutex<[Set; 2]>> = Arc::default();
  for (i, u) in [providers::TWITCH_GLOBAL, providers::HEATSYNC_GLOBAL].into_iter().enumerate() {
    let (http, out, twitch) = (http.clone(), out.clone(), twitch.clone());
    tokio::spawn(async move {
      let Some(v) = retry::until(&out, "twitch emotes", || true, || providers::fetch(&http, u)).await else { return };
      let mut sets = twitch.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
      sets[i] = providers::twitch_global(&v);
      let mut all = sets[0].clone();
      providers::merge(&mut all, sets[1].clone());
      out.send(NetEvent::Emotes(None, Provider::Twitch, all));
    });
  }
  let sets: [(Provider, &str, Parse); 3] = [
    (Provider::SevenTv, providers::SEVENTV_GLOBAL, providers::seventv),
    (Provider::Bttv, providers::BTTV_GLOBAL, providers::bttv_global),
    (Provider::Ffz, providers::FFZ_GLOBAL, providers::ffz_global),
  ];
  for (p, u, parse) in sets {
    let (http, out) = (http.clone(), out.clone());
    tokio::spawn(async move {
      let what = format!("{} emotes", provider_name(p));
      if let Some(v) = retry::until(&out, &what, || true, || providers::fetch(&http, u)).await {
        out.send(NetEvent::Emotes(None, p, parse(&v)));
      }
    });
  }
}

// a long-lived part of the network side; if it crashes, say so instead of going quiet
fn watch(name: &'static str, task: impl std::future::Future<Output = ()> + Send + 'static, out: Out) {
  use futures_util::FutureExt;
  tokio::spawn(async move {
    if std::panic::AssertUnwindSafe(task).catch_unwind().await.is_err() {
      out.status(format!(
        "{name} stopped after a crash: restart heatsync-chat (details in {})",
        crate::util::crash::LOG
      ));
    }
  });
}

pub struct Net {
  tx: mpsc::UnboundedSender<Cmd>,
}

impl Net {
  pub fn spawn(proxy: EventLoopProxy<NetEvent>) -> Net {
    Self::spawn_out(Out::Win(proxy))
  }

  pub(crate) fn spawn_out(out: Out) -> Net {
    let (tx, rx) = mpsc::unbounded_channel();
    let spawned = std::thread::Builder::new().name("net".into()).spawn(move || {
      let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
          eprintln!("network runtime: {e}");
          return out.status("can't start the network, restart heatsync-chat");
        }
      };
      let o = out.clone();
      if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.block_on(main(rx, o)))).is_err() {
        out.status(format!(
          "network stopped after a crash: restart heatsync-chat (details in {})",
          crate::util::crash::LOG
        ));
      }
    });
    if let Err(e) = spawned {
      eprintln!("net thread: {e}");
    }
    Net { tx }
  }

  pub fn send(&self, c: Cmd) {
    let _ = self.tx.send(c);
  }

  // false once the network part has stopped (a crash): nothing sent now would ever go anywhere
  pub fn alive(&self) -> bool {
    !self.tx.is_closed()
  }

  // a net that goes nowhere: what the app asks for lands in the returned receiver (offline probes)
  #[cfg(test)]
  pub(crate) fn tap() -> (Net, mpsc::UnboundedReceiver<Cmd>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (Net { tx }, rx)
  }
}

const MAX_REDIRECTS: usize = 5;

// a redirect is followed only over https and only so far: a hostile reply can't bounce us around or
// down to plain http (`https_only` on the client refuses http outright)
fn follow_redirect(hops: usize, to: &reqwest::Url) -> bool {
  hops < MAX_REDIRECTS && to.scheme() == "https"
}

fn redirects() -> reqwest::redirect::Policy {
  reqwest::redirect::Policy::custom(
    |a| if follow_redirect(a.previous().len(), a.url()) { a.follow() } else { a.stop() },
  )
}

// the one client for every api and json fetch
fn main_client() -> Result<reqwest::Client, reqwest::Error> {
  reqwest::Client::builder()
    .user_agent(USER_AGENT)
    .connect_timeout(Duration::from_secs(8))
    .timeout(Duration::from_secs(12))
    .https_only(true)
    .redirect(redirects())
    .build()
}

async fn main(mut rx: mpsc::UnboundedReceiver<Cmd>, out: Out) {
  let http = match main_client() {
    Ok(c) => c,
    Err(e) => {
      eprintln!("http client: {e}");
      return out.status("can't start the network, restart heatsync-chat");
    }
  };
  let open = retry::Open::default();
  let (sess_tx, sess_rx) = mpsc::unbounded_channel();
  let (irc_tx, irc_rx) = mpsc::unbounded_channel();
  watch("chat", irc::run(irc_rx, sess_tx.clone(), http.clone(), out.clone()), out.clone());
  watch("emotes", global_emotes(http.clone(), out.clone()), out.clone());
  watch("badges", global_badges(http.clone(), out.clone()), out.clone());
  let (hs_tx, hs_rx) = mpsc::unbounded_channel();
  let (pin_tx, pin_rx) = mpsc::unbounded_channel();
  watch("pins", pin::run(http.clone(), pin_rx, out.clone()), out.clone());
  watch("heatsync", hs_ws::run(hs_rx, pin_tx, sess_tx.clone(), out.clone()), out.clone());
  let hs_auth = hs_tx.clone();
  let (snd_tx, snd_rx) = mpsc::unbounded_channel();
  watch("sending", senders::run(http.clone(), snd_rx, out.clone()), out.clone());
  watch("login", session::run(http.clone(), sess_rx, irc_tx.clone(), hs_auth, out.clone()), out.clone());
  let (cos_tx, cos_rx) = mpsc::unbounded_channel();
  watch("paints", cosmetics::run(http.clone(), cos_rx, out.clone()), out.clone());
  let (mk_tx, mk_rx) = mpsc::unbounded_channel();
  watch("marks", marks::run(http.clone(), mk_rx, out.clone()), out.clone());
  let (av_tx, av_rx) = mpsc::unbounded_channel();
  watch("avatars", avatars::run(http.clone(), av_rx, out.clone()), out.clone());
  let (rl_tx, rl_rx) = mpsc::unbounded_channel();
  watch("rooms", roomname::run(http.clone(), rl_rx, out.clone()), out.clone());
  let (stv_tx, stv_rx) = mpsc::unbounded_channel();
  watch("7tv", seventv::run(http.clone(), stv_rx, stv_tx.clone(), out.clone()), out.clone());
  let Some(dec) = spawn_decoder(out.clone()) else {
    return out.status("can't start the picture decoder, restart heatsync-chat");
  };
  let Some(img_http) = image_client() else {
    return out.status("can't start the picture loader, restart heatsync-chat");
  };
  let gate = Arc::new(Semaphore::new(IMAGE_FETCHES));
  let reads = Arc::new(Semaphore::new(personal::IN_FLIGHT));
  let (irc_send, sess_send) = (irc_tx.clone(), sess_tx.clone());
  while let Some(cmd) = rx.recv().await {
    match cmd {
      Cmd::Join(ch) => {
        let tab = open.join(&ch);
        watch("chat history", recent::fetch(http.clone(), ch.clone(), tab.clone(), out.clone()), out.clone());
        if ident::twitch_room(&ch) {
          watch("badges", channel_badges(http.clone(), ch.clone(), tab.clone(), out.clone()), out.clone());
          watch("linked channels", links::discover(http.clone(), ch.clone(), tab, out.clone()), out.clone());
          let _ = hs_tx.send(hs_ws::Cmd::Join(Platform::Twitch, ch.clone()));
        }
        let _ = irc_tx.send(irc::Cmd::Join(ch));
      }
      Cmd::Part(ch) => {
        open.part(&ch);
        let _ = stv_tx.send(seventv::Cmd::Unwatch(ch.clone()));
        let _ = hs_tx.send(hs_ws::Cmd::Leave(Platform::Twitch, ch.clone()));
        let _ = irc_tx.send(irc::Cmd::Part(ch));
      }
      Cmd::Hs(c) => {
        if let hs_ws::Cmd::Join(Platform::Kick, slug) = &c
          && ident::relay_key(slug)
        {
          watch("kick badges", kick_badges(http.clone(), slug.clone(), out.clone()), out.clone());
        }
        let _ = hs_tx.send(c);
      }
      Cmd::Backfill(p, key, archive) => {
        watch("history", links::backfill(http.clone(), p, key, archive, out.clone()), out.clone());
      }
      Cmd::Login(p) => {
        let _ = sess_tx.send(session::Cmd::Login(p, "base"));
      }
      Cmd::Link(p) => {
        let _ = sess_tx.send(session::Cmd::Link(p, "base"));
      }
      Cmd::CancelLogin => {
        let _ = sess_tx.send(session::Cmd::CancelLogin);
      }
      Cmd::AllowYoutubePosting => {
        let _ = sess_tx.send(session::Cmd::Link(auth::Provider::Youtube, "post"));
      }
      Cmd::Lists => {
        let _ = sess_tx.send(session::Cmd::Lists);
      }
      Cmd::Mute(op) => {
        let _ = sess_tx.send(session::Cmd::Mute(op));
      }
      Cmd::Mod(r) => {
        let _ = sess_tx.send(session::Cmd::Mod(r));
      }
      Cmd::Clip(login, chan) => {
        let _ = sess_tx.send(session::Cmd::Clip(login, chan));
      }
      Cmd::Nuke(reqs, term) => {
        let _ = sess_tx.send(session::Cmd::Nuke(reqs, term));
      }
      Cmd::Pin(op) => {
        let _ = sess_tx.send(session::Cmd::Pin(op));
      }
      Cmd::Held(c) => {
        let _ = sess_tx.send(session::Cmd::Held(c));
      }
      Cmd::TagCard(tag) => {
        let _ = sess_tx.send(session::Cmd::TagCard(tag));
      }
      Cmd::Following => {
        let _ = sess_tx.send(session::Cmd::Following);
      }
      Cmd::Import => {
        let _ = sess_tx.send(session::Cmd::Import);
      }
      Cmd::LiveDots(twitch, kick) => {
        watch("live dots", live_dots::ask(http.clone(), twitch, kick, out.clone()), out.clone());
      }
      Cmd::Blocked(room, op) => {
        let _ = sess_tx.send(session::Cmd::Blocked(room, op));
      }
      Cmd::Modes(chan, set) => {
        let _ = sess_tx.send(session::Cmd::Modes(chan, set));
      }
      Cmd::StreamInfo(chan) => {
        let _ = sess_tx.send(session::Cmd::StreamInfo(chan));
      }
      Cmd::Pred(chan) => {
        let _ = sess_tx.send(session::Cmd::Pred(chan));
      }
      Cmd::Suite(chan, c) => {
        let _ = sess_tx.send(session::Cmd::Suite(chan, c));
      }
      Cmd::Chan(chan, c) => {
        let _ = sess_tx.send(session::Cmd::Chan(chan, c));
      }
      Cmd::Tools(chan, op) => {
        let _ = sess_tx.send(session::Cmd::Tools(chan, op));
      }
      Cmd::Relink(pack) => {
        let _ = sess_tx.send(session::Cmd::Login(auth::Provider::Twitch, pack));
      }
      Cmd::Social(c) => {
        let _ = sess_tx.send(session::Cmd::Social(c));
      }
      Cmd::Update(c) => {
        watch("updater", update::run(http.clone(), c, out.clone()), out.clone());
      }
      Cmd::Pronouns(uid) => {
        watch("pronouns", pronouns(http.clone(), uid, out.clone()), out.clone());
      }
      Cmd::Preview(url) => {
        watch("previews", preview(http.clone(), url, out.clone()), out.clone());
      }
      Cmd::Play(url) => {
        let o = out.clone();
        if let Err(e) = crate::util::open::play(&url, move |s| o.status(s)) {
          out.status(e);
        }
      }
      Cmd::Recheck => {
        let _ = sess_tx.send(session::Cmd::Recheck);
      }
      Cmd::Logout => {
        let _ = sess_tx.send(session::Cmd::Logout);
      }
      Cmd::Inventory(op) => {
        let _ = sess_tx.send(session::Cmd::Inventory(op));
      }
      Cmd::Emote(c) => {
        let _ = sess_tx.send(session::Cmd::Emote(c));
      }
      Cmd::Catalog { q, epoch, page, stv, ffz } => {
        watch("emote search", catalog::run(http.clone(), q, epoch, page, (stv, ffz), out.clone()), out.clone());
      }
      Cmd::PickerSearch { q, epoch, page, ask } => {
        watch("emote search", catalog::search(http.clone(), q, epoch, page, ask, out.clone()), out.clone());
      }
      Cmd::Upload(p) => {
        let _ = sess_tx.send(session::Cmd::Upload(p));
      }
      Cmd::Complete { seq, input, home } => upload::complete(seq, input, home, out.clone()),
      Cmd::Picture(bytes) => {
        let _ = sess_tx.send(session::Cmd::Picture(bytes));
      }
      Cmd::Copy { text, done } => clip::copy(text, done, out.clone()),
      Cmd::Paste { to_picker } => clip::paste(to_picker, out.clone()),
      Cmd::Font(name) => font::find(name, out.clone()),
      Cmd::Pick => {
        let _ = sess_tx.send(session::Cmd::Pick);
      }
      // a line never waits on an emote add (emote rule 7): the room draws a pick from its vouch
      c @ Cmd::Send { .. } => send_line(c, &irc_send, &sess_send),
      Cmd::Sender(id, ver) => {
        let _ = snd_tx.send((id, ver));
      }
      Cmd::Avatar(p, login) => {
        let _ = av_tx.send((p, login));
      }
      Cmd::RoomLogin(id) => {
        let _ = rl_tx.send(id);
      }
      Cmd::Cosmetics(id) => {
        let _ = cos_tx.send(id);
      }
      Cmd::Mark(a) => {
        let _ = mk_tx.send(a);
      }
      Cmd::SevenTvMe(id) => {
        let _ = stv_tx.send(seventv::Cmd::Me(id));
      }
      Cmd::SevenTvPersonal(rooms) => {
        let _ = stv_tx.send(seventv::Cmd::Personal(rooms));
      }
      Cmd::PersonalSets(sets) => {
        watch("personal emotes", personal::read(http.clone(), reads.clone(), sets, out.clone()), out.clone());
      }
      Cmd::Room(ch, room) => {
        watch(
          "channel emotes",
          channel_emotes(http.clone(), ch, room, stv_tx.clone(), open.clone(), out.clone()),
          out.clone(),
        );
      }
      Cmd::Image(id, u, h) => {
        tokio::spawn(image(img_http.clone(), gate.clone(), dec.clone(), id, u, h, out.clone()));
      }
    }
  }
}

// a chat line
fn send_line(c: Cmd, irc_tx: &mpsc::UnboundedSender<irc::Cmd>, sess_tx: &mpsc::UnboundedSender<session::Cmd>) {
  match c {
    Cmd::Send { platform: Platform::Twitch, chan, text, reply } => {
      let _ = irc_tx.send(irc::Cmd::Send { chan, text, reply });
    }
    Cmd::Send { platform: Platform::Kick, chan, text, reply } => {
      let _ = sess_tx.send(session::Cmd::Kick { chan, text, reply: reply.map(|r| r.id) });
    }
    Cmd::Send { platform: Platform::Youtube, chan, text, .. } => {
      let _ = sess_tx.send(session::Cmd::Youtube { chan, text });
    }
    _ => {}
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // one-shot local server answering whatever it is asked with `reply`
  async fn serve(reply: String) -> std::net::SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
      let (mut s, _) = l.accept().await.unwrap();
      let mut buf = [0u8; 1024];
      let _ = s.read(&mut buf).await;
      let _ = s.write_all(reply.as_bytes()).await;
    });
    addr
  }

  // a one-shot job that panics says so instead of dying silently
  #[tokio::test]
  async fn a_panicking_job_is_reported() {
    let (tx, rx) = std::sync::mpsc::channel();
    watch("chat history", async { panic!("boom") }, Out::Pipe(tx));
    for _ in 0..50 {
      tokio::time::sleep(Duration::from_millis(10)).await;
      if let Ok(NetEvent::Status(s)) = rx.try_recv() {
        assert!(s.starts_with("chat history stopped after a crash"), "{s}");
        return;
      }
    }
    panic!("no word of the crash");
  }

  #[test]
  fn an_outage_is_a_few_plain_lines_that_say_what_happens_next() {
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    for attempt in 1..=10 {
      retrying(&out, "can't reach twitch", attempt, Duration::from_millis(7600));
    }
    let said: Vec<String> =
      rx.try_iter().filter_map(|e| if let NetEvent::Status(s) = e { Some(s) } else { None }).collect();
    assert_eq!(said.len(), 5, "attempts 1, 2, 3, 5 and 10: {said:?}");
    assert_eq!(said[0], "can't reach twitch, trying again in 8s");
    assert_eq!(secs(Duration::from_millis(100)), 1, "never 0s");
    for s in &said {
      assert_eq!(*s, s.to_lowercase());
    }
  }

  #[test]
  fn redirects_stop_at_five_hops_and_never_leave_https() {
    let https = reqwest::Url::parse("https://heatsync.org/x").unwrap();
    let http = reqwest::Url::parse("http://heatsync.org/x").unwrap();
    for hops in 0..MAX_REDIRECTS {
      assert!(follow_redirect(hops, &https), "hop {hops}");
    }
    assert!(!follow_redirect(MAX_REDIRECTS, &https), "the sixth hop is a loop or a trap");
    assert!(!follow_redirect(0, &http), "never down to plain http");
    assert!(!follow_redirect(0, &reqwest::Url::parse("ftp://x/y").unwrap()));
  }

  #[tokio::test]
  async fn the_main_client_refuses_plain_http_and_does_not_chase_a_redirect_to_it() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let c = main_client().unwrap();
    let e = c.get("http://127.0.0.1:9/").send().await.unwrap_err();
    assert!(e.is_builder(), "plain http is refused before anything is sent: {e}");
    // the same redirect policy on a client that can reach a local server: a 302 to http stays a 302
    let target = serve("HTTP/1.1 200 OK\r\ncontent-length: 8\r\nconnection: close\r\n\r\nfollowed".into()).await;
    let hop = serve(format!(
      "HTTP/1.1 302 Found\r\nlocation: http://{target}/\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    ))
    .await;
    let plain = reqwest::Client::builder().redirect(redirects()).build().unwrap();
    let r = plain.get(format!("http://{hop}/")).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 302, "not followed");
    // the default policy would have walked right on
    let target = serve("HTTP/1.1 200 OK\r\ncontent-length: 8\r\nconnection: close\r\n\r\nfollowed".into()).await;
    let hop = serve(format!(
      "HTTP/1.1 302 Found\r\nlocation: http://{target}/\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    ))
    .await;
    let r = reqwest::Client::new().get(format!("http://{hop}/")).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
  }

  #[test]
  fn a_crashed_task_says_so() {
    let (tx, rx) = std::sync::mpsc::channel();
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
      watch("chat", async { panic!("boom") }, Out::Pipe(tx.clone()));
      watch("pins", async {}, Out::Pipe(tx));
      tokio::task::yield_now().await;
      tokio::task::yield_now().await;
    });
    let said: Vec<String> =
      rx.try_iter().filter_map(|e| if let NetEvent::Status(s) = e { Some(s) } else { None }).collect();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("chat stopped after a crash"), "{}", said[0]);
  }
}

#[cfg(test)]
mod image_io {
  use super::*;

  const URL: &str = "https://cdn.7tv.app/emote/t1/1x.webp";

  fn job(store: bool, bytes: &[u8]) -> DecodeJob {
    DecodeJob { id: 0, url: URL.into(), h: 28, bytes: bytes.to_vec(), store }
  }

  fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
  }

  fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
  }

  #[test]
  fn a_reply_that_is_not_a_picture_is_never_cached() {
    cache::remove(URL);
    assert!(run_job(&job(true, b"<html>502 bad gateway</html>")).is_err());
    assert!(cache::read(URL).is_none(), "junk must not be kept for the next retry");
  }

  #[test]
  fn a_cached_file_that_no_longer_decodes_is_dropped() {
    cache::write(URL, b"<html>oops</html>");
    assert!(cache::read(URL).is_some());
    assert!(run_job(&job(false, b"<html>oops</html>")).is_err());
    assert!(cache::read(URL).is_none());
  }

  #[test]
  fn a_good_picture_is_cached_once_it_decodes() {
    cache::remove(URL);
    let mut png = Vec::new();
    {
      let mut e = png::Encoder::new(&mut png, 2, 2);
      e.set_color(png::ColorType::Rgba);
      e.set_depth(png::BitDepth::Eight);
      e.write_header().unwrap().write_image_data(&[9u8; 16]).unwrap();
    }
    assert!(run_job(&job(true, &png)).is_ok());
    assert_eq!(cache::read(URL).as_deref(), Some(&png[..]));
    // and a good cached file stays
    assert!(run_job(&job(false, &png)).is_ok());
    assert!(cache::read(URL).is_some());
  }

  #[test]
  fn a_dead_decoder_fails_the_picture_and_says_so_once() {
    cache::write(URL, b"anything");
    let (dec, rx) = std::sync::mpsc::channel::<DecodeJob>();
    drop(rx);
    let (tx, ev) = std::sync::mpsc::channel();
    let out = Out::Pipe(tx);
    DECODER_GONE_SAID.store(false, std::sync::atomic::Ordering::Relaxed);
    for id in [3, 4] {
      run(image(client(), Arc::new(Semaphore::new(1)), dec.clone(), id, URL.into(), 28, out.clone()));
    }
    let evs: Vec<NetEvent> = ev.try_iter().collect();
    let failed = evs.iter().filter(|e| matches!(e, NetEvent::Image(_, _, Err(_)))).count();
    let said: Vec<&String> =
      evs.iter().filter_map(|e| if let NetEvent::Status(s) = e { Some(s) } else { None }).collect();
    assert_eq!(failed, 2, "each picture fails instead of loading forever");
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(*said[0], said[0].to_lowercase());
  }

  // a cache file that blocks on read (a fifo) may hold up only its own picture, not the net thread
  #[cfg(unix)]
  #[test]
  fn the_disk_read_does_not_stall_the_network_thread() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let path = cache::path(URL).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(std::process::Command::new("mkfifo").arg(&path).status().unwrap().success());
    let p = path.clone();
    let releaser = std::thread::spawn(move || {
      std::thread::sleep(Duration::from_millis(600));
      std::fs::write(p, b"anything").unwrap();
    });
    let (dec, _rx) = std::sync::mpsc::channel::<DecodeJob>();
    let (tx, _ev) = std::sync::mpsc::channel();
    let image_done = Arc::new(AtomicBool::new(false));
    let ticker_saw_done = run({
      let image_done = image_done.clone();
      async move {
        let seen = image_done.clone();
        let ticker = tokio::spawn(async move {
          tokio::time::sleep(Duration::from_millis(50)).await;
          seen.load(Ordering::SeqCst)
        });
        image(client(), Arc::new(Semaphore::new(1)), dec, 1, URL.into(), 28, Out::Pipe(tx)).await;
        image_done.store(true, Ordering::SeqCst);
        ticker.await.unwrap()
      }
    });
    releaser.join().unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(!ticker_saw_done, "the timer fired while the file read was still waiting");
  }
}

#[cfg(test)]
mod live {
  use super::*;
  use crate::cosmetics::id::PaintId;

  // real network, no window and no event loop: join nl_kripp, wait for ennortix's backfilled line, ask for
  // their cosmetics the way the ui does, expect the heatsync paint back
  #[test]
  #[ignore]
  fn live_ennortix_paint_arrives_through_net() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (tx, rx) = std::sync::mpsc::channel();
    let net = Net::spawn_out(Out::Pipe(tx));
    net.send(Cmd::Join("nl_kripp".into()));
    let (deadline, id) = (std::time::Instant::now() + Duration::from_secs(40), PaintId::Twitch(37005319));
    let (mut asked, mut painted, mut log) = (false, None, Vec::new());
    while painted.is_none() && std::time::Instant::now() < deadline {
      match rx.recv_timeout(Duration::from_millis(250)) {
        Ok(NetEvent::Backfill(_, evs)) if !asked => {
          let hit = evs.iter().any(|e| matches!(e, Chat::Msg(m) if PaintId::of_message(m).as_ref() == Some(&id)));
          log.push(format!("backfill {} lines, ennortix among them: {hit}", evs.len()));
          if hit {
            asked = true;
            net.send(Cmd::Cosmetics(id.clone()));
          }
        }
        Ok(NetEvent::Cosmetics(got)) => {
          log.push(format!("cosmetics {:?}", got.iter().map(|(i, c)| (i.wire(), c.is_some())).collect::<Vec<_>>()));
          painted = Some(got.iter().any(|(i, c)| *i == id && c.as_ref().is_some_and(|c| c.paint.is_some())));
        }
        Ok(NetEvent::Status(s)) => log.push(format!("status {s}")),
        _ => {}
      }
    }
    eprintln!("{}", log.join("\n"));
    assert_eq!(painted, Some(true));
  }
}
