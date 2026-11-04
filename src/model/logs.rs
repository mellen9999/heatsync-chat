// the chat-log viewer's data (ext chat-logs.js): one person's archived lines, read newest first from
// heatsync's archive and shown oldest first, newest at the bottom. the request, the hostile page parse,
// the cursor paging with its caps, the scope (this channel / all) and the permalink live here;
// app/logs.rs drives it. every byte of a page is hostile: bounded, typed, urls allowlisted
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::Value;

use super::message::{Extra, HsEmote, Kind, Message, Placed, Platform, Seg, flag};
use crate::heatsync::{inventory_url, valid_name};
use crate::images::url::allowed;
use crate::model::color;
use crate::ui::theme;
use crate::util::time::{parse_iso, utc_day, utc_stamp};
use crate::util::{clamp, path_seg};

// lines per request (the server's own ceiling)
pub const PAGE: usize = 100;
// lines kept in memory (and in the view)
pub const CAP: usize = 2000;
// empty pages followed in a row before a sparse search waits for the next scroll
const HOPS: u8 = 6;
const MAX_TEXT: usize = 2000;
const MAX_NAME: usize = 100;
const MAX_BADGES: usize = 20;
const MAX_EMOTES: usize = 50;
const MAX_CURSOR: usize = 100;
const MAX_HTML: usize = 16 << 10;
// the search the server takes (`q` ≤ 200)
pub const MAX_QUERY: usize = 200;
const PUBLIC: &str = "https://heatsync.org";

// every request gets its own number so an answer from before a scope or search switch (or from a
// view since closed) is dropped when it lands
static EPOCH: AtomicU32 = AtomicU32::new(1);

fn epoch() -> u32 {
  EPOCH.fetch_add(1, Ordering::Relaxed)
}

// the person whose lines these are (all rows of a view share it)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Who {
  pub platform: Platform,
  pub login: Box<str>,
  pub nick: Box<str>,
  // twitch id (or kick / youtube id) the line already carried: the name wears its paint
  pub user_id: Box<str>,
}

// one request to the archive
#[derive(Clone, Debug)]
pub struct Req {
  pub epoch: u32,
  pub who: Who,
  pub channel: Option<Box<str>>,
  pub query: Option<Box<str>>,
  pub cursor: Option<Box<str>>,
}

impl Req {
  // GET /api/archive/user/<platform>/<login>/messages?channel=&q=&limit=&cursor=
  pub fn path(&self) -> String {
    let mut p =
      format!("/api/archive/user/{}/{}/messages?limit={PAGE}", self.who.platform.name(), path_seg(&self.who.login));
    if let Some(c) = &self.channel {
      p.push_str(&format!("&channel={}", path_seg(c)));
    }
    if let Some(q) = &self.query {
      p.push_str(&format!("&q={}", path_seg(q)));
    }
    if let Some(c) = &self.cursor {
      p.push_str(&format!("&cursor={}", path_seg(c)));
    }
    p
  }
}

// a line and the room it was said in (the permalink needs it; a channel-scoped view carries it once)
#[derive(Clone, Debug)]
pub struct Row {
  pub msg: Message,
  pub channel: Box<str>,
}

// one answer: lines newest first as the server sends them, where the older ones start, and whether the
// server began fetching history it did not have yet
#[derive(Debug, Default)]
pub struct Page {
  pub rows: Vec<Row>,
  pub cursor: Option<Box<str>>,
  pub backfill: bool,
}

// a room name as the archive keys it: no `#`, no `@`, lowercase, one bounded word
pub fn room(s: &str) -> Option<Box<str>> {
  let s = s.trim().trim_start_matches(['#', '@']).to_ascii_lowercase();
  let ok = !s.is_empty() && s.len() <= 50 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
  ok.then(|| s.into())
}

// the archive's own name for a person: twitch/kick login, a youtube `@handle`
pub fn archive_name(platform: Platform, login: &str) -> Option<Box<str>> {
  let l = login.trim().to_ascii_lowercase();
  let bare = l.trim_start_matches('@');
  let ok = !bare.is_empty()
    && bare.len() <= 40
    && bare.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.');
  match (ok, platform) {
    (false, _) => None,
    (true, Platform::Youtube) => Some(format!("@{bare}").into()),
    (true, _) => Some(bare.into()),
  }
}

// https://heatsync.org/search/logs/<platform>/<channel>/<yyyy-mm-dd utc>?m=<message id>
// (ext buildLogPermalink). the day is the line's own utc day, never today's: it only picks the page,
// the id picks the line. none without a room or a real time
pub fn permalink(platform: Platform, channel: &str, id: &str, ts: i64) -> Option<String> {
  let c = room(channel)?;
  if ts <= 0 {
    return None;
  }
  let mut url = format!("{PUBLIC}/search/logs/{}/{c}/{}", platform.name(), utc_day(ts));
  if !id.is_empty() {
    url.push_str(&format!("?m={}", path_seg(id)));
  }
  Some(url)
}

fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
  v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

// a cursor we will hand back: short and made of the characters the server writes (`2026-10-30|1`)
fn cursor_of(v: &Value) -> Option<Box<str>> {
  let c = text(v, "next_cursor")?;
  let ok = c.len() <= MAX_CURSOR && c.bytes().all(|b| b.is_ascii_alphanumeric() || b"|:._+-".contains(&b));
  ok.then(|| c.into())
}

// the time at the head of every row (utc: the archive's own days, and the permalink's)
fn stamp(ts: i64) -> Seg {
  Seg { text: format!("{} ", utc_stamp(ts)).into(), ink: theme::DIM, bold: false }
}

fn unescape(s: &str, out: &mut String) {
  let mut rest = s;
  while let Some(i) = rest.find('&') {
    out.push_str(&rest[..i]);
    let tail = &rest[i..];
    let hit = [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>'), ("&quot;", '"'), ("&#39;", '\''), ("&#x27;", '\'')]
      .iter()
      .find(|(e, _)| tail.starts_with(e));
    match hit {
      Some((e, c)) => {
        out.push(*c);
        rest = &tail[e.len()..];
      }
      None => {
        out.push('&');
        rest = &tail[1..];
      }
    }
  }
  out.push_str(rest);
}

// the value of `name="…"` inside one tag
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
  let key = format!(" {name}=\"");
  let from = tag.find(&key)? + key.len();
  let len = tag[from..].find('"')?;
  Some(&tag[from..from + len])
}

// where an emote image lives: heatsync's own `/e/<provider>/<id>`, or an allowlisted https url
fn emote_url(src: &str) -> Option<Box<str>> {
  if let Some(path) = src.strip_prefix("/e/") {
    let (prov, id) = path.split_once('/')?;
    let ok = ["7tv", "bttv", "ffz", "hs", "twitch", "kick"].contains(&prov)
      && (1..=64).contains(&id.len())
      && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    return ok.then(|| format!("{PUBLIC}/e/{prov}/{id}").into());
  }
  allowed(src).then(|| src.into())
}

// the server's own render of a line (`message_html`) knows every provider's emotes. read as data, never
// as markup: text is rebuilt, each `<img>` becomes a word in it, and the rebuilt text must equal the
// plain `message` — else no emote is trusted. overlays (zero-width) ride the by-word list
fn html_emotes(html: &str, message: &str) -> (Box<[Placed]>, Box<[HsEmote]>) {
  let none = || (Box::<[Placed]>::default(), Box::<[HsEmote]>::default());
  if html.len() > MAX_HTML || !html.contains("<img") {
    return none();
  }
  let mut plain = String::with_capacity(message.len());
  let (mut placed, mut over): (Vec<Placed>, Vec<HsEmote>) = (Vec::new(), Vec::new());
  let mut rest = html;
  while let Some(i) = rest.find('<') {
    unescape(&rest[..i], &mut plain);
    let tail = &rest[i..];
    let Some(end) = tail.find('>') else { return none() };
    let tag = &tail[..end];
    rest = &tail[end + 1..];
    if !(tag.starts_with("<img ") || tag.starts_with("<img\t")) {
      continue;
    }
    let (Some(src), Some(alt)) = (attr(tag, "src"), attr(tag, "alt")) else { return none() };
    let mut word = String::new();
    unescape(alt, &mut word);
    // a heatsync inventory emote is not the render's to hand us: the word stays plain text and draws
    // only if the sender's own set has it (the one gate, ui::chat_view::resolve)
    let url = emote_url(src).filter(|u| !inventory_url(u));
    let start = plain.len() as u32;
    plain.push_str(&word);
    let Some(url) = url.filter(|_| valid_name(&word) && placed.len() + over.len() < MAX_EMOTES) else { continue };
    let class = attr(tag, "class").unwrap_or("");
    if class.split_whitespace().any(|c| c == "zw" || c == "zwo") {
      over.push((word.into(), url, true));
    } else {
      placed.push((start..plain.len() as u32, url));
    }
  }
  unescape(rest, &mut plain);
  if plain != message {
    return none();
  }
  (placed.into(), over.into())
}

// one archive row as a chat line. None = not a line we can show (no words, no real time)
pub fn parse_row(v: &Value, who: &Who) -> Option<Row> {
  let raw = text(v, "message")?;
  let body = clamp(raw, MAX_TEXT);
  let ts = text(v, "timestamp").and_then(parse_iso).filter(|t| *t > 0)?;
  let nick = text(v, "display_name").map(|n| clamp(n, MAX_NAME)).unwrap_or(&who.nick);
  let colour = text(v, "color").or_else(|| text(v, "user_color")).and_then(color::parse_hex);
  // the render's emote positions only mean something against the whole, unclipped text
  let (emotes, hs) = match text(v, "message_html") {
    Some(h) if body.len() == raw.len() => html_emotes(h, body),
    _ => Default::default(),
  };
  let badges: Box<[(Box<str>, Box<str>)]> = match who.platform {
    Platform::Twitch => v
      .get("badges")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .take(MAX_BADGES)
      .filter_map(|b| Some((clamp(text(b, "name")?, 64).into(), clamp(text(b, "version")?, 64).into())))
      .collect(),
    _ => Box::default(),
  };
  let channel = text(v, "channel").and_then(room).unwrap_or_default();
  let mut flags = 0;
  if v.get("deleted_at").is_some_and(|d| !d.is_null()) {
    flags |= flag::CLEARED;
  }
  let msg = Message {
    id: clamp(text(v, "message_id").unwrap_or(""), 100).into(),
    ts,
    platform: who.platform,
    kind: Kind::Chat,
    flags,
    login: who.login.clone(),
    user_id: who.user_id.clone(),
    nick: nick.into(),
    color: colour.unwrap_or_else(|| color::default_for(&who.login)),
    body: body.into(),
    emotes,
    hs,
    badges,
    reply_to: None,
    reply_text: None,
    system: None,
    x: Some(Box::new(Extra { head: Box::new([stamp(ts)]), ..Extra::default() })),
  };
  Some(Row { msg, channel })
}

// a whole answer. rows the server sent beyond one page are not read
pub fn parse_page(v: &Value, who: &Who) -> Page {
  let rows = v
    .get("results")
    .and_then(Value::as_array)
    .map(|a| a.iter().take(PAGE).filter_map(|r| parse_row(r, who)).collect())
    .unwrap_or_default();
  Page { rows, cursor: cursor_of(v), backfill: v.get("backfill_pending").and_then(Value::as_bool) == Some(true) }
}

// what a landed answer asks of the app
#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
  // an answer for a view since changed or closed
  Stale,
  // show it
  Shown,
  // show it, and the next page is wanted right away (a page of nothing, more behind it)
  More,
  // it failed: the status (0 = no answer)
  Failed(u16),
}

pub struct Logs {
  pub who: Who,
  // the channel it was opened from; the scope toggle's other side. None = opened with no room
  pub home: Option<Box<str>>,
  pub all: bool,
  // the search in effect (`/`), None = the timeline
  pub query: Option<Box<str>>,
  // oldest first
  rows: Vec<Row>,
  cursor: Option<Box<str>>,
  pub loading: bool,
  // nothing older exists (or will be asked for)
  pub done: bool,
  // the oldest lines were dropped at the cap: the cursor no longer joins up with what is left
  pub trimmed: bool,
  pub failed: bool,
  pub backfill: bool,
  epoch: u32,
  hops: u8,
}

impl Logs {
  pub fn new(who: Who, home: Option<Box<str>>) -> Self {
    let all = home.is_none();
    Logs {
      who,
      home,
      all,
      query: None,
      rows: Vec::new(),
      cursor: None,
      loading: false,
      done: false,
      trimmed: false,
      failed: false,
      backfill: false,
      epoch: epoch(),
      hops: 0,
    }
  }

  // the room the archive is asked about: None = every channel
  pub fn channel(&self) -> Option<&str> {
    if self.all { None } else { self.home.as_deref() }
  }

  #[cfg(test)]
  pub fn len(&self) -> usize {
    self.rows.len()
  }

  #[cfg(test)]
  pub fn is_empty(&self) -> bool {
    self.rows.is_empty()
  }

  // start over: a new scope or search. answers still on the way are dropped
  fn reset(&mut self) {
    self.rows.clear();
    self.cursor = None;
    self.loading = false;
    self.done = false;
    self.trimmed = false;
    self.failed = false;
    self.backfill = false;
    self.hops = 0;
    self.epoch = epoch();
  }

  pub fn refresh(&mut self) {
    self.reset();
  }

  // this channel ↔ all channels. false when it was opened with no channel (all is all there is)
  pub fn toggle_scope(&mut self) -> bool {
    if self.home.is_none() {
      return false;
    }
    self.all = !self.all;
    self.reset();
    true
  }

  // the timeline (None / empty) or a search within this person's lines
  pub fn set_query(&mut self, q: &str) {
    let q = clamp(q.trim(), MAX_QUERY);
    self.query = (!q.is_empty()).then(|| q.into());
    self.reset();
  }

  // the next page to ask for, marking it in flight. None = one is already out, or nothing is left
  pub fn next(&mut self) -> Option<Req> {
    if self.loading || self.done || self.trimmed {
      return None;
    }
    self.loading = true;
    self.failed = false;
    Some(Req {
      epoch: self.epoch,
      who: self.who.clone(),
      channel: self.channel().map(Into::into),
      query: self.query.clone(),
      cursor: self.cursor.clone(),
    })
  }

  // an answer. a failure never ends the archive (the next ask retries); a 404 / 410 does
  pub fn apply(&mut self, epoch: u32, res: Result<Page, u16>) -> Applied {
    if epoch != self.epoch {
      return Applied::Stale;
    }
    self.loading = false;
    let page = match res {
      Ok(p) => p,
      Err(status) => {
        self.failed = true;
        self.done = matches!(status, 404 | 410);
        return Applied::Failed(status);
      }
    };
    self.failed = false;
    self.backfill |= page.backfill;
    let empty = page.rows.is_empty();
    // a cursor that does not move would ask the same page forever
    self.done = page.cursor.is_none() || page.cursor == self.cursor;
    self.cursor = page.cursor;
    // older lines go in front; at the cap the newest are let go (the reader is looking up, and the
    // cursor stays joined to what is kept)
    let mut older: Vec<Row> = page.rows;
    older.reverse();
    older.append(&mut self.rows);
    older.truncate(CAP);
    self.rows = older;
    if empty && !self.done && self.hops < HOPS {
      self.hops += 1;
      return Applied::More;
    }
    if !empty {
      self.hops = 0;
    }
    Applied::Shown
  }

  // a line the person says while it is open, at the bottom. false = not for this view (a search is
  // showing results, another room, or already here)
  pub fn push_live(&mut self, channel: &str, mut msg: Message) -> Option<Row> {
    if self.query.is_some() || !msg.kind.said() || msg.body.is_empty() || msg.ts <= 0 {
      return None;
    }
    let channel = room(channel)?;
    if self.channel().is_some_and(|c| c != &*channel) {
      return None;
    }
    if !msg.id.is_empty() && self.rows.iter().rev().take(40).any(|r| r.msg.id == msg.id) {
      return None;
    }
    msg.flags &= !(flag::HISTORY | flag::ALERT | flag::NEW);
    msg.user_id = self.who.user_id.clone();
    let x = msg.x.get_or_insert_with(Default::default);
    x.head = Box::new([stamp(msg.ts)]);
    let row = Row { msg, channel };
    self.rows.push(row.clone());
    if self.rows.len() > CAP {
      self.rows.remove(0);
      // what the cursor pointed past is gone from the top: older lines would leave a gap
      self.trimmed = true;
    }
    Some(row)
  }

  // a row as the view draws it: all-channels rows name their room
  pub fn view_msg(&self, r: &Row) -> Message {
    let mut m = r.msg.clone();
    if self.all {
      m.x.get_or_insert_with(Default::default).chan = r.channel.clone();
    }
    m
  }

  // the line at the top: what is (not) above the oldest one shown
  fn top_note(&self) -> Option<&'static str> {
    if self.rows.is_empty() {
      return None;
    }
    Some(if self.loading {
      "loading…"
    } else if self.failed {
      "couldn't load older lines — press r to retry"
    } else if self.trimmed {
      "older lines dropped to save memory — press r to reload"
    } else if self.done {
      "no older messages"
    } else {
      "scroll up for older"
    })
  }

  // everything the view shows, oldest first
  pub fn lines(&self) -> Vec<Message> {
    let mut v = Vec::with_capacity(self.rows.len() + 1);
    if let Some(n) = self.top_note() {
      v.push(Message::system(0, self.who.platform, n));
    }
    v.extend(self.rows.iter().map(|r| self.view_msg(r)));
    v
  }

  // what an empty view says
  pub fn empty_text(&self) -> String {
    let room = self.channel().map(|c| format!(" in #{c}")).unwrap_or_default();
    if self.loading {
      "loading…".into()
    } else if self.failed && !self.done {
      "couldn't load the archive — press r to try again".into()
    } else if self.backfill {
      "fetching older logs… press r in half a minute".into()
    } else if let Some(q) = &self.query {
      format!("no matches for \"{q}\"{room}")
    } else {
      format!("no logged messages from {}{room}", self.who.nick)
    }
  }

  // the public link for a line the view shows (its room: the row's own, else the view's)
  pub fn link_of(&self, m: &Message) -> Option<String> {
    let row = self.rows.iter().rev().find(|r| r.msg.ts == m.ts && r.msg.id == m.id && r.msg.body == m.body)?;
    let channel = if row.channel.is_empty() { self.home.as_deref()? } else { &row.channel };
    permalink(m.platform, channel, &m.id, m.ts)
  }

  // the line under the header: who, where, and the keys that matter
  pub fn hint(&self) -> String {
    let scope = match (self.all, &self.home) {
      (false, Some(c)) => format!("#{c}"),
      _ => "all channels".into(),
    };
    let what = self.query.as_ref().map_or_else(String::new, |q| format!(" · search \"{q}\""));
    let swap = if self.home.is_some() { " · a swap scope" } else { "" };
    format!("{} · {scope}{what} · / search{swap} · y copy link · esc back", self.who.nick)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::util::time::parse_iso;
  use serde_json::json;

  fn who() -> Who {
    Who { platform: Platform::Twitch, login: "bob".into(), nick: "Bob".into(), user_id: "42".into() }
  }

  fn row(n: i64) -> Value {
    json!({"message": format!("line {n}"), "message_id": format!("id{n}"), "channel": "xqc",
      "timestamp": format!("2026-10-30T20:{:02}:00.000Z", n % 60), "color": "#0000FF", "display_name": "Bob"})
  }

  fn page(from: i64, n: i64, cursor: Option<&str>) -> Page {
    // newest first, like the server
    let rows = (from..from + n).rev().filter_map(|i| parse_row(&row(i), &who())).collect();
    Page { rows, cursor: cursor.map(Into::into), backfill: false }
  }

  #[test]
  fn row_reads_like_a_chat_line() {
    let r = parse_row(&row(5), &who()).unwrap();
    assert_eq!((&*r.msg.body, &*r.msg.id, &*r.channel, &*r.msg.login), ("line 5", "id5", "xqc", "bob"));
    assert_eq!((r.msg.color, &*r.msg.user_id, r.msg.kind), (0x0000ff, "42", Kind::Chat));
    assert_eq!(r.msg.x.unwrap().head[0].text.as_ref(), "10-30 20:05:00 ", "the time leads every row, utc");
  }

  #[test]
  fn hostile_rows_are_dropped_or_bounded() {
    let w = who();
    for bad in [
      json!(null),
      json!(5),
      json!({}),
      json!({"message": "x"}),
      json!({"message": 5, "timestamp": "x"}),
      json!({"message": "", "timestamp": "2026-10-30T20:00:00Z"}),
      json!({"message": "x", "timestamp": "nope"}),
      json!({"message": "x", "timestamp": "1970-01-01T00:00:00Z"}),
    ] {
      assert!(parse_row(&bad, &w).is_none(), "{bad}");
    }
    let huge = "é".repeat(10_000);
    let r = parse_row(&json!({"message": huge, "timestamp": "2026-10-30T20:00:00Z", "display_name": huge,
      "message_id": huge, "channel": huge, "color": "javascript:1", "badges": vec![json!({"name": "a", "version": "1"}); 500]}), &w)
    .unwrap();
    assert!(r.msg.body.len() <= MAX_TEXT && r.msg.nick.len() <= MAX_NAME && r.msg.id.len() <= 100);
    assert!(r.channel.is_empty(), "a room that is no room name is none");
    assert_eq!(r.msg.color, color::default_for("bob"), "a bad color is the default one");
    assert_eq!(r.msg.badges.len(), MAX_BADGES);
    let many: Vec<Value> = (0..5000).map(row).collect();
    assert_eq!(parse_page(&json!({"results": many}), &w).rows.len(), PAGE, "rows past a page are not read");
    for v in [json!(null), json!([]), json!({"results": 5}), json!({"results": {}})] {
      assert!(parse_page(&v, &w).rows.is_empty());
    }
  }

  #[test]
  fn cursor_is_the_servers_shape_or_nothing() {
    let p = |c: Value| parse_page(&json!({"results": [], "next_cursor": c}), &who()).cursor;
    assert_eq!(p(json!("2026-10-30|1")).as_deref(), Some("2026-10-30|1"));
    for bad in
      [json!("a b"), json!("x".repeat(500)), json!("../../etc"), json!("a&b=c"), json!(5), json!(null), json!("")]
    {
      assert!(p(bad.clone()).is_none(), "{bad}");
    }
  }

  #[test]
  fn badges_only_on_twitch_and_deleted_dims() {
    let v = json!({"message": "x", "timestamp": "2026-10-30T20:00:00Z", "badges": [{"name": "subscriber", "version": "3"}, {"name": 5}],
      "deleted_at": "2026-10-30T20:01:00Z"});
    let r = parse_row(&v, &who()).unwrap();
    assert_eq!(r.msg.badges.len(), 1);
    assert!(r.msg.has(flag::CLEARED));
    let k = Who { platform: Platform::Kick, ..who() };
    assert!(parse_row(&v, &k).unwrap().msg.badges.is_empty());
  }

  #[test]
  fn emotes_come_from_the_servers_render_and_only_when_it_matches_the_text() {
    let html = "hi <img class=\"hs-archive-emote\" src=\"/e/7tv/01ABC\" alt=\"LUL\" title=\"LUL\"> &amp; <img class=\"hs-archive-emote zw\" src=\"/e/7tv/zz-1\" alt=\"wide\"> x";
    let msg = "hi LUL & wide x";
    let (placed, over) = html_emotes(html, msg);
    assert_eq!(placed.len(), 1);
    assert_eq!(&msg[placed[0].0.start as usize..placed[0].0.end as usize], "LUL");
    assert_eq!(&*placed[0].1, "https://heatsync.org/e/7tv/01ABC");
    assert_eq!((&*over[0].0, over[0].2), ("wide", true), "an overlay is a by-word zero-width emote");
    // the render does not spell the message: nothing in it is trusted
    let (p, o) = html_emotes(html, "something else entirely");
    assert!(p.is_empty() && o.is_empty());
    // hostile sources and shapes
    for src in [
      "/e/evil/1",
      "/e/7tv/../x",
      "/e/7tv/",
      "//evil.example/x",
      "http://static-cdn.jtvnw.net/a",
      "https://evil.example/e.png",
      "javascript:1",
    ] {
      let h = format!("<img src=\"{src}\" alt=\"A\">");
      let (p, o) = html_emotes(&h, "A");
      assert!(p.is_empty() && o.is_empty(), "{src}");
    }
    let ok = "<img src=\"https://static-cdn.jtvnw.net/emoticons/v2/9/default/dark/1.0\" alt=\"Kappa\">";
    assert_eq!(html_emotes(ok, "Kappa").0.len(), 1, "an allowlisted https image is fine");
    assert!(html_emotes("<img src=\"/e/7tv/1\" alt=\"A\"", "A").0.is_empty(), "an unclosed tag");
    assert!(
      html_emotes(&"<img src=\"/e/7tv/1\" alt=\"A\">".repeat(2000), "A").0.is_empty(),
      "a huge render is not read"
    );
    let many = "<img src=\"/e/7tv/1\" alt=\"A\">".repeat(80);
    let (p, _) = html_emotes(&many, &"A".repeat(80));
    assert_eq!(p.len(), MAX_EMOTES, "bounded");
  }

  // privacy invariant: a heatsync emote draws iff the SENDER's own set has it. the render's `/e/hs/…`
  // images (and uploads, the cdn) are therefore plain words here, zero-width ones too; the provider
  // mirrors stay the channel's emotes
  #[test]
  fn the_renders_heatsync_inventory_emotes_are_not_trusted() {
    let html = "<img src=\"/e/hs/abc\" alt=\"kek\"> <img class=\"zw\" src=\"/e/hs/zz\" alt=\"wide\"> \
      <img src=\"https://cdn.heatsync.org/uploads/a.webp\" alt=\"up\"> \
      <img src=\"https://heatsync.org/uploads/b.webp\" alt=\"up2\"> <img src=\"/e/ffz/7\" alt=\"LUL\">";
    let (placed, over) = html_emotes(html, "kek wide up up2 LUL");
    assert!(over.is_empty(), "no zero-width inventory overlay");
    assert_eq!(placed.len(), 1, "only the provider mirror");
    assert_eq!(&*placed[0].1, "https://heatsync.org/e/ffz/7");
    let v = json!({"message": "kek wide", "timestamp": "2026-10-30T20:00:00Z",
      "message_html": "<img src=\"/e/hs/abc\" alt=\"kek\"> <img class=\"zw\" src=\"/e/hs/zz\" alt=\"wide\">"});
    let m = parse_row(&v, &who()).unwrap().msg;
    assert_eq!(&*m.body, "kek wide");
    assert!(m.emotes.is_empty() && m.hs.is_empty(), "the words are left for the sender's own set to claim");
  }

  #[test]
  fn emotes_land_on_the_row() {
    let v = json!({"message": "yo LUL", "timestamp": "2026-10-30T20:00:00Z",
      "message_html": "yo <img src=\"/e/7tv/9\" alt=\"LUL\">"});
    let r = parse_row(&v, &who()).unwrap();
    assert_eq!(r.msg.emotes.len(), 1);
    assert_eq!(&r.msg.body[r.msg.emotes[0].0.start as usize..], "LUL");
  }

  #[test]
  fn permalink_uses_the_utc_day_of_the_line_across_midnight() {
    let late = parse_iso("2026-10-31T23:59:59.900Z").unwrap();
    let url = |ts| permalink(Platform::Twitch, "#XQC", "a b", ts).unwrap();
    assert_eq!(url(late), "https://heatsync.org/search/logs/twitch/xqc/2026-10-31?m=a%20b");
    assert_eq!(url(late + 100), "https://heatsync.org/search/logs/twitch/xqc/2026-11-01?m=a%20b");
    assert_eq!(permalink(Platform::Kick, "x", "", late).unwrap(), "https://heatsync.org/search/logs/kick/x/2026-10-31");
    assert!(permalink(Platform::Twitch, "", "i", late).is_none(), "no room, no link");
    assert!(permalink(Platform::Twitch, "bad room", "i", late).is_none());
    assert!(permalink(Platform::Twitch, "x", "i", 0).is_none(), "no time, no link");
  }

  #[test]
  fn request_paths_are_encoded_and_bounded() {
    let mut r = Req { epoch: 1, who: who(), channel: None, query: None, cursor: None };
    assert_eq!(r.path(), "/api/archive/user/twitch/bob/messages?limit=100");
    r.channel = Some("xqc".into());
    r.query = Some("a&b c#".into());
    r.cursor = Some("2026-10-30|1".into());
    assert_eq!(
      r.path(),
      "/api/archive/user/twitch/bob/messages?limit=100&channel=xqc&q=a%26b%20c%23&cursor=2026-10-30%7C1"
    );
    r.who.login = "@we ird/../".into();
    assert!(r.path().contains("/%40we%20ird%2F..%2F/messages"));
  }

  #[test]
  fn names_and_rooms_as_the_archive_keys_them() {
    assert_eq!(room("#XQC").as_deref(), Some("xqc"));
    assert_eq!(room("@kick-slug_1").as_deref(), Some("kick-slug_1"));
    for bad in ["", "#", "a b", "a/b", "é", &"x".repeat(51)] {
      assert!(room(bad).is_none(), "{bad}");
    }
    assert_eq!(archive_name(Platform::Twitch, "Bob_1").as_deref(), Some("bob_1"));
    assert_eq!(archive_name(Platform::Youtube, "handle").as_deref(), Some("@handle"));
    assert_eq!(archive_name(Platform::Youtube, "@handle").as_deref(), Some("@handle"));
    assert!(archive_name(Platform::Twitch, "a b").is_none() && archive_name(Platform::Kick, "").is_none());
  }

  #[test]
  fn pages_prepend_older_lines_and_stop_when_the_cursor_ends() {
    let mut l = Logs::new(who(), Some("xqc".into()));
    let r = l.next().unwrap();
    assert!(l.next().is_none(), "one request at a time");
    assert_eq!((r.channel.as_deref(), r.cursor.as_deref()), (Some("xqc"), None));
    assert_eq!(l.apply(r.epoch, Ok(page(10, 5, Some("c1")))), Applied::Shown);
    let r2 = l.next().unwrap();
    assert_eq!(r2.cursor.as_deref(), Some("c1"), "the next page is the one older than the last");
    assert_eq!(l.apply(r2.epoch, Ok(page(0, 5, None))), Applied::Shown);
    assert!(l.done && l.next().is_none(), "no cursor: exhausted");
    let bodies: Vec<_> = l.lines().iter().map(|m| m.body.to_string()).collect();
    assert_eq!(bodies[0], "no older messages");
    assert_eq!(&bodies[1..4], ["line 0", "line 1", "line 2"], "oldest first, newest at the bottom");
    assert_eq!(bodies.last().unwrap(), "line 14");
    assert_eq!(l.len(), 10);
  }

  #[test]
  fn a_failure_retries_and_a_404_ends() {
    let mut l = Logs::new(who(), None);
    let r = l.next().unwrap();
    assert_eq!(l.apply(r.epoch, Err(503)), Applied::Failed(503));
    assert!(l.failed && !l.done, "a blip never ends the archive");
    assert!(l.empty_text().contains("press r"));
    let r = l.next().unwrap();
    assert_eq!(l.apply(r.epoch, Err(404)), Applied::Failed(404));
    assert!(l.done && l.next().is_none());
    assert_eq!(l.empty_text(), "no logged messages from Bob");
  }

  #[test]
  fn late_answers_for_another_scope_or_search_are_dropped() {
    let mut l = Logs::new(who(), Some("xqc".into()));
    let old = l.next().unwrap();
    assert!(l.toggle_scope() && l.all && l.channel().is_none());
    assert_eq!(l.apply(old.epoch, Ok(page(0, 3, None))), Applied::Stale);
    assert!(l.is_empty() && !l.loading);
    let r = l.next().unwrap();
    assert!(r.channel.is_none(), "all channels: no channel filter");
    l.set_query("  lol ");
    assert_eq!(l.apply(r.epoch, Ok(page(0, 3, None))), Applied::Stale);
    let r = l.next().unwrap();
    assert_eq!(r.query.as_deref(), Some("lol"));
    assert!(!l.empty_text().is_empty());
    l.set_query("   ");
    assert!(l.query.is_none(), "an empty search is the timeline");
  }

  #[test]
  fn scope_toggle_needs_a_home_channel() {
    let mut l = Logs::new(who(), None);
    assert!(l.all && !l.toggle_scope() && l.all);
    let mut l = Logs::new(who(), Some("xqc".into()));
    assert!(!l.all && l.channel() == Some("xqc"));
    assert!(l.toggle_scope() && l.all);
    assert!(l.toggle_scope() && !l.all && l.channel() == Some("xqc"));
    assert!(l.hint().contains("#xqc") && l.hint().contains("a swap scope"));
  }

  #[test]
  fn the_cap_drops_the_newest_when_reading_up_and_the_oldest_when_live() {
    let mut l = Logs::new(who(), None);
    let r = l.next().unwrap();
    let full: Vec<Row> = (0..CAP as i64 - 2).rev().filter_map(|i| parse_row(&row(i), &who())).collect();
    l.apply(r.epoch, Ok(Page { rows: full, cursor: Some("c1".into()), backfill: false }));
    let r = l.next().unwrap();
    l.apply(r.epoch, Ok(page(5000, 50, Some("c2"))));
    assert_eq!(l.len(), CAP, "an older page past the cap is cut to fit");
    assert_eq!(&*l.rows[0].msg.id, "id5000", "what was loaded older stays");
    assert!(!l.trimmed && l.next().is_some(), "the cursor still joins up");
    l.loading = false;
    let live = |n: i64| parse_row(&row(n), &who()).unwrap().msg;
    assert!(l.push_live("xqc", live(1)).is_some());
    assert_eq!(l.len(), CAP);
    assert!(l.trimmed, "the oldest went: older pages would leave a gap");
    assert!(l.next().is_none());
    assert!(l.lines()[0].body.contains("press r"));
  }

  #[test]
  fn sparse_pages_hop_a_few_times_then_wait() {
    let mut l = Logs::new(who(), None);
    l.set_query("rare");
    for i in 0..HOPS {
      let r = l.next().unwrap();
      let c = format!("c{i}");
      assert_eq!(
        l.apply(r.epoch, Ok(Page { rows: vec![], cursor: Some(c.as_str().into()), backfill: false })),
        Applied::More
      );
    }
    let r = l.next().unwrap();
    assert_eq!(l.apply(r.epoch, Ok(Page { rows: vec![], cursor: Some("cz".into()), backfill: false })), Applied::Shown);
    assert!(!l.done, "still more behind it: the next scroll asks again");
    // a cursor that does not move would loop
    let r = l.next().unwrap();
    assert_eq!(l.apply(r.epoch, Ok(Page { rows: vec![], cursor: Some("cz".into()), backfill: false })), Applied::Shown);
    assert!(l.done);
  }

  #[test]
  fn live_lines_follow_the_scope_and_never_double() {
    let mut l = Logs::new(who(), Some("xqc".into()));
    let msg = |id: &str| {
      let mut m = parse_row(&row(3), &who()).unwrap().msg;
      m.id = id.into();
      m
    };
    assert!(l.push_live("other", msg("a")).is_none(), "another room is not this view");
    assert!(l.push_live("XQC", msg("a")).is_some());
    assert!(l.push_live("xqc", msg("a")).is_none(), "a line already shown");
    l.toggle_scope();
    assert!(l.push_live("other", msg("b")).is_some(), "all channels hears every room");
    let shown = l.lines();
    assert_eq!(&*shown.last().unwrap().x.as_ref().unwrap().chan, "other", "and names the room");
    l.set_query("x");
    assert!(l.push_live("other", msg("c")).is_none(), "search results are not a timeline");
    let mut sys = msg("d");
    sys.kind = Kind::System;
    l.set_query("");
    assert!(l.push_live("x", sys).is_none());
  }

  #[test]
  fn links_resolve_for_shown_rows_in_either_scope() {
    let mut l = Logs::new(who(), Some("home".into()));
    let r = l.next().unwrap();
    l.apply(r.epoch, Ok(page(1, 2, None)));
    let m = l.lines().pop().unwrap();
    assert_eq!(l.link_of(&m).unwrap(), "https://heatsync.org/search/logs/twitch/xqc/2026-10-30?m=id2");
    assert!(l.link_of(&Message::system(0, Platform::Twitch, "no older messages")).is_none());
  }

  #[test]
  fn empty_texts_say_where_and_what() {
    let mut l = Logs::new(who(), Some("xqc".into()));
    assert_eq!(l.empty_text(), "no logged messages from Bob in #xqc");
    l.set_query("lol");
    assert_eq!(l.empty_text(), "no matches for \"lol\" in #xqc");
    l.backfill = true;
    assert!(l.empty_text().starts_with("fetching"));
    l.next();
    assert_eq!(l.empty_text(), "loading…");
  }
}
