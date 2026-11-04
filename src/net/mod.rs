// all network io lives on one background thread running a current-thread tokio runtime.
// the ui talks to it through `Net` and hears back through winit user events
pub mod auth;
mod avatars;
pub mod blocked;
mod catalog;
pub mod chantools;
mod clip;
mod cosmetics;
pub mod dupe;
pub mod emote_ops;
pub mod held;
pub mod hs_ws;
pub mod http;
pub mod irc;
mod links;
pub mod modact;
pub mod modes;
pub mod modsuite;
pub mod modtools;
pub mod mutes;
pub mod pin;
pub mod pred;
mod recent;
pub mod relay;
mod retry;
mod send_queue;
mod senders;
mod session;
pub mod seventv;
pub mod social;
pub mod update;
pub mod upload;
mod youtube;

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
  Upload(upload::Upload),
  // what the clipboard held when asked (the key's target: the picker's search box or the line), or why
  // it couldn't say
  Pasted {
    to_picker: bool,
    got: Result<crate::util::clip::Paste, String>,
  },
  // a chatter's picture url (None = none found)
  Avatar(Platform, Box<str>, Option<Box<str>>),
  // 7tv paints + badges for twitch ids (None = wears nothing)
  Cosmetics(Vec<(crate::cosmetics::id::PaintId, Option<crate::cosmetics::Cosmetic>)>),
  // 7tv says this twitch user's cosmetics changed: ask again
  CosmeticsStale(crate::cosmetics::id::PaintId),
  // heatsync says these twitch ids changed their paint: ask again
  CosmeticsChanged(Vec<crate::cosmetics::id::PaintId>),
  // a live edit of a channel's 7tv emote set
  EmoteDelta(Box<str>, crate::emotes::live::Delta),
  // a channel's pinned line was set or cleared (None)
  Pin {
    platform: Platform,
    key: Box<str>,
    pin: Option<crate::model::pin::Pin>,
  },
  // went live / offline / switched game, already paired and de-duplicated
  Stream(crate::model::stream_events::Line),
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
  // followed streamers' twitch colors (lowercase login, rgb)
  FollowColors(Vec<(Box<str>, u32)>),
  // your mute / block lists on heatsync, read after a login
  Lists(mutes::Lists),
  // a mute changed on another device
  MuteSync(mutes::Sync),
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
  // someone's pronouns from pronoundb, by twitch user id
  Pronouns(u64),
  // automod holds: watch a room you moderate, allow / deny one
  Held(held::Cmd),
  // a #tag's hover card (signed in: whether you follow it)
  TagCard(Box<str>),
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
  // search the 7tv / ffz catalogs for tab-complete: the word, the ask's number, which page, from whom
  Catalog { q: Box<str>, epoch: u32, page: u32, stv: bool, ffz: bool },
  // upload this file / ask a file dialog for one, then upload it
  Upload(std::path::PathBuf),
  // a pasted picture: its own bytes (png, jpeg, webp or gif), checked like any upload
  Picture(Vec<u8>),
  // copy this to the clipboard and say `done` (empty = nothing); paste: read it, and say where for
  Copy { text: String, done: String },
  Paste { to_picker: bool },
  Pick,
  // a chatter's picture
  Avatar(Platform, Box<str>),
  // chatters' 7tv paints + badges
  Cosmetics(crate::cosmetics::id::PaintId),
  // our own twitch id, for watching our own 7tv cosmetics
  SevenTvMe(Option<u64>),
  Send { platform: Platform, chan: Box<str>, text: String, reply: Option<Reply> },
  // read the account again (heatsync said it changed)
  Recheck,
  // the next line sent waits (bounded) for the emote adds/swaps asked for just before it
  HoldSends,
  // whispers and feed calls bound to the login
  Social(social::Cmd),
  // a newer release: look, or install it
  Update(update::Cmd),
  // log in again asking for one more permission pack (twitch whispers, mod)
  Relink(&'static str),
  // a ban, timeout, delete… in a room we moderate
  Mod(modact::Req),
  // your mute list changed (synced while logged in)
  Mute(mutes::Op),
}

const IMAGE_FETCHES: usize = 6;
const DISK_CACHE_MAX: u64 = 256 << 20;
const MAX_IMAGE_BYTES: usize = 8 << 20;
const MAX_PREVIEW: usize = 64 << 10;
// a pronoundb answer for one id is a few dozen bytes
const MAX_PRONOUNS: usize = 4 << 10;

struct DecodeJob {
  id: ImgId,
  url: Box<str>,
  h: u32,
  bytes: Vec<u8>,
  // fresh from the network: persist before decoding
  store: bool,
}

// decoding is cpu work; keep it off the network loop
fn spawn_decoder(out: Out) -> Option<std::sync::mpsc::Sender<DecodeJob>> {
  let (tx, rx) = std::sync::mpsc::channel::<DecodeJob>();
  let spawned = std::thread::Builder::new().name("decode".into()).spawn(move || {
    cache::prune(DISK_CACHE_MAX);
    for j in rx {
      if j.store {
        cache::write(&j.url, &j.bytes);
      }
      // a decoder bug on one hostile file must not end decoding for the session
      let res =
        std::panic::catch_unwind(|| decode::decode(&j.bytes, j.h)).unwrap_or_else(|_| Err("decoder crashed".into()));
      if !out.send(NetEvent::Image(j.id, j.h, res)) {
        return;
      }
    }
  });
  spawned.ok().map(|_| tx)
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
  let fail = |e: String| {
    out.send(NetEvent::Image(id, h, Err(e)));
  };
  if !url::allowed(&url) {
    return fail("host not allowed".into());
  }
  if let Some(bytes) = cache::read(&url) {
    let _ = dec.send(DecodeJob { id, url, h, bytes, store: false });
    return;
  }
  let Ok(_permit) = gate.acquire().await else { return };
  match http::capped(http.get(&*url).send().await, MAX_IMAGE_BYTES).await {
    Ok(bytes) => {
      let _ = dec.send(DecodeJob { id, url, h, bytes, store: true });
    }
    Err(e) => fail(e),
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

// pronoundb (ext fetch_pronouns): public, by twitch id, no token ever goes there. 5s at most
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
    .map(|v| crate::model::pronouns::parse(&v, uid))
    .ok_or(());
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
  tokio::spawn(global_emotes(http.clone(), out.clone()));
  tokio::spawn(global_badges(http.clone(), out.clone()));
  let (hs_tx, hs_rx) = mpsc::unbounded_channel();
  let (pin_tx, pin_rx) = mpsc::unbounded_channel();
  watch("pins", pin::run(http.clone(), pin_rx, out.clone()), out.clone());
  watch("heatsync", hs_ws::run(hs_rx, pin_tx, out.clone()), out.clone());
  let hs_auth = hs_tx.clone();
  let (snd_tx, snd_rx) = mpsc::unbounded_channel();
  watch("sending", senders::run(http.clone(), snd_rx, out.clone()), out.clone());
  watch("login", session::run(http.clone(), sess_rx, irc_tx.clone(), hs_auth, out.clone()), out.clone());
  let (cos_tx, cos_rx) = mpsc::unbounded_channel();
  watch("paints", cosmetics::run(http.clone(), cos_rx, out.clone()), out.clone());
  let (av_tx, av_rx) = mpsc::unbounded_channel();
  watch("avatars", avatars::run(http.clone(), av_rx, out.clone()), out.clone());
  let (stv_tx, stv_rx) = mpsc::unbounded_channel();
  watch("7tv", seventv::run(http.clone(), stv_rx, stv_tx.clone(), out.clone()), out.clone());
  let Some(dec) = spawn_decoder(out.clone()) else {
    return out.status("can't start the picture decoder, restart heatsync-chat");
  };
  let Some(img_http) = image_client() else {
    return out.status("can't start the picture loader, restart heatsync-chat");
  };
  let gate = Arc::new(Semaphore::new(IMAGE_FETCHES));
  let (sendq, sendq_rx) = mpsc::unbounded_channel();
  let (irc_send, sess_send) = (irc_tx.clone(), sess_tx.clone());
  watch("sending chat", send_queue::run(sendq_rx, move |c| send_line(c, &irc_send, &sess_send)), out.clone());
  // what the next line waits for: the session answers once the emote ops asked before it are done
  let mut hold = None;
  while let Some(cmd) = rx.recv().await {
    match cmd {
      Cmd::Join(ch) => {
        let tab = open.join(&ch);
        tokio::spawn(recent::fetch(http.clone(), ch.clone(), tab.clone(), out.clone()));
        if ident::twitch_room(&ch) {
          tokio::spawn(channel_badges(http.clone(), ch.clone(), tab.clone(), out.clone()));
          tokio::spawn(links::discover(http.clone(), ch.clone(), tab, out.clone()));
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
          tokio::spawn(kick_badges(http.clone(), slug.clone(), out.clone()));
        }
        let _ = hs_tx.send(c);
      }
      Cmd::Backfill(p, key, archive) => {
        tokio::spawn(links::backfill(http.clone(), p, key, archive, out.clone()));
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
      Cmd::Mute(op) => {
        let _ = sess_tx.send(session::Cmd::Mute(op));
      }
      Cmd::Mod(r) => {
        let _ = sess_tx.send(session::Cmd::Mod(r));
      }
      Cmd::Held(c) => {
        let _ = sess_tx.send(session::Cmd::Held(c));
      }
      Cmd::TagCard(tag) => {
        let _ = sess_tx.send(session::Cmd::TagCard(tag));
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
        tokio::spawn(update::run(http.clone(), c, out.clone()));
      }
      Cmd::Pronouns(uid) => {
        tokio::spawn(pronouns(http.clone(), uid, out.clone()));
      }
      Cmd::Preview(url) => {
        tokio::spawn(preview(http.clone(), url, out.clone()));
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
      Cmd::Emote(c) => {
        let _ = sess_tx.send(session::Cmd::Emote(c));
      }
      Cmd::Catalog { q, epoch, page, stv, ffz } => {
        tokio::spawn(catalog::run(http.clone(), q, epoch, page, (stv, ffz), out.clone()));
      }
      Cmd::Upload(p) => {
        let _ = sess_tx.send(session::Cmd::Upload(p));
      }
      Cmd::Picture(bytes) => {
        let _ = sess_tx.send(session::Cmd::Picture(bytes));
      }
      Cmd::Copy { text, done } => clip::copy(text, done, out.clone()),
      Cmd::Paste { to_picker } => clip::paste(to_picker, out.clone()),
      Cmd::Pick => {
        let _ = sess_tx.send(session::Cmd::Pick);
      }
      Cmd::HoldSends => {
        let (done, wait) = tokio::sync::oneshot::channel();
        let _ = sess_tx.send(session::Cmd::Barrier(done));
        hold = Some(wait);
      }
      c @ Cmd::Send { .. } => {
        let _ = sendq.send((hold.take(), c));
      }
      Cmd::Sender(id, ver) => {
        let _ = snd_tx.send((id, ver));
      }
      Cmd::Avatar(p, login) => {
        let _ = av_tx.send((p, login));
      }
      Cmd::Cosmetics(id) => {
        let _ = cos_tx.send(id);
      }
      Cmd::SevenTvMe(id) => {
        let _ = stv_tx.send(seventv::Cmd::Me(id));
      }
      Cmd::Room(ch, room) => {
        tokio::spawn(channel_emotes(http.clone(), ch, room, stv_tx.clone(), open.clone(), out.clone()));
      }
      Cmd::Image(id, u, h) => {
        tokio::spawn(image(img_http.clone(), gate.clone(), dec.clone(), id, u, h, out.clone()));
      }
    }
  }
}

// a chat line, its turn come (send_queue)
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
