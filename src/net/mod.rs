// all network io lives on one background thread running a current-thread tokio runtime.
// the ui talks to it through `Net` and hears back through winit user events
pub mod auth;
pub mod blocked;
mod catalog;
mod cosmetics;
pub mod emote_ops;
pub mod held;
pub mod hs_ws;
pub mod irc;
mod links;
pub mod modact;
pub mod modes;
pub mod modsuite;
pub mod modtools;
pub mod mutes;
pub mod pin;
mod recent;
pub mod relay;
mod senders;
mod session;
pub mod seventv;
pub mod social;
pub mod update;
pub mod upload;
mod youtube;

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
  // channel = None for globals
  Emotes(Option<Box<str>>, Provider, Set),
  Image(ImgId, u32, Result<decode::Decoded, String>),
  TwitchBadges(Option<Box<str>>, TwitchSet),
  // a channel's own ffz mod / vip badge art
  FfzRoomBadges(Box<str>, TwitchSet),
  // kick/youtube lines relayed by heatsync, keyed by kick slug / youtube handle
  Relay {
    platform: Platform,
    key: Box<str>,
    msgs: Vec<Message>,
    history: bool,
  },
  RelayClear {
    platform: Platform,
    key: Box<str>,
    login: Box<str>,
  },
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
  SendersStale {
    ids: Vec<u64>,
    ver: Option<Box<str>>,
    removed: Option<Box<str>>,
  },
  // logged in as / logged out
  Me(Option<auth::Me>),
  // the server's blocked-emote hashes for this login
  BlockedHashes(Vec<Box<str>>),
  // our own heatsync inventory changed (an add, or a removal of `removed`): refetch it
  Inventory {
    removed: Option<Box<str>>,
  },
  Upload(upload::Upload),
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
  // a /warn, /shoutout or /shield settled (Ok(Some(on)) = shield mode now)
  Suite {
    chan: Box<str>,
    cmd: crate::model::modsuite::Cmd,
    res: Result<Option<bool>, blocked::Fail>,
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

// the line being answered: its platform message id, and @nick + its words for our own echo
pub struct Reply {
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
  // twitch's mod suite in a room (by login): warn, shoutout, shield mode
  Suite(Box<str>, crate::model::modsuite::Cmd),
  // the mod tools screen: load, shield, an unban request, an automod level
  Tools(Box<str>, modtools::Op),
  Emote(emote_ops::EmoteCmd),
  // search the 7tv / ffz catalogs for tab-complete: the word, the ask's number, which page, from whom
  Catalog { q: Box<str>, epoch: u32, page: u32, stv: bool, ffz: bool },
  // upload this file / ask a file dialog for one, then upload it
  Upload(std::path::PathBuf),
  // a pasted picture: width, height, rgba bytes
  Pixels(u32, u32, Vec<u8>),
  Pick,
  // chatters' 7tv paints + badges
  Cosmetics(crate::cosmetics::id::PaintId),
  // our own twitch id, for watching our own 7tv cosmetics
  SevenTvMe(Option<u64>),
  Send { platform: Platform, chan: Box<str>, text: String, reply: Option<Reply> },
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
      let res = decode::decode(&j.bytes, j.h);
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
  match capped(http.get(&*url).send().await, MAX_IMAGE_BYTES).await {
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
  let p = capped(res, MAX_PREVIEW)
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
      r => capped(r, MAX_TAG_CARD)
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
  let res = capped(r, MAX_PRONOUNS)
    .await
    .ok()
    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    .map(|v| crate::model::pronouns::parse(&v, uid))
    .ok_or(());
  out.send(NetEvent::Pronouns(uid, res));
}

// the body, abandoned as soon as it passes `max` (a hostile host can't make us buffer it all)
async fn capped(res: Result<reqwest::Response, reqwest::Error>, max: usize) -> Result<Vec<u8>, String> {
  let mut r = res.map_err(|e| e.to_string())?;
  if !r.status().is_success() {
    return Err(format!("http {}", r.status().as_u16()));
  }
  if r.content_length().is_some_and(|n| n > max as u64) {
    return Err("too large".into());
  }
  let mut out = Vec::new();
  while let Some(c) = r.chunk().await.map_err(|e| e.to_string())? {
    if out.len() + c.len() > max {
      return Err("too large".into());
    }
    out.extend_from_slice(&c);
  }
  Ok(out)
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

async fn channel_emotes(
  http: reqwest::Client,
  ch: Box<str>,
  room: Box<str>,
  stv: mpsc::UnboundedSender<seventv::Cmd>,
  out: Out,
) {
  if !room.bytes().all(|b| b.is_ascii_digit()) {
    return;
  }
  for (p, u) in providers::channel_urls(&room) {
    match providers::fetch(&http, &u).await {
      Ok(v) => {
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
        out.send(NetEvent::Emotes(Some(ch.clone()), p, providers::parse_channel(p, &v)));
      }
      Err(e) => out.status(format!("{p:?} emotes for {ch}: {e}").to_lowercase()),
    }
  }
}

async fn global_badges(http: reqwest::Client, out: Out) {
  match badges::fetch::twitch_global(&http).await {
    Ok(s) => {
      out.send(NetEvent::TwitchBadges(None, s));
    }
    Err(e) => out.status(format!("twitch badges: {e}")),
  }
  type Parse = fn(&serde_json::Value) -> UserBadges;
  let third: [(Source, &str, Parse); 2] = [
    (Source::Chatterino, badges::fetch::CHATTERINO, badges::fetch::chatterino),
    (Source::Ffz, badges::fetch::FFZ_IDS, badges::fetch::ffz),
  ];
  for (src, u, parse) in third {
    match providers::fetch(&http, u).await {
      Ok(v) => {
        out.send(NetEvent::UserBadges(src, parse(&v)));
      }
      Err(e) => out.status(format!("{src:?} badges: {e}").to_lowercase()),
    }
  }
}

async fn channel_badges(http: reqwest::Client, ch: Box<str>, out: Out) {
  match badges::fetch::twitch_channel(&http, &ch).await {
    Ok(s) => {
      out.send(NetEvent::TwitchBadges(Some(ch), s));
    }
    Err(e) => out.status(format!("badges for {ch}: {e}")),
  }
}

async fn global_emotes(http: reqwest::Client, out: Out) {
  type Parse = fn(&serde_json::Value) -> Set;
  // twitch's: the ext's list (every helix global: LUL, :P) laid over the catalog's twitch rows (the
  // event unlocks, which that list lacks). either alone is short; one failing still leaves the other
  let mut twitch = Set::new();
  for u in [providers::TWITCH_GLOBAL, providers::HEATSYNC_GLOBAL] {
    match providers::fetch(&http, u).await {
      Ok(v) => providers::merge(&mut twitch, providers::twitch_global(&v)),
      Err(e) => out.status(format!("global twitch emotes: {e}")),
    }
  }
  out.send(NetEvent::Emotes(None, Provider::Twitch, twitch));
  let sets: [(Provider, &str, Parse); 3] = [
    (Provider::SevenTv, providers::SEVENTV_GLOBAL, providers::seventv),
    (Provider::Bttv, providers::BTTV_GLOBAL, providers::bttv_global),
    (Provider::Ffz, providers::FFZ_GLOBAL, providers::ffz_global),
  ];
  for (p, u, parse) in sets {
    match providers::fetch(&http, u).await {
      Ok(v) => {
        out.send(NetEvent::Emotes(None, p, parse(&v)));
      }
      Err(e) => out.status(format!("global {p:?} emotes: {e}").to_lowercase()),
    }
  }
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
        Err(e) => return out.status(format!("network runtime failed: {e}")),
      };
      rt.block_on(main(rx, out));
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

async fn main(mut rx: mpsc::UnboundedReceiver<Cmd>, out: Out) {
  let http = match reqwest::Client::builder()
    .user_agent(USER_AGENT)
    .connect_timeout(Duration::from_secs(8))
    .timeout(Duration::from_secs(12))
    .build()
  {
    Ok(c) => c,
    Err(e) => return out.status(format!("http client failed: {e}")),
  };
  let (irc_tx, irc_rx) = mpsc::unbounded_channel();
  tokio::spawn(irc::run(irc_rx, out.clone()));
  tokio::spawn(global_emotes(http.clone(), out.clone()));
  tokio::spawn(global_badges(http.clone(), out.clone()));
  let (hs_tx, hs_rx) = mpsc::unbounded_channel();
  let (pin_tx, pin_rx) = mpsc::unbounded_channel();
  tokio::spawn(pin::run(http.clone(), pin_rx, out.clone()));
  tokio::spawn(hs_ws::run(hs_rx, pin_tx, out.clone()));
  let hs_auth = hs_tx.clone();
  let (snd_tx, snd_rx) = mpsc::unbounded_channel();
  tokio::spawn(senders::run(http.clone(), snd_rx, out.clone()));
  let (sess_tx, sess_rx) = mpsc::unbounded_channel();
  tokio::spawn(session::run(http.clone(), sess_rx, irc_tx.clone(), hs_auth, out.clone()));
  let (cos_tx, cos_rx) = mpsc::unbounded_channel();
  tokio::spawn(cosmetics::run(http.clone(), cos_rx, out.clone()));
  let (stv_tx, stv_rx) = mpsc::unbounded_channel();
  tokio::spawn(seventv::run(http.clone(), stv_rx, stv_tx.clone(), out.clone()));
  let Some(dec) = spawn_decoder(out.clone()) else {
    return out.status("image decoder failed to start");
  };
  let Some(img_http) = image_client() else {
    return out.status("image client failed to start");
  };
  let gate = Arc::new(Semaphore::new(IMAGE_FETCHES));
  while let Some(cmd) = rx.recv().await {
    match cmd {
      Cmd::Join(ch) => {
        tokio::spawn(recent::fetch(http.clone(), ch.clone(), out.clone()));
        if irc::valid_channel(&ch) {
          tokio::spawn(channel_badges(http.clone(), ch.clone(), out.clone()));
          tokio::spawn(links::discover(http.clone(), ch.clone(), out.clone()));
          let _ = hs_tx.send(hs_ws::Cmd::Join(Platform::Twitch, ch.clone()));
        }
        let _ = irc_tx.send(irc::Cmd::Join(ch));
      }
      Cmd::Part(ch) => {
        let _ = stv_tx.send(seventv::Cmd::Unwatch(ch.clone()));
        let _ = hs_tx.send(hs_ws::Cmd::Leave(Platform::Twitch, ch.clone()));
        let _ = irc_tx.send(irc::Cmd::Part(ch));
      }
      Cmd::Hs(c) => {
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
      Cmd::Suite(chan, c) => {
        let _ = sess_tx.send(session::Cmd::Suite(chan, c));
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
      Cmd::Pixels(w, h, px) => {
        let _ = sess_tx.send(session::Cmd::Pixels(w, h, px));
      }
      Cmd::Pick => {
        let _ = sess_tx.send(session::Cmd::Pick);
      }
      Cmd::Send { platform: Platform::Twitch, chan, text, reply } => {
        let _ = irc_tx.send(irc::Cmd::Send { chan, text, reply });
      }
      Cmd::Send { platform: Platform::Kick, chan, text, reply } => {
        let _ = sess_tx.send(session::Cmd::Kick { chan, text, reply: reply.map(|r| r.id) });
      }
      Cmd::Send { platform: Platform::Youtube, chan, text, .. } => {
        let _ = sess_tx.send(session::Cmd::Youtube { chan, text });
      }
      Cmd::Sender(id, ver) => {
        let _ = snd_tx.send((id, ver));
      }
      Cmd::Cosmetics(id) => {
        let _ = cos_tx.send(id);
      }
      Cmd::SevenTvMe(id) => {
        let _ = stv_tx.send(seventv::Cmd::Me(id));
      }
      Cmd::Room(ch, room) => {
        tokio::spawn(channel_emotes(http.clone(), ch, room, stv_tx.clone(), out.clone()));
      }
      Cmd::Image(id, u, h) => {
        tokio::spawn(image(img_http.clone(), gate.clone(), dec.clone(), id, u, h, out.clone()));
      }
    }
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
