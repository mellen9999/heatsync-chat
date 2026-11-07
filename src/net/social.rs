// heatsync's account-bound calls the whispers and feed tabs make. each command is one task; the
// session hands it the current token (None = logged out). answers go back as NetEvents
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out};
use crate::model::engage;
use crate::model::logs;
use crate::model::message::Platform;
use crate::model::post::{self, Post};
use crate::model::unseen::{self, Surface};
use crate::model::whisper::{self, Fail, Via, Whisper, Who};
use crate::ui::theme;

const MAX_BODY: usize = 4 << 20;
// conversations opened when the whispers tab first shows, and messages read from each
const CONVERSATIONS: usize = 8;
const PER_CONVERSATION: usize = 40;

// one page of the feed. `hot` = the follows feed was empty so this is the hot (or newest) fallback;
// `cold` = even hot had nothing carrying heat
pub struct Page {
  pub page: u32,
  pub posts: Vec<Post>,
  pub hot: bool,
  pub cold: bool,
  pub more: bool,
}

pub enum Cmd {
  // page 1 = a fresh load; later pages append
  Feed { page: u32 },
  Thread { id: Box<str> },
  // a held post's withheld files (the viewer pressed show)
  Media { id: Box<str> },
  Vote { id: Box<str> },
  // react to a post with an emote (`on`), or take your reaction back off
  React { id: Box<str>, emote: u64, on: bool },
  // `on` = the state wanted: true bookmarks, false takes it off
  Bookmark { id: Box<str>, on: bool },
  // which of these (at most 100) posts you bookmarked
  BookmarkCheck { ids: Vec<Box<str>> },
  // your own post, new words (the server keeps the ten minute window)
  Edit { id: Box<str>, text: String },
  Delete { id: Box<str> },
  // reply_to None = a top-level post
  Post { text: String, reply_to: Option<Box<str>> },
  // `me` = our twitch id (None: a login without twitch, so dms only)
  WhisperHistory { me: Option<u64>, epoch: u32 },
  // `id` is the twitch user id when we already know it, else it is looked up from the login
  WhisperSend { key: u64, login: Box<str>, id: Option<Box<str>>, text: String },
  // a heatsync dm. `id` is their heatsync user id when known, else it is looked up from the login
  // on `platform` (the card's lookup)
  DmSend { key: u64, platform: Platform, login: Box<str>, id: Option<Box<str>>, text: String },
  // take back a dm of yours: `with` is their heatsync id, `id` the dm's (digits)
  DmDelete { with: Box<str>, id: Box<str> },
  // the user card's facts: who, on which platform, and the channel it was opened from
  Card { platform: Platform, login: Box<str>, channel: Option<Box<str>> },
  // follow (want) or unfollow someone on heatsync; their account is looked up when its id isn't known
  Follow { platform: Platform, login: Box<str>, profile_id: Option<Box<str>>, want: bool },
  // your private note on a heatsync account (empty text clears it)
  PutNote { platform: Platform, login: Box<str>, profile_id: Box<str>, text: String },
  // when you last looked at the mentions, whispers and feed tabs, on any client
  SeenGet,
  // you looked at the heatsync notifications in the mentions tab: the account marks them read, for every device
  NotifsRead,
  // your settings as the account holds them
  UiStateGet,
  // the site's flagged-content switches as the account holds them
  ContentGet,
  // you looked at one just now (`at`); the server tells your other clients
  SeenBump { surface: Surface, at: i64 },
  // one page of a person's archived lines (the chat-log viewer)
  Logs(logs::Req),
  // the gifs tab's search (empty = the top list); `epoch` names the ask
  Gifs { q: Box<str>, epoch: u32 },
  // your mention rules: read, add, switch, remove (net/mrules.rs)
  MentionRules(super::mrules::Op),
}

pub async fn run(http: reqwest::Client, token: Option<String>, cmd: Cmd, out: Out) {
  match cmd {
    Cmd::Feed { page } => feed(&http, token.as_deref(), page, &out).await,
    Cmd::Thread { id } => thread(&http, token.as_deref(), id, &out).await,
    Cmd::Media { id } => media(&http, token.as_deref(), id, &out).await,
    Cmd::Vote { id } => vote(&http, token.as_deref(), id, &out).await,
    Cmd::React { id, emote, on } => react(&http, token.as_deref(), (id, emote), on, &out).await,
    Cmd::Bookmark { id, on } => bookmark(&http, token.as_deref(), id, on, &out).await,
    Cmd::BookmarkCheck { ids } => bookmark_check(&http, token.as_deref(), ids, &out).await,
    Cmd::Edit { id, text } => edit(&http, token.as_deref(), id, text, &out).await,
    Cmd::Delete { id } => delete(&http, token.as_deref(), id, &out).await,
    Cmd::Post { text, reply_to } => publish(&http, token.as_deref(), text, reply_to, &out).await,
    Cmd::WhisperHistory { me, epoch } => {
      if let Some(t) = token {
        history(&http, &t, me, epoch, &out).await;
      }
    }
    Cmd::UiStateGet => {
      if let Some(t) = token {
        ui_state_get(&http, &t, &out).await;
      }
    }
    Cmd::ContentGet => {
      if let Some(t) = token {
        content_get(&http, &t, &out).await;
      }
    }
    Cmd::NotifsRead => {
      if let Some(t) = token {
        let body = json!({ "type": "all" }).to_string();
        let (status, _) = call(authed(&http, &t, "/api/notifications/mark-read", body)).await;
        // still unread on the account then, and the next look says so
        if status != 200 {
          eprintln!("notifications/mark-read: heatsync said {status}");
        }
      }
    }
    Cmd::SeenGet => {
      if let Some(t) = token {
        seen_get(&http, &t, &out).await;
      }
    }
    Cmd::SeenBump { surface, at } => {
      if let Some(t) = token {
        let body = json!({ "surface": surface.key(), "at": at }).to_string();
        call(authed(&http, &t, "/api/user/seen-state", body)).await;
      }
    }
    Cmd::MentionRules(op) => super::mrules::run(http, token, op, out).await,
    Cmd::Logs(req) => archive(&http, req, &out).await,
    Cmd::Gifs { q, epoch } => gifs(&http, q, epoch, &out).await,
    Cmd::Card { platform, login, channel } => card(&http, token.as_deref(), platform, login, channel, &out).await,
    Cmd::Follow { platform, login, profile_id, want } => {
      let res = match token {
        Some(t) => follow(&http, &t, platform, &login, profile_id, want).await,
        None => Err(FollowFail::Auth),
      };
      void(out.send(NetEvent::Followed { platform, login, want, res }))
    }
    Cmd::PutNote { platform, login, profile_id, text } => {
      let ok = match token {
        Some(t) => {
          let body = json!({ "note": &text }).to_string();
          let url = format!("{ORIGIN}/api/user-notes/{}", crate::util::path_seg(&profile_id));
          let req = http.put(url).bearer_auth(t).header("content-type", "application/json").body(body);
          call(req).await.0 == 200
        }
        None => false,
      };
      void(out.send(NetEvent::NoteSaved { platform, login, text, ok }))
    }
    Cmd::DmSend { key, platform, login, id, text } => {
      let res = match token {
        Some(t) => send_dm(&http, &t, platform, &login, id, &text).await,
        None => Err(Fail::Auth),
      };
      // heatsync's id for it first, so the row can be taken back as soon as it shows as sent
      let res = res.map(|(who, stored)| {
        if let Some(id) = stored {
          out.send(NetEvent::DmStored { key, id });
        }
        who
      });
      out.send(NetEvent::WhisperSent { key, res });
    }
    Cmd::DmDelete { with, id } => super::dmdel::run(http, token, with, id, out).await,
    Cmd::WhisperSend { key, login, id, text } => {
      let res = match token {
        Some(t) => send(&http, &t, &login, id, &text).await,
        None => Err(Fail::Auth),
      };
      out.send(NetEvent::WhisperSent { key, res });
    }
  }
}

// status + json body, the body capped. a network failure is status 0
pub(super) async fn call(req: reqwest::RequestBuilder) -> (u16, Value) {
  let Ok(mut r) = req.send().await else { return (0, Value::Null) };
  let status = r.status().as_u16();
  if r.content_length().is_some_and(|n| n > MAX_BODY as u64) {
    return (status, Value::Null);
  }
  let mut buf = Vec::new();
  while let Ok(Some(c)) = r.chunk().await {
    if buf.len() + c.len() > MAX_BODY {
      return (status, Value::Null);
    }
    buf.extend_from_slice(&c);
  }
  (status, serde_json::from_slice(&buf).unwrap_or(Value::Null))
}

fn get(http: &reqwest::Client, token: &str, path: &str) -> reqwest::RequestBuilder {
  http.get(format!("{ORIGIN}{path}")).bearer_auth(token)
}

// a read that works logged out too (the feed and threads are public)
fn read(http: &reqwest::Client, token: Option<&str>, path: &str) -> reqwest::RequestBuilder {
  let r = http.get(format!("{ORIGIN}{path}"));
  match token {
    Some(t) => r.bearer_auth(t),
    None => r,
  }
}

fn posts_of(v: Option<&Value>, ops_only: bool, drop_anon: bool) -> Vec<Post> {
  let rows = v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  rows
    .iter()
    .take(MAX_ROWS)
    .filter_map(post::parse)
    .filter(|p| (!ops_only || p.is_op) && !(drop_anon && p.anon))
    .collect()
}

const PER_PAGE: usize = 30;
const MAX_ROWS: usize = 150;

// ext fetchFeed: the follows timeline, and on an empty first page the hot posts of the month
async fn feed(http: &reqwest::Client, token: Option<&str>, page: u32, out: &Out) {
  let path = format!("/api/messages?sort=time&limit={PER_PAGE}&page={page}&following=true");
  let (status, v) = call(read(http, token, &path)).await;
  if status != 200 {
    return void(out.send(NetEvent::FeedFailed { status }));
  }
  let posts = posts_of(v.get("messages"), true, true);
  let more = v.pointer("/pagination/hasMore").and_then(Value::as_bool).unwrap_or(posts.len() >= PER_PAGE);
  if page == 1 && posts.is_empty() {
    let (st, h) = call(read(http, token, "/api/messages/hot?limit=30&hours=720")).await;
    let hot = if st == 200 { posts_of(h.get("messages"), true, false) } else { Vec::new() };
    if !hot.is_empty() {
      let cold = h.get("fallback").and_then(Value::as_bool) == Some(true);
      return void(out.send(NetEvent::Feed(Page { page, posts: hot, hot: true, cold, more: false })));
    }
  }
  void(out.send(NetEvent::Feed(Page { page, posts, hot: false, cold: false, more })))
}

fn void(_: bool) {}

// the archive is public (no login): a page of one person's lines, or the status that said why not
async fn archive(http: &reqwest::Client, req: logs::Req, out: &Out) {
  let r = http.get(format!("{ORIGIN}{}", req.path())).timeout(std::time::Duration::from_secs(12));
  let (status, v) = call(r).await;
  let res = if status == 200 { Ok(logs::parse_page(&v, &req.who)) } else { Err(status) };
  void(out.send(NetEvent::Logs { epoch: req.epoch, res }))
}

// the gif corpus is public (no login, no cookies): one page, the status that said why not
async fn gifs(http: &reqwest::Client, q: Box<str>, epoch: u32, out: &Out) {
  let r = http.get(format!("{ORIGIN}{}", crate::model::gifs::path(&q))).timeout(std::time::Duration::from_secs(10));
  let (status, v) = call(r).await;
  let res = if status == 200 {
    Ok(crate::model::gifs::parse(&v))
  } else {
    Err(crate::model::gifs::Fail::of(status, &v, crate::util::time::now_ms()))
  };
  void(out.send(NetEvent::Gifs { epoch, q, res }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowFail {
  Auth,
  // no heatsync account behind the name
  NotOnHeatsync,
  // already as asked (true = already following)
  Already(bool),
  Limited,
  Other,
}

// ext pcToggleFollow: POST (follow) / DELETE (unfollow) /api/follow/:id, the id from the card when not known
async fn follow(
  http: &reqwest::Client,
  token: &str,
  platform: Platform,
  login: &str,
  profile_id: Option<Box<str>>,
  want: bool,
) -> Result<(), FollowFail> {
  let id = match profile_id {
    Some(id) => id,
    None => {
      let body = json!({ "platform": platform.name(), "login": login }).to_string();
      let (status, v) = call(authed(http, token, "/api/card", body)).await;
      if status != 200 {
        return Err(if status == 429 { FollowFail::Limited } else { FollowFail::Other });
      }
      let info = crate::model::card::parse(&v, crate::util::time::now_ms());
      if info.follow == Some(want) {
        return Err(FollowFail::Already(want));
      }
      info.profile_id.ok_or(FollowFail::NotOnHeatsync)?
    }
  };
  let url = format!("{ORIGIN}/api/follow/{}", crate::util::path_seg(&id));
  let req = if want { http.post(url) } else { http.delete(url) };
  let (status, v) = call(req.bearer_auth(token)).await;
  let words = v.get("error").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
  match status {
    200..=299 => Ok(()),
    _ if words.contains("already following") => Err(FollowFail::Already(true)),
    _ if words.contains("not following") => Err(FollowFail::Already(false)),
    401 => Err(FollowFail::Auth),
    404 => Err(FollowFail::NotOnHeatsync),
    429 => Err(FollowFail::Limited),
    _ => Err(FollowFail::Other),
  }
}

// POST /api/card: profile, followage, archive counts in one trip (works logged out; a login adds how you relate)
async fn card(
  http: &reqwest::Client,
  token: Option<&str>,
  platform: Platform,
  login: Box<str>,
  channel: Option<Box<str>>,
  out: &Out,
) {
  let who = platform.name();
  let mut body = json!({ "platform": who, "login": &*login });
  if let Some(c) = channel.filter(|_| platform == Platform::Twitch) {
    body["channel"] = json!(&*c);
  }
  let mut req =
    http.post(format!("{ORIGIN}/api/card")).header("content-type", "application/json").body(body.to_string());
  if let Some(t) = token {
    req = req.bearer_auth(t);
  }
  let (status, v) = call(req).await;
  let mut res =
    if status == 200 { Ok(crate::model::card::parse(&v, crate::util::time::now_ms())) } else { Err(status) };
  // a linked twitch account keeps its twitch color (ext hsResolveUserColor step 1), and anyone on
  // twitch without a heatsync picture gets twitch's. one anonymous read, 3s at most; a miss leaves
  // the heatsync color and no picture. the answer is cached with the card
  if let Ok(info) = &mut res
    && platform == Platform::Twitch
    && (info.twitch_name.is_some() || info.avatar.is_none())
  {
    let name = info.twitch_name.clone().unwrap_or_else(|| login.clone());
    let (c, pic) = twitch_user(http, &name).await;
    if info.twitch_name.is_some() && c.is_some() {
      info.color = c;
    }
    info.avatar = info.avatar.take().or(pic);
  }
  void(out.send(NetEvent::Card { platform, login, res }))
}

// twitch's own chat color and picture for a login: the public web gql, no auth (ext hsResolveUserColor)
async fn twitch_user(http: &reqwest::Client, login: &str) -> (Option<u32>, Option<Box<str>>) {
  let body = json!({
    "query": "query($login:String!){user(login:$login){chatColor profileImageURL(width:70)}}",
    "variables": { "login": login },
  });
  let req = http
    .post(crate::badges::fetch::GQL)
    .header("Client-Id", crate::badges::fetch::GQL_CLIENT_ID)
    .timeout(std::time::Duration::from_secs(3))
    .body(body.to_string());
  let (status, v) = call(req).await;
  if status != 200 {
    return (None, None);
  }
  (crate::model::card::chat_color(&v), crate::model::card::twitch_avatar(&v))
}

// the OP and its replies, replies oldest first (the server pages newest first)
async fn thread(http: &reqwest::Client, token: Option<&str>, id: Box<str>, out: &Out) {
  let (status, v) = call(read(http, token, &format!("/api/thread/{id}"))).await;
  let op = (status == 200).then(|| v.get("original").and_then(post::parse)).flatten();
  let res = op.map(|op| {
    let mut replies = posts_of(v.get("replies"), false, false);
    replies.reverse();
    (op, replies)
  });
  void(out.send(NetEvent::Thread { id, res: res.ok_or(status) }))
}

// logged out reads as not cleared: either way the answer is to log in as an adult
async fn media(http: &reqwest::Client, token: Option<&str>, id: Box<str>, out: &Out) {
  let (status, v) = call(read(http, token, &format!("/api/messages/{}/media", crate::util::path_seg(&id)))).await;
  // 403 says why: not logged in (log in), or this account isn't cleared for it (451 here: no button helps)
  let res = match status {
    200 => Ok(post::media_of(&v)),
    401 => Err(403),
    403 if v.get("code").and_then(Value::as_str) == Some("not_available") => Err(451),
    s => Err(s),
  };
  void(out.send(NetEvent::Media { id, res }))
}

fn authed(http: &reqwest::Client, token: &str, path: &str, body: String) -> reqwest::RequestBuilder {
  http.post(format!("{ORIGIN}{path}")).bearer_auth(token).header("content-type", "application/json").body(body)
}

async fn ui_state_get(http: &reqwest::Client, token: &str, out: &Out) {
  let (status, v) = call(get(http, token, "/api/user/ui-state")).await;
  if status != 200 {
    return;
  }
  let (state, _) = crate::model::sync::pluck(v.get("state").unwrap_or(&Value::Null));
  if !state.is_empty() {
    void(out.send(NetEvent::UiState(state)));
  }
}

async fn content_get(http: &reqwest::Client, token: &str, out: &Out) {
  let (status, v) = call(get(http, token, "/api/user/settings")).await;
  let prefs = crate::net::pushed::content_prefs(&v);
  if status == 200 && !prefs.is_empty() {
    void(out.send(NetEvent::ContentPrefs(prefs)));
  }
}

async fn seen_get(http: &reqwest::Client, token: &str, out: &Out) {
  let (status, v) = call(get(http, token, "/api/user/seen-state")).await;
  if status != 200 {
    return;
  }
  for surface in unseen::ALL {
    if let Some(at) = unseen::at(v.get(surface.key())) {
      void(out.send(NetEvent::Seen { surface, at }));
    }
  }
}

// toggles your upvote. ok = (voted now, heat if the server said)
async fn vote(http: &reqwest::Client, token: Option<&str>, id: Box<str>, out: &Out) {
  let Some(t) = token else { return void(out.send(NetEvent::Voted { id, res: Err(401) })) };
  let (status, v) =
    call(authed(http, t, &format!("/api/messages/{id}/vote"), json!({ "vote_type": 1 }).to_string())).await;
  let res = if status == 200 && v.get("success").and_then(Value::as_bool) == Some(true) {
    let heat = v.get("heat").and_then(Value::as_f64).filter(|h| h.is_finite() && *h >= 0.0).map(|h| h.min(1e9) as u32);
    Ok((v.get("user_vote").and_then(Value::as_i64) == Some(1), heat))
  } else {
    Err(status)
  };
  void(out.send(NetEvent::Voted { id, res }))
}

// POST /api/messages/:id/react {emote_id} adds your reaction (`added:false` = you already had it), DELETE
// .../react/:emote_id takes it off (ext feedReactionToggle). ok = whether the server added it. taking back one
// that is not there (404) is as good as done
async fn react(http: &reqwest::Client, token: Option<&str>, (id, emote): (Box<str>, u64), on: bool, out: &Out) {
  let Some(t) = token else { return void(out.send(NetEvent::Reacted { id, emote, on, res: Err(401) })) };
  let base = format!("/api/messages/{}/react", crate::util::path_seg(&id));
  let (status, v) = if on {
    call(authed(http, t, &base, json!({ "emote_id": emote }).to_string())).await
  } else {
    call(http.delete(format!("{ORIGIN}{base}/{emote}")).bearer_auth(t)).await
  };
  let res = if (200..300).contains(&status) {
    Ok(v.get("added").and_then(Value::as_bool).unwrap_or(true))
  } else if !on && status == 404 {
    Ok(true)
  } else {
    Err(status)
  };
  void(out.send(NetEvent::Reacted { id, emote, on, res }))
}

// POST adds a bookmark, DELETE takes it off (ext feedBookmarkToggle); 2xx = settled
async fn bookmark(http: &reqwest::Client, token: Option<&str>, id: Box<str>, on: bool, out: &Out) {
  let Some(t) = token else { return void(out.send(NetEvent::Bookmarked { id, on, res: Err(401) })) };
  let url = format!("{ORIGIN}/api/bookmarks/{}", crate::util::path_seg(&id));
  let req = if on { http.post(url) } else { http.delete(url) }.bearer_auth(t);
  let (status, _) = call(req).await;
  let res = if (200..300).contains(&status) { Ok(()) } else { Err(status) };
  void(out.send(NetEvent::Bookmarked { id, on, res }))
}

async fn bookmark_check(http: &reqwest::Client, token: Option<&str>, ids: Vec<Box<str>>, out: &Out) {
  let Some(t) = token.filter(|_| !ids.is_empty()) else { return };
  let ids: Vec<Box<str>> = ids.into_iter().take(engage::CHECK_CHUNK).collect();
  let (status, v) = call(authed(http, t, "/api/bookmarks/check", json!({ "message_ids": &ids }).to_string())).await;
  let res = if status == 200 { engage::parse_checked(&v, &ids).ok_or(500) } else { Err(status) };
  void(out.send(NetEvent::Marks { ids, res }))
}

// PATCH /api/messages/:id { content }: the saved words come back
async fn edit(http: &reqwest::Client, token: Option<&str>, id: Box<str>, text: String, out: &Out) {
  let Some(t) = token else { return void(out.send(NetEvent::Edited { id, res: Err(401) })) };
  let url = format!("{ORIGIN}/api/messages/{}", crate::util::path_seg(&id));
  let req = http.patch(url).bearer_auth(t).header("content-type", "application/json");
  let (status, v) = call(req.body(json!({ "content": text }).to_string())).await;
  let res = match engage::parse_edit(&v) {
    Some(saved) if (200..300).contains(&status) => Ok(saved),
    _ => Err(if (200..300).contains(&status) { 500 } else { status }),
  };
  void(out.send(NetEvent::Edited { id, res }))
}

// DELETE /api/messages/:id: a soft delete. already gone (410) is as good as done
async fn delete(http: &reqwest::Client, token: Option<&str>, id: Box<str>, out: &Out) {
  let Some(t) = token else { return void(out.send(NetEvent::Deleted { id, res: Err(401) })) };
  let (status, _) =
    call(http.delete(format!("{ORIGIN}/api/messages/{}", crate::util::path_seg(&id))).bearer_auth(t)).await;
  let res = if (200..300).contains(&status) || status == 410 { Ok(()) } else { Err(status) };
  void(out.send(NetEvent::Deleted { id, res }))
}

async fn publish(http: &reqwest::Client, token: Option<&str>, text: String, reply_to: Option<Box<str>>, out: &Out) {
  let Some(t) = token else { return void(out.send(NetEvent::Posted { res: Err(401) })) };
  let mut body = json!({ "content": text });
  if let Some(r) = &reply_to {
    body["reply_to"] = json!(r);
  }
  let (status, v) = call(authed(http, t, "/api/messages", body.to_string())).await;
  let res = match v.get("message").and_then(post::parse) {
    Some(p) if (200..300).contains(&status) => Ok(p),
    _ => Err(if (200..300).contains(&status) { 500 } else { status }),
  };
  void(out.send(NetEvent::Posted { res }))
}

async fn history(http: &reqwest::Client, token: &str, me: Option<u64>, epoch: u32, out: &Out) {
  let mut all = match me {
    Some(me) => twitch_history(http, token, me, out).await,
    None => Vec::new(),
  };
  all.extend(dm_history(http, token, out).await);
  all.sort_by_key(|w| w.ts);
  out.send(NetEvent::Whispers { items: all, live: false, epoch: Some(epoch) });
}

async fn twitch_history(http: &reqwest::Client, token: &str, me: u64, out: &Out) -> Vec<Whisper> {
  let (status, list) = call(get(http, token, "/api/whispers")).await;
  if status != 200 {
    out.status(format!("whispers: {}", fail_text(&classify(status, &list))));
    return Vec::new();
  }
  let mut all = Vec::new();
  let convos = list.get("conversations").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  for c in convos.iter().take(CONVERSATIONS) {
    let Some(id) =
      c.get("other_twitch_id").and_then(|v| v.as_str().map(String::from).or_else(|| v.as_u64().map(|n| n.to_string())))
    else {
      continue;
    };
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
      continue;
    }
    let login: Box<str> = c
      .get("other_twitch_login")
      .and_then(Value::as_str)
      .unwrap_or("")
      .to_lowercase()
      .chars()
      .take(25)
      .collect::<String>()
      .into();
    let name: Box<str> = c
      .get("other_twitch_display")
      .and_then(Value::as_str)
      .filter(|s| !s.is_empty())
      .unwrap_or(&login)
      .chars()
      .take(100)
      .collect::<String>()
      .into();
    let color = c
      .get("other_color")
      .and_then(Value::as_str)
      .and_then(crate::model::color::parse_hex)
      .map_or(theme::WHITE, crate::model::color::readable);
    let who = Who { via: Via::Twitch, id: id.clone().into(), login, name, color };
    let (status, thread) = call(get(http, token, &format!("/api/whispers/{id}?limit={PER_CONVERSATION}"))).await;
    if status != 200 {
      continue;
    }
    let rows = thread.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    all.extend(rows.iter().take(PER_CONVERSATION).filter_map(|m| whisper::parse(m, me, Some(&who))));
  }
  all
}

// GET /api/dm (newest conversation first), then each one's messages
async fn dm_history(http: &reqwest::Client, token: &str, out: &Out) -> Vec<Whisper> {
  let (status, list) = call(get(http, token, "/api/dm")).await;
  if status != 200 {
    out.status(format!("dms: {}", fail_text(&classify_dm(status, &list, ""))));
    return Vec::new();
  }
  let mut all = Vec::new();
  let convos = list.get("conversations").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
  for c in convos.iter().take(CONVERSATIONS) {
    let Some(who) = dm_person(c) else { continue };
    let url = format!("/api/dm/{}?limit={PER_CONVERSATION}", crate::util::path_seg(&who.id));
    let (status, thread) = call(get(http, token, &url)).await;
    if status != 200 {
      continue;
    }
    let rows = thread.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    all.extend(rows.iter().take(PER_CONVERSATION).filter_map(|m| whisper::parse_dm(m, Some(&who))));
  }
  all
}

// the other side of one GET /api/dm conversation
fn dm_person(c: &Value) -> Option<Who> {
  let id = whisper::user_id(c.get("other_user_id"))?;
  let text = |k: &str| c.get(k).and_then(Value::as_str).map(|s| s.trim().chars().take(100).collect::<String>());
  let login: Box<str> = text("username").unwrap_or_default().to_lowercase().into();
  let name: Box<str> = text("display_name").filter(|n| !n.is_empty()).map_or_else(|| login.clone(), Into::into);
  let color = c
    .get("user_color")
    .and_then(Value::as_str)
    .and_then(crate::model::color::parse_hex)
    .map_or(theme::WHITE, crate::model::color::readable);
  Some(Who { via: Via::Heatsync, id, login, name, color })
}

async fn send(http: &reqwest::Client, token: &str, login: &str, id: Option<Box<str>>, text: &str) -> Result<Who, Fail> {
  let who = match id {
    Some(id) => Who { via: Via::Twitch, id, login: login.into(), name: login.into(), color: theme::WHITE },
    None => resolve(http, login).await?,
  };
  let req = http
    .post(format!("{ORIGIN}/api/twitch/whisper"))
    .bearer_auth(token)
    .header("content-type", "application/json")
    .body(json!({ "toUserId": &*who.id, "message": text }).to_string());
  let (status, v) = call(req).await;
  if (200..300).contains(&status) && v.get("error").is_none() { Ok(who) } else { Err(classify(status, &v)) }
}

pub(super) async fn resolve(http: &reqwest::Client, login: &str) -> Result<Who, Fail> {
  let (status, v) = call(http.get(format!("{ORIGIN}/api/resolve/twitch/{login}"))).await;
  if status == 404 {
    return Err(Fail::Other(format!("twitch user \"{login}\" not found").into()));
  }
  let id = v.get("id").and_then(|i| i.as_str().map(String::from).or_else(|| i.as_u64().map(|n| n.to_string())));
  match id.filter(|i| !i.is_empty() && i.len() <= 20 && i.bytes().all(|b| b.is_ascii_digit())) {
    Some(id) if status == 200 => {
      let name = v.get("displayName").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(login);
      Ok(Who {
        via: Via::Twitch,
        id: id.into(),
        login: login.into(),
        name: name.chars().take(100).collect::<String>().into(),
        color: theme::WHITE,
      })
    }
    _ => Err(Fail::Other("failed to resolve twitch user".into())),
  }
}

// POST /api/dm { toUserId, content }. their heatsync id comes from the card when it isn't known. the person,
// and the id heatsync gave the dm (digits) when its answer said
async fn send_dm(
  http: &reqwest::Client,
  token: &str,
  platform: Platform,
  login: &str,
  id: Option<Box<str>>,
  text: &str,
) -> Result<(Who, Option<Box<str>>), Fail> {
  let who = match id {
    Some(id) => Who { via: Via::Heatsync, id, login: login.into(), name: login.into(), color: theme::WHITE },
    None => {
      let body = json!({ "platform": platform.name(), "login": login }).to_string();
      let (status, v) = call(authed(http, token, "/api/card", body)).await;
      if status != 200 {
        return Err(classify_dm(status, &v, login));
      }
      let info = crate::model::card::parse(&v, crate::util::time::now_ms());
      let Some(id) = info.profile_id else { return Err(Fail::Other(format!("{login} isn't on heatsync").into())) };
      let color = info.color.map_or(theme::WHITE, crate::model::color::readable);
      Who { via: Via::Heatsync, id, login: login.into(), name: login.into(), color }
    }
  };
  let body = json!({ "toUserId": &*who.id, "content": text }).to_string();
  let (status, v) = call(authed(http, token, "/api/dm", body)).await;
  if (200..300).contains(&status) && v.get("error").is_none() {
    Ok((who, whisper::digits(v.get("dm").and_then(|d| d.get("id")))))
  } else {
    Err(classify_dm(status, &v, login))
  }
}

fn fail_text(f: &Fail) -> String {
  whisper::fail_text(f)
}

// why a send failed, by the ext's rules (whispers.js sendTwitchWhisper): a missing whispers permission
// (the server flags it, or says re-link) beats a plain logged-out answer; 5xx or no answer is the server
pub fn classify(status: u16, v: &Value) -> Fail {
  let err = v.get("error").and_then(Value::as_str).unwrap_or("");
  let low = err.to_ascii_lowercase();
  let flagged = status == 401
    && (v.get("relink_required").and_then(Value::as_bool) == Some(true)
      || v.get("scope_pack").and_then(Value::as_str) == Some("whispers"));
  if flagged || low.contains("relink") || low.contains("re-link") {
    Fail::Relink
  } else if status == 401 && ["no twitch token", "not authenticated", "re-login"].iter().any(|k| low.contains(k)) {
    Fail::Auth
  } else if status == 0 || status >= 500 {
    Fail::Server
  } else if err.is_empty() {
    Fail::Other(super::http::plain_status(status).into())
  } else {
    Fail::Other(super::http::server_words(err).into())
  }
}

// why a dm failed (ext whispers.js): the server's `code` is the contract, its prose is for humans
pub fn classify_dm(status: u16, v: &Value, login: &str) -> Fail {
  let code = v.get("code").and_then(Value::as_str).unwrap_or("");
  let err = v.get("error").and_then(Value::as_str).unwrap_or("");
  if code == "mutual_follow_required" {
    Fail::Follow(login.into())
  } else if status == 401 {
    Fail::Auth
  } else if status == 0 || status >= 500 {
    Fail::Server
  } else if err.is_empty() {
    Fail::Other(super::http::plain_status(status).into())
  } else {
    Fail::Other(super::http::server_words(err).into())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn dm_failures_read_plainly() {
    let m = json!({"error": "mutual follow required to send DMs", "code": "mutual_follow_required"});
    assert_eq!(classify_dm(403, &m, "amy"), Fail::Follow("amy".into()));
    assert_eq!(classify_dm(401, &Value::Null, "amy"), Fail::Auth);
    assert_eq!(classify_dm(0, &Value::Null, "amy"), Fail::Server);
    assert_eq!(classify_dm(500, &json!({}), "amy"), Fail::Server);
    assert_eq!(classify_dm(403, &json!({"error": "blocked", "code": "blocked"}), "amy"), Fail::Other("blocked".into()));
    assert_eq!(classify_dm(400, &Value::Null, "amy"), Fail::Other("that didn't work, try again".into()));
    let long = json!({"error": "x\u{7}".repeat(300)});
    assert_eq!(classify_dm(400, &long, "a"), Fail::Other("x".repeat(100).into()));
  }

  #[test]
  fn a_dm_conversation_names_the_other_side() {
    let c = json!({"other_user_id": "77", "username": "Amy", "display_name": "Amy_X", "user_color": "#00ff00"});
    let w = dm_person(&c).unwrap();
    assert_eq!((w.via, &*w.id, &*w.login, &*w.name), (Via::Heatsync, "77", "amy", "Amy_X"));
    assert!(dm_person(&json!({"other_user_id": "../x"})).is_none());
    assert!(dm_person(&json!({"username": "amy"})).is_none());
    assert_eq!(&*dm_person(&json!({"other_user_id": 5, "username": "Bo"})).unwrap().name, "bo");
  }

  #[test]
  fn failures_are_classified_like_the_ext() {
    let relink = json!({"error": "No Twitch token — re-link on heatsync.org", "relink_required": true});
    assert_eq!(classify(401, &relink), Fail::Relink);
    assert_eq!(classify(403, &json!({"error": "Please re-link twitch"})), Fail::Relink);
    assert_eq!(classify(401, &json!({"error": "Not authenticated with Twitch"})), Fail::Auth);
    assert_eq!(classify(0, &Value::Null), Fail::Server);
    assert_eq!(classify(502, &json!({})), Fail::Server);
    assert_eq!(classify(400, &json!({"error": "Message too long"})), Fail::Other("message too long".into()));
    assert_eq!(classify(429, &Value::Null), Fail::Other("slow down, try again in a bit".into()));
    // a bare 401 with an unknown reason is shown as the server said it
    assert_eq!(classify(401, &json!({"error": "nope"})), Fail::Other("nope".into()));
  }
}
