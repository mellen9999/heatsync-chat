// heatsync feed posts (ext social.js buildFeedMessageDiv): `>>id [OP] [T] name  heat° ▲ ↩n: text`.
// pure: parsing the server's json, the heat ladder, the row a post becomes. the server escapes
// post text on store, so it is unescaped here and drawn as plain text
use serde_json::Value;

use super::color;
use super::markup::{self, Mark};
use super::message::{Extra, HsEmote, Kind, Message, Pic, Platform, Seg};
use super::reaction::{self, Reaction};
use crate::cosmetics::id::PaintId;
use crate::heatsync::content::{self, emote_hidden};
use crate::heatsync::valid_name;
use crate::images::url::allowed;
use crate::ui::theme;
use crate::util::time::{now_ms, parse_iso};

const MAX_TEXT: usize = 4000;
const MAX_NAME: usize = 100;
const MAX_REFS: usize = 50;
const MAX_MEDIA: usize = 8;
const SITE: &str = "https://heatsync.org";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
  Image,
  Gif,
  Video,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Media {
  pub url: Box<str>,
  pub kind: MediaKind,
  // a video's still (the picture a video row draws)
  pub thumb: Option<Box<str>>,
}

impl Media {
  // what draws: the picture itself, a video's still
  pub fn pic(&self) -> Option<&str> {
    match self.kind {
      MediaKind::Video => self.thumb.as_deref(),
      _ => Some(&self.url),
    }
  }
}

#[derive(Clone, Debug)]
pub struct Post {
  // base36, zero padded (the server's id)
  pub id: Box<str>,
  pub user: Box<str>,
  pub color: u32,
  pub anon: bool,
  pub platform: Option<Platform>,
  // whose paint the author's name wears (their id in their own platform's namespace), if the post names one
  pub paint: Option<PaintId>,
  pub text: Box<str>,
  pub ts: i64,
  pub heat: u32,
  pub replies: u32,
  pub reply_to: Option<Box<str>>,
  pub is_op: bool,
  // the thread's author replying in their own thread (ext [OP] in magenta)
  pub thread_op: bool,
  pub voted: bool,
  pub edited: bool,
  // the AUTHOR's own heatsync emotes, server-resolved: the only heatsync emote source for a post
  pub refs: Box<[HsEmote]>,
  pub media: Box<[Media]>,
  // what moderation flagged the media for (held unless the viewer's switch is on, content::held)
  pub cats: Box<[Box<str>]>,
  pub media_count: u8,
  // the server kept the files back (flagged, viewer not cleared): `show` asks /api/messages/:id/media
  pub withheld: bool,
  // still in moderation: tagged, never peeked
  pub pending: bool,
  // video when the server says so; a withheld post's kind is all we know of it
  pub video: bool,
  // the chips under it (model::reaction); none for most posts, and a boxed empty slice holds nothing
  pub reactions: Box<[Reaction]>,
}

// where the viewer's `show` on a held post stands (model::feed keeps one per post, for the session)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reveal {
  Open,
  Asking,
  // 403: log in (as an adult) to see it
  Denied,
  // logged in, and this account isn't cleared for it
  Unavailable,
  Failed,
}

fn clamp(s: &str, max: usize) -> String {
  s.chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).take(max).collect()
}

// &amp; &lt; &gt; &quot; &#39; &#x27;, in one pass so "&amp;lt;" stays "&lt;"
pub fn unescape(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut rest = s;
  while let Some(i) = rest.find('&') {
    out.push_str(&rest[..i]);
    rest = &rest[i..];
    let hit = [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>'), ("&quot;", '"'), ("&#39;", '\''), ("&#x27;", '\'')]
      .into_iter()
      .find(|(e, _)| rest.starts_with(e));
    match hit {
      Some((e, c)) => {
        out.push(c);
        rest = &rest[e.len()..];
      }
      None => {
        out.push('&');
        rest = &rest[1..];
      }
    }
  }
  out.push_str(rest);
  out
}

pub fn id_ok(s: &str) -> bool {
  (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

// { name: url } or { name: { url, cw, nsfw, cw_cats } }; a gated or non-https ref draws as its name
pub fn refs_of(v: Option<&Value>) -> Box<[HsEmote]> {
  let Some(map) = v.and_then(Value::as_object) else { return Box::new([]) };
  map
    .iter()
    .filter_map(|(word, e)| {
      let url = e.as_str().or_else(|| e.get("url").and_then(Value::as_str))?;
      let gated =
        e.is_object() && (emote_hidden(e) || e.get("cw").is_some_and(|c| c.as_str().is_some_and(|s| !s.is_empty())));
      let zero = e.get("zeroWidth").and_then(Value::as_bool) == Some(true);
      (valid_name(word) && !gated && allowed(url)).then(|| (word.as_str().into(), url.into(), zero))
    })
    .take(MAX_REFS)
    .collect()
}

// `/uploads/x.webp` → the site's own url; anything else must already be an allowed https url
fn media_url(v: Option<&Value>) -> Option<Box<str>> {
  let u = v?.as_str()?;
  let u = if u.starts_with("/uploads/") { format!("{SITE}{u}") } else { u.to_owned() };
  allowed(&u).then(|| u.into())
}

fn kind_of(ty: Option<&str>, url: &str) -> MediaKind {
  let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
  if ty == Some("video") {
    MediaKind::Video
  } else if path.ends_with(".gif") || path.ends_with(".apng") {
    MediaKind::Gif
  } else {
    MediaKind::Image
  }
}

// `media[]` ({type, url, thumbnail_url}), else the single `media_url` + `media_type`. a post's own
// fields or the answer to /api/messages/:id/media, same shape
pub fn media_of(v: &Value) -> Box<[Media]> {
  let one = |url: Option<&Value>, ty: Option<&Value>, thumb: Option<&Value>| {
    let url = media_url(url)?;
    let kind = kind_of(ty.and_then(Value::as_str), &url);
    Some(Media { url, kind, thumb: media_url(thumb) })
  };
  let list = v.get("media").and_then(Value::as_array).filter(|a| !a.is_empty());
  match list {
    Some(a) => {
      a.iter().take(MAX_MEDIA).filter_map(|m| one(m.get("url"), m.get("type"), m.get("thumbnail_url"))).collect()
    }
    None => one(v.get("media_url"), v.get("media_type"), v.get("thumbnail_url")).into_iter().collect(),
  }
}

// `media_held` (what the server kept back), else what moderation flagged
fn cats_of(v: &Value) -> Box<[Box<str>]> {
  let held = v.get("media_held").and_then(Value::as_array).filter(|a| !a.is_empty());
  let flagged = || v.get("moderation_result").and_then(|r| r.get("flaggedCategories")).and_then(Value::as_array);
  held
    .or_else(flagged)
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .filter(|c| !c.is_empty())
    .take(MAX_MEDIA)
    .map(|c| clamp(c, 32).into())
    .collect()
}

// a post's words as the server stored them (escaped, any bytes), as a row shows them
pub fn words(raw: &str) -> Box<str> {
  clamp(&unescape(raw), MAX_TEXT).trim().into()
}

// one post from /api/messages, /api/thread or a `new-message` frame
pub fn parse(v: &Value) -> Option<Post> {
  let id = v.get("base36_id").and_then(Value::as_str).filter(|s| id_ok(s))?;
  let text = v.get("content").and_then(Value::as_str)?;
  let name = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
  let user = name("username").or_else(|| name("display_name")).unwrap_or("anon");
  let anon = user == "Anonymous" || v.get("user_id").is_none_or(Value::is_null);
  let reply_to = name("reply_to").filter(|s| id_ok(s)).map(Into::into);
  let platform = match name("platform") {
    Some("twitch") => Some(Platform::Twitch),
    Some("kick") => Some(Platform::Kick),
    Some("youtube") => Some(Platform::Youtube),
    _ => None,
  };
  let num =
    |k: &str| v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0).map_or(0, |n| n.min(1e9) as u32);
  // ext isPaintLookupSafeId: the author's id in their own platform's namespace; the server already
  // writes kick/youtube accounts as `kick_<n>` / `yt_UC…`. anything unsure stays plain
  let uid = match v.get("user_id") {
    Some(Value::String(s)) => s.clone(),
    Some(Value::Number(n)) => n.to_string(),
    _ => String::new(),
  };
  let paint = PaintId::of(name("platform").unwrap_or(""), &uid);
  let media = media_of(v);
  // a count comes as a number or a numeric string
  let count = match v.get("media_count") {
    Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
    Some(Value::String(s)) => s.parse().unwrap_or(0),
    _ => 0,
  };
  let has = v.get("has_media").and_then(Value::as_bool) == Some(true) || count > 0;
  let media_count = (count as usize).max(media.len()).min(MAX_MEDIA * 4) as u8;
  Some(Post {
    id: id.into(),
    user: clamp(user, MAX_NAME).into(),
    color: name("user_color").and_then(color::parse_hex).map_or(theme::WHITE, color::readable),
    anon,
    platform,
    paint,
    text: words(text),
    ts: name("created_at").and_then(parse_iso).unwrap_or_else(now_ms),
    heat: num("heat"),
    replies: num("reply_count"),
    thread_op: v.get("is_thread_op").and_then(Value::as_bool).unwrap_or(false),
    is_op: v.get("is_op").and_then(Value::as_bool).unwrap_or(reply_to.is_none()),
    reply_to,
    voted: v.get("user_vote").and_then(Value::as_i64) == Some(1),
    edited: v.get("edited_at").is_some_and(|e| !e.is_null()),
    refs: refs_of(v.get("emote_refs")),
    withheld: has && media.is_empty(),
    media_count: if has { media_count.max(1) } else { media_count },
    media,
    cats: cats_of(v),
    video: name("media_type") == Some("video"),
    reactions: reaction::parse_list(v.get("reactions")),
    pending: v.get("moderation_pending").and_then(Value::as_bool) == Some(true)
      || name("moderation_status") == Some("pending"),
  })
}

// the site's heat ladder (ext heatLadder): white, white bold, yellow, yellow bold, red bold
pub fn tier(heat: u32) -> (u32, bool) {
  match heat {
    1000.. => (theme::RED, true),
    250.. => (theme::YELLOW, true),
    50.. => (theme::YELLOW, false),
    10.. => (theme::WHITE, true),
    _ => (theme::WHITE, false),
  }
}

// 1234 -> 1.2k, 2000 -> 2k
pub fn heat_text(heat: u32) -> String {
  if heat < 1000 {
    return heat.to_string();
  }
  let k = format!("{:.1}", heat as f64 / 1000.0);
  format!("{}k", k.strip_suffix(".0").unwrap_or(&k))
}

// the number as the ext shows it: a degree sign from 10 up, never an emoji
fn heat_seg(heat: u32) -> Seg {
  let (ink, bold) = tier(heat);
  let deg = if heat >= 10 { "°" } else { "" };
  Seg { text: format!(" {}{deg}", heat_text(heat)).into(), ink, bold }
}

pub fn row_id(id: &str) -> Box<str> {
  id.into()
}

// short id the way the site writes it: leading zeros dropped
pub fn short(id: &str) -> &str {
  let s = id.trim_start_matches('0');
  if s.is_empty() { "0" } else { s }
}

// `me` = your heatsync name, `signed_in` = a token exists (the ▲ shows only then, and never on your own post)
pub fn message(p: &Post, me: Option<&str>, signed_in: bool) -> Message {
  let own = me.is_some_and(|m| m.eq_ignore_ascii_case(&p.user));
  // ext palette: >>id yellow (mention), [OP] red, the OP in their own thread magenta, [RE] cyan (reply)
  let tag = if p.thread_op {
    ("[OP] ", theme::MAGENTA)
  } else if p.is_op {
    ("[OP] ", theme::RED)
  } else {
    ("[RE] ", theme::CYAN)
  };
  let mut head = vec![
    Seg { text: format!(">>{} ", short(&p.id)).into(), ink: theme::YELLOW, bold: false },
    Seg { text: tag.0.into(), ink: tag.1, bold: true },
  ];
  if let Some(pl) = p.platform {
    let (t, ink) = match pl {
      Platform::Twitch => ("[T] ", theme::TWITCH),
      Platform::Kick => ("[K] ", theme::GREEN),
      Platform::Youtube => ("[Y] ", theme::RED),
    };
    head.push(Seg { text: t.into(), ink, bold: true });
  }
  let mut tail = Vec::new();
  if p.heat > 0 {
    tail.push(heat_seg(p.heat));
  }
  if signed_in && !own {
    tail.push(Seg { text: " ▲".into(), ink: if p.voted { theme::HEATSYNC } else { theme::DIM }, bold: p.voted });
  }
  if p.replies > 0 {
    tail.push(Seg { text: format!(" ↩{}", p.replies).into(), ink: theme::DIM, bold: false });
  }
  let mut m = Message::system(p.ts, Platform::Twitch, "");
  m.kind = Kind::Chat;
  m.id = row_id(&p.id);
  m.login = p.user.to_lowercase().into();
  // a twitch author keeps their id on the line (cards, sender refresh); every other namespace rides in `paint`
  if let Some(PaintId::Twitch(n)) = &p.paint {
    m.user_id = n.to_string().into();
  }
  m.nick = if p.anon { "Anonymous".into() } else { p.user.clone() };
  m.color = if p.anon { theme::WHITE } else { p.color };
  // the words with their **bold** and *underline* markers cut out; the runs ride on the line (model::markup)
  let (words, marks): (Box<str>, Vec<Mark>) = match markup::parse(&p.text) {
    Some((w, marks)) => (w.into(), marks),
    None => (p.text.clone(), Vec::new()),
  };
  m.body = if p.edited { format!("{words} (edited)").into() } else { words };
  m.hs = p.refs.clone();
  m.x = Some(Box::new(Extra {
    chan: "".into(),
    head: head.into(),
    tail: tail.into(),
    edge: if p.heat > 0 { tier(p.heat).0 } else { 0 },
    paint: p.paint.clone(),
    pic: pic(p, false, None).map(Box::new),
    react: p.reactions.clone(),
    late: false,
    held: None,
    marks: marks.into(),
    ..Extra::default()
  }));
  m
}

// ext inlineMediaTag: img / gif / vid, `media` when they differ, a count when there are several
fn tag(p: &Post) -> Box<str> {
  let word = |k: MediaKind| match k {
    MediaKind::Image => "img",
    MediaKind::Gif => "gif",
    MediaKind::Video => "vid",
  };
  let label = match p.media.split_first() {
    Some((a, rest)) if rest.iter().all(|b| b.kind == a.kind) => word(a.kind),
    Some(_) => "media",
    None if p.video => "vid",
    None if p.media_count > 1 => "media",
    None => "img",
  };
  let n = (p.media.len() as u8).max(p.media_count);
  if n > 1 { format!("[{n} {label}]").into() } else { format!("[{label}]").into() }
}

// what a post's media looks like on its row. `inline` = an open thread draws it; `reveal` = the
// viewer's show on a held one
pub fn pic(p: &Post, inline: bool, reveal: Option<Reveal>) -> Option<Pic> {
  if p.media_count == 0 {
    return None;
  }
  let hold = content::held(p.cats.iter().map(|c| &**c))
    .or_else(|| (p.withheld && p.media.is_empty()).then(|| content::Hold { label: "media".into(), can_show: true }));
  let open = reveal == Some(Reveal::Open) && !p.media.is_empty() && hold.as_ref().is_some_and(|h| h.can_show);
  let items: Box<[_]> = p
    .media
    .iter()
    .filter_map(|m| Some((m.pic()?.into(), (m.kind == MediaKind::Video).then(|| m.url.clone()))))
    .collect();
  let (hidden, button) = match &hold {
    None => (None, None),
    Some(_) if open => (None, Some("hide")),
    Some(h) => {
      let b = h.can_show.then_some(match reveal {
        Some(Reveal::Asking) => "loading...",
        Some(Reveal::Denied) => "log in as an adult to view",
        Some(Reveal::Unavailable) => "not available to your account",
        Some(Reveal::Failed) => "retry",
        _ => "show",
      });
      (Some(format!("[media hidden — {}]", h.label).into()), b)
    }
  };
  let peek = if p.pending { None } else { p.media.first().and_then(Media::pic).map(Into::into) };
  Some(Pic {
    tag: tag(p),
    // a held post peeks as its box's words instead (the tag's hover reads `hidden`)
    peek,
    inline,
    items: if hidden.is_some() { Box::new([]) } else { items },
    hidden,
    button: button.map(Into::into),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn sample() -> Value {
    json!({
      "base36_id": "00abc1", "username": "bob", "user_id": "7", "user_color": "#ff0000",
      "content": "a &gt; b &amp;&amp; &lt;c&gt; &#x27;q&#x27; &amp;lt;",
      "created_at": "2026-01-02T03:04:05Z", "heat": 1500, "reply_count": 3, "user_vote": 1,
      "emote_refs": { "catJAM": "https://cdn.7tv.app/a.webp", "nope": { "url": "https://cdn.7tv.app/b.webp", "nsfw": true },
                      "bad": "javascript:x", "obj": { "url": "https://cdn.7tv.app/c.webp" } },
    })
  }

  #[test]
  fn authors_carry_a_paint_id_in_their_own_namespace() {
    let mut v = sample();
    assert!(parse(&v).unwrap().paint.is_none(), "no platform = plain");
    v["platform"] = json!("twitch");
    assert_eq!(parse(&v).unwrap().paint, Some(PaintId::Twitch(7)));
    assert_eq!(&*message(&parse(&v).unwrap(), None, false).user_id, "7");
    v["user_id"] = json!(37005319);
    assert_eq!(parse(&v).unwrap().paint, Some(PaintId::Twitch(37005319)));
    // the same number on kick or as a native account is not the twitch chatter
    v["user_id"] = json!("7");
    v["platform"] = json!("kick");
    let k = parse(&v).unwrap();
    assert_eq!(k.paint, Some(PaintId::Kick(7)));
    let m = message(&k, None, false);
    assert!(m.user_id.is_empty(), "never read as a twitch id");
    assert_eq!(PaintId::of_message(&m), Some(PaintId::Kick(7)));
    v["platform"] = json!("heatsync");
    assert_eq!(parse(&v).unwrap().paint, Some(PaintId::Native("7".into())));
    assert_ne!(PaintId::of_message(&message(&parse(&v).unwrap(), None, false)), Some(PaintId::Twitch(7)));
    // a server-namespaced id keeps its namespace whatever platform label rides with it
    v["user_id"] = json!("kick_9");
    v["platform"] = json!("twitch");
    assert_eq!(parse(&v).unwrap().paint, Some(PaintId::Kick(9)));
    for bad in [json!("0"), json!("12x"), json!("99999999999999999999999")] {
      v["user_id"] = bad;
      v["platform"] = json!("twitch");
      assert!(parse(&v).unwrap().paint.is_none());
    }
    v["user_id"] = json!("yt_short");
    assert!(parse(&v).unwrap().paint.is_none());
  }

  #[test]
  fn parses_and_unescapes() {
    let p = parse(&sample()).unwrap();
    assert_eq!(&*p.id, "00abc1");
    assert_eq!(&*p.text, "a > b && <c> 'q' &lt;");
    assert!(p.is_op && p.voted && !p.anon);
    assert_eq!((p.heat, p.replies), (1500, 3));
    let names: Vec<_> = p.refs.iter().map(|r| &*r.0).collect();
    assert!(names.contains(&"catJAM") && names.contains(&"obj"));
    assert!(!names.contains(&"nope") && !names.contains(&"bad"), "gated or non-https refs never become emotes");
  }

  #[test]
  fn rejects_bad_ids_and_missing_text() {
    assert!(parse(&json!({"base36_id": "../x", "content": "hi"})).is_none());
    assert!(parse(&json!({"base36_id": "abc"})).is_none());
  }

  #[test]
  fn anonymous_and_reply_tags() {
    let p = parse(&json!({"base36_id": "1", "content": "x", "username": "Anonymous", "reply_to": "00abc1"})).unwrap();
    assert!(p.anon && !p.is_op);
    let m = message(&p, None, false);
    assert_eq!(&*m.nick, "Anonymous");
    let x = m.x.unwrap();
    assert_eq!(&*x.head[0].text, ">>1 ");
    assert_eq!(&*x.head[1].text, "[RE] ");
  }

  #[test]
  fn heat_ladder_and_text() {
    assert_eq!(tier(0), (theme::WHITE, false));
    assert_eq!(tier(10), (theme::WHITE, true));
    assert_eq!(tier(50), (theme::YELLOW, false));
    assert_eq!(tier(250), (theme::YELLOW, true));
    assert_eq!(tier(5000), (theme::RED, true));
    assert_eq!(heat_text(999), "999");
    assert_eq!(heat_text(1000), "1k");
    assert_eq!(heat_text(1234), "1.2k");
  }

  #[test]
  fn row_shows_heat_vote_and_replies_without_fire() {
    let p = parse(&sample()).unwrap();
    let m = message(&p, Some("alice"), true);
    let x = m.x.as_ref().unwrap();
    let tail: String = x.tail.iter().map(|s| &*s.text).collect();
    assert_eq!(tail, " 1.5k° ▲ ↩3");
    assert!(!tail.contains('🔥'));
    assert_eq!(x.edge, theme::RED);
    // your own post has no ▲; logged out has none either
    let own = message(&p, Some("BOB"), true);
    assert!(!own.x.unwrap().tail.iter().any(|s| s.text.contains('▲')));
    assert!(!message(&p, None, false).x.unwrap().tail.iter().any(|s| s.text.contains('▲')));
  }

  // the ext's **bold** and *italic* (underlined here): the markers leave the words, the runs ride on the line,
  // an edit still starts from what was typed, and a post without a pair is the same line it always was
  #[test]
  fn bold_and_underline_markers_leave_the_words_and_ride_as_runs() {
    let mut v = sample();
    v["content"] = json!("a **loud** and *low* one, snake_case_name, https://x.com/a_b_c");
    let p = parse(&v).unwrap();
    assert!(p.text.contains("**loud**"), "the post keeps what was typed (an edit starts from it)");
    let m = message(&p, None, false);
    assert_eq!(&*m.body, "a loud and low one, snake_case_name, https://x.com/a_b_c");
    let runs: Vec<_> = m
      .x
      .as_ref()
      .unwrap()
      .marks
      .iter()
      .map(|k| (k.style, &m.body[k.range.start as usize..k.range.end as usize]))
      .collect();
    assert_eq!(runs, [(markup::Style::Bold, "loud"), (markup::Style::Under, "low")]);
    // edited: the tag goes after the cut words, and the runs still land on theirs
    let mut e = p.clone();
    e.edited = true;
    let m = message(&e, None, false);
    assert!(m.body.ends_with("c (edited)") && m.body.starts_with("a loud and low"));
    assert_eq!(m.x.as_ref().unwrap().marks.len(), 2);
    // nothing to mark: no runs, the words as they came
    v["content"] = json!("2 * 3 and a lone ** here");
    let m = message(&parse(&v).unwrap(), None, false);
    assert_eq!(&*m.body, "2 * 3 and a lone ** here");
    assert!(m.x.as_ref().unwrap().marks.is_empty());
  }

  #[test]
  fn emote_refs_ride_the_post_not_the_viewer() {
    let m = message(&parse(&sample()).unwrap(), None, false);
    assert_eq!(m.hs.len(), 2);
    assert_eq!(&*m.user_id, "", "no twitch id, so no sender-inventory lookup can run for a feed author");
  }

  #[test]
  fn media_parses_bounded_and_allowlisted() {
    let one = json!({"base36_id": "47", "content": "x", "has_media": true, "media_count": "1",
      "media": [], "media_url": "/uploads/a.webp", "media_type": "image"});
    let p = parse(&one).unwrap();
    assert_eq!(&*p.media[0].url, "https://heatsync.org/uploads/a.webp");
    assert!(!p.withheld && p.media_count == 1);
    let many: Vec<_> =
      (0..20).map(|i| json!({"type": "image", "url": format!("https://heatsync.org/uploads/{i}.gif")})).collect();
    let mut v = json!({"base36_id": "48", "content": "x", "media": many});
    let p = parse(&v).unwrap();
    assert_eq!(p.media.len(), MAX_MEDIA);
    assert_eq!(p.media[0].kind, MediaKind::Gif);
    v["media"] = json!([{"type": "image", "url": "https://evil.example/a.png"}, {"type": "video", "url": "https://heatsync.org/v.mp4",
      "thumbnail_url": "javascript:x"}]);
    let p = parse(&v).unwrap();
    assert_eq!(p.media.len(), 1, "a url off the allowlist is dropped");
    assert_eq!((p.media[0].kind, p.media[0].pic()), (MediaKind::Video, None), "a bad still draws nothing");
    let plain = parse(&json!({"base36_id": "49", "content": "x"})).unwrap();
    assert!(plain.media.is_empty() && plain.media_count == 0 && pic(&plain, true, None).is_none());
  }

  #[test]
  fn held_media_shows_its_box_until_shown() {
    content::set_shown([]);
    // >>4a as the server sends it to a viewer not cleared: the files withheld
    let v = json!({"base36_id": "4a", "content": "x", "has_media": true, "media_count": "1", "media": [],
      "media_url": null, "media_type": "image", "media_held": ["nsfw"],
      "moderation_result": {"flaggedCategories": ["nsfw"]}});
    let mut p = parse(&v).unwrap();
    assert!(p.withheld);
    let x = pic(&p, true, None).unwrap();
    assert_eq!(
      (&*x.tag, x.hidden.as_deref(), x.button.as_deref()),
      ("[img]", Some("[media hidden — nsfw]"), Some("show"))
    );
    assert!(x.items.is_empty() && x.peek.is_none());
    assert_eq!(pic(&p, true, Some(Reveal::Denied)).unwrap().button.as_deref(), Some("log in as an adult to view"));
    assert_eq!(pic(&p, true, Some(Reveal::Failed)).unwrap().button.as_deref(), Some("retry"));
    // open with no files yet still holds
    assert!(pic(&p, true, Some(Reveal::Open)).unwrap().hidden.is_some());
    p.media = media_of(&json!({"media_url": "https://heatsync.org/uploads/b.webp", "media_type": "image"}));
    let x = pic(&p, true, Some(Reveal::Open)).unwrap();
    assert_eq!((x.hidden, x.button.as_deref(), x.items.len()), (None, Some("hide"), 1));
    // switched on: no box at all
    content::set_shown([(content::Cat::Sexual, true)]);
    let x = pic(&p, true, None).unwrap();
    assert_eq!((x.hidden, x.button), (None, None));
    // csam: a box with no button, whatever the switches
    p.cats = Box::new(["csam".into()]);
    let x = pic(&p, true, Some(Reveal::Open)).unwrap();
    assert_eq!((x.hidden.as_deref(), x.button), (Some("[media hidden — blocked]"), None));
  }

  #[test]
  fn tags_name_the_media_like_the_ext() {
    let t = |v: Value| tag(&parse(&v).unwrap());
    let base = |m: Value| json!({"base36_id": "1", "content": "x", "media": m});
    assert_eq!(&*t(base(json!([{"type": "image", "url": "https://heatsync.org/a.webp"}]))), "[img]");
    assert_eq!(&*t(base(json!([{"type": "video", "url": "https://heatsync.org/a.mp4"}]))), "[vid]");
    let mixed = json!([{"type": "image", "url": "https://heatsync.org/a.webp"}, {"type": "image", "url": "https://heatsync.org/b.gif"}]);
    assert_eq!(&*t(base(mixed)), "[2 media]");
    let held = json!({"base36_id": "1", "content": "x", "has_media": true, "media_count": 3, "media_held": ["nsfw"]});
    assert_eq!(&*t(held), "[3 media]");
    let vid =
      json!({"base36_id": "1", "content": "x", "has_media": true, "media_type": "video", "media_held": ["nsfw"]});
    assert_eq!(&*t(vid), "[vid]");
  }

  #[test]
  fn tag_colors_follow_the_ext() {
    let ink = |v: Value| {
      let x = message(&parse(&v).unwrap(), None, false).x.unwrap();
      (x.head[0].ink, x.head[1].ink)
    };
    assert_eq!(ink(json!({"base36_id": "2o", "content": "x", "username": "a"})), (theme::YELLOW, theme::RED));
    let re = json!({"base36_id": "2p", "content": "x", "username": "b", "reply_to": "2o"});
    assert_eq!(ink(re).1, theme::CYAN);
    let own = json!({"base36_id": "2q", "content": "x", "username": "a", "reply_to": "2o", "is_thread_op": true});
    assert_eq!(ink(own).1, theme::MAGENTA);
  }
}
