// heatsync's account-bound calls the whispers and feed tabs make. each command is one task; the
// session hands it the current token (None = logged out). answers go back as NetEvents
use serde_json::{Value, json};

use super::{NetEvent, ORIGIN, Out};
use crate::model::message::Platform;
use crate::model::post::{self, Post};
use crate::model::unseen::{self, Surface};
use crate::model::whisper::{self, Fail, Who};
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
  // reply_to None = a top-level post
  Post { text: String, reply_to: Option<Box<str>> },
  WhisperHistory { me: u64, epoch: u32 },
  // `id` is the twitch user id when we already know it, else it is looked up from the login
  WhisperSend { key: u64, login: Box<str>, id: Option<Box<str>>, text: String },
  // the user card's facts: who, on which platform, and the channel it was opened from
  Card { platform: Platform, login: Box<str>, channel: Option<Box<str>> },
  // your private note on a heatsync account (empty text clears it)
  PutNote { platform: Platform, login: Box<str>, profile_id: Box<str>, text: String },
  // when you last looked at the mentions, whispers and feed tabs, on any client
  SeenGet,
  // you looked at one just now (`at`); the server tells your other clients
  SeenBump { surface: Surface, at: i64 },
}

pub async fn run(http: reqwest::Client, token: Option<String>, cmd: Cmd, out: Out) {
  match cmd {
    Cmd::Feed { page } => feed(&http, token.as_deref(), page, &out).await,
    Cmd::Thread { id } => thread(&http, token.as_deref(), id, &out).await,
    Cmd::Media { id } => media(&http, token.as_deref(), id, &out).await,
    Cmd::Vote { id } => vote(&http, token.as_deref(), id, &out).await,
    Cmd::Post { text, reply_to } => publish(&http, token.as_deref(), text, reply_to, &out).await,
    Cmd::WhisperHistory { me, epoch } => {
      if let Some(t) = token {
        history(&http, &t, me, epoch, &out).await;
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
    Cmd::Card { platform, login, channel } => card(&http, token.as_deref(), platform, login, channel, &out).await,
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
async fn call(req: reqwest::RequestBuilder) -> (u16, Value) {
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

async fn history(http: &reqwest::Client, token: &str, me: u64, epoch: u32, out: &Out) {
  let (status, list) = call(get(http, token, "/api/whispers")).await;
  if status != 200 {
    return out.status(format!("whispers: {}", fail_text(&classify(status, &list))));
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
    let who = Who { id: id.clone().into(), login, name, color };
    let (status, thread) = call(get(http, token, &format!("/api/whispers/{id}?limit={PER_CONVERSATION}"))).await;
    if status != 200 {
      continue;
    }
    let rows = thread.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    all.extend(rows.iter().take(PER_CONVERSATION).filter_map(|m| whisper::parse(m, me, Some(&who))));
  }
  all.sort_by_key(|w| w.ts);
  out.send(NetEvent::Whispers { items: all, live: false, epoch: Some(epoch) });
}

async fn send(http: &reqwest::Client, token: &str, login: &str, id: Option<Box<str>>, text: &str) -> Result<Who, Fail> {
  let who = match id {
    Some(id) => Who { id, login: login.into(), name: login.into(), color: theme::WHITE },
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
        id: id.into(),
        login: login.into(),
        name: name.chars().take(100).collect::<String>().into(),
        color: theme::WHITE,
      })
    }
    _ => Err(Fail::Other("failed to resolve twitch user".into())),
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
    Fail::Other(format!("twitch error {status}").into())
  } else {
    Fail::Other(err.chars().take(100).collect::<String>().into())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn failures_are_classified_like_the_ext() {
    let relink = json!({"error": "No Twitch token — re-link on heatsync.org", "relink_required": true});
    assert_eq!(classify(401, &relink), Fail::Relink);
    assert_eq!(classify(403, &json!({"error": "Please re-link twitch"})), Fail::Relink);
    assert_eq!(classify(401, &json!({"error": "Not authenticated with Twitch"})), Fail::Auth);
    assert_eq!(classify(0, &Value::Null), Fail::Server);
    assert_eq!(classify(502, &json!({})), Fail::Server);
    assert_eq!(classify(400, &json!({"error": "Message too long"})), Fail::Other("Message too long".into()));
    assert_eq!(classify(429, &Value::Null), Fail::Other("twitch error 429".into()));
    // a bare 401 with an unknown reason is shown as the server said it
    assert_eq!(classify(401, &json!({"error": "nope"})), Fail::Other("nope".into()));
  }
}
