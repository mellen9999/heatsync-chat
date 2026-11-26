// a twitch gif-keyboard line: twitch sends the line as a caption ("[Mood Lol GIF by Giphy]") and names the
// picture only in the EventSub copy heatsync reads. heatsync tells the room, keyed by message id like
// `chat:origin`: `{type:'chat:gif', platform:'twitch', channel, messageId, gifs:[{text, url}]}`. the line draws the
// picture where its caption was. never matched by words
//
// the rule is the site's `shared/gif-links.ts` chatGif / chatGifs and `client/utils/chat-gifs.js`: https, a giphy or
// tenor media host, a picture file, the query dropped, four at most, one per caption. a little stricter where the site's
// url parser would quietly mend a url (a port, a dot segment, a space, a backslash, a link over 600 characters)
use std::collections::{HashMap, VecDeque};
use std::ops::Range;

use serde_json::Value;

use super::ident;

// the media hosts a gif fragment may name (gif-links.ts GIPHY_MEDIA_HOSTS + TENOR_MEDIA_HOSTS)
const HOSTS: [&str; 10] = [
  "media.giphy.com",
  "media0.giphy.com",
  "media1.giphy.com",
  "media2.giphy.com",
  "media3.giphy.com",
  "media4.giphy.com",
  "i.giphy.com",
  "media.tenor.com",
  "media1.tenor.com",
  "c.tenor.com",
];
const FILES: [&str; 6] = [".gif", ".webp", ".png", ".jpg", ".jpeg", ".avif"];
// a line holds one caption per gif and twitch caps a line at 500 characters
const MAX: usize = 4;
const TEXT_MAX: usize = 500;
// short enough that the proxied form stays under the 2048 url cap even fully escaped (ui::embed)
const URL_MAX: usize = 600;
// the lines whose picture came before the line did
const HELD: usize = 300;

// the caption as twitch sent it, and the picture that replaces it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
  pub text: Box<str>,
  pub url: Box<str>,
}

// one `chat:gif`: the room (a twitch login), the line (its message id) and the line's picture
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
  pub key: Box<str>,
  pub id: Box<str>,
  pub pic: Picture,
}

// one picture, or None
fn one(url: &str, text: &str) -> Option<Picture> {
  if text.is_empty() || text.encode_utf16().count() > TEXT_MAX || url.len() > URL_MAX {
    return None;
  }
  // a url with a space, a control byte or a backslash in it is refused whole, not mended
  if url.bytes().any(|b| b.is_ascii_control() || b == b' ' || b == b'\\') {
    return None;
  }
  let rest = url.get(..8).filter(|p| p.eq_ignore_ascii_case("https://")).map(|_| &url[8..])?;
  // the host is the whole authority: a port or `user@` makes it a name that is not on the list
  let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let host = rest[..end].to_ascii_lowercase();
  if !HOSTS.contains(&&*host) {
    return None;
  }
  // the path, minus the query and the fragment (tracking)
  let path = rest[end..].split(['?', '#']).next().unwrap_or("");
  let file = path.to_ascii_lowercase();
  if !FILES.iter().any(|e| file.ends_with(e)) {
    return None;
  }
  if file.split('/').any(|s| matches!(&*s.replace("%2e", "."), "." | "..")) {
    return None;
  }
  Some(Picture { text: text.into(), url: format!("https://{host}{path}").into() })
}

// the pictures of a list of unknown shape: each through `one`, one per caption, four at most
pub fn pictures(raw: Option<&Value>) -> Vec<Picture> {
  let mut out: Vec<Picture> = Vec::new();
  for item in raw.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
    if out.len() >= MAX {
      break;
    }
    let got = item.get("url").and_then(Value::as_str).zip(item.get("text").and_then(Value::as_str));
    if let Some(p) = got.and_then(|(url, text)| one(url, text))
      && !out.iter().any(|o| o.text == p.text)
    {
      out.push(p);
    }
  }
  out
}

// a `chat:gif` frame: twitch, a room, a message id, and a picture that passes. anything else is nothing.
// the line draws the first picture, as the site does
pub fn frame(v: &Value) -> Option<Frame> {
  if v.get("platform").and_then(Value::as_str) != Some("twitch") {
    return None;
  }
  let room = v.get("channel").and_then(Value::as_str)?.to_ascii_lowercase();
  let key = room.strip_prefix('#').unwrap_or(&room);
  let id = v.get("messageId").and_then(Value::as_str).filter(|i| ident::relay_key(i))?;
  let pic = pictures(v.get("gifs")).into_iter().next()?;
  ident::twitch_room(key).then(|| Frame { key: key.into(), id: id.into(), pic })
}

// where the caption sits in the line, when it is all the line says: from its first occurrence to its last. words
// besides it keep the whole line on screen; a reply's leading @name is the reply, not something said
pub fn fold(body: &str, text: &str, reply: bool) -> Option<Range<usize>> {
  let first = body.find(text).filter(|_| !text.is_empty())?;
  let left = body.replace(text, "");
  let rest = left.trim_start();
  let rest = match rest.strip_prefix('@') {
    Some(name) if reply && name.starts_with(|c: char| !c.is_whitespace()) => {
      name.trim_start_matches(|c: char| !c.is_whitespace())
    }
    _ => rest,
  };
  rest.trim().is_empty().then(|| first..body.rfind(text).unwrap_or(first) + text.len())
}

// what `chat:gif` named for a line that has not been drawn yet: the event comes down heatsync's socket and the line
// down twitch's, neither waits for the other. keyed by message id, the oldest forgotten past 300
#[derive(Default)]
pub struct Ledger {
  map: HashMap<Box<str>, Picture>,
  order: VecDeque<Box<str>>,
}

impl Ledger {
  pub fn note(&mut self, id: &str, pic: Picture) {
    if self.map.insert(id.into(), pic).is_some() {
      self.order.retain(|o| **o != *id);
    }
    self.order.push_back(id.into());
    if self.order.len() > HELD
      && let Some(old) = self.order.pop_front()
    {
      self.map.remove(&old);
    }
  }

  pub fn get(&self, id: &str) -> Option<&Picture> {
    self.map.get(id)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  const CAPTION: &str = "[Mood Lol GIF by Giphy]";
  const GIPHY: &str = "https://media.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif";

  fn pic(url: &str) -> Option<Picture> {
    one(url, CAPTION)
  }

  #[test]
  fn a_giphy_or_tenor_picture_is_kept_as_twitch_sent_it() {
    assert_eq!(pic(GIPHY).unwrap(), Picture { text: CAPTION.into(), url: GIPHY.into() });
    // the shard the sender got, the query (tracking) dropped, the `/media/v1.<cid>/` form too
    let v1 = "https://media4.giphy.com/media/v1.Y2lkPTc5MGI3NjEx/giphy.gif?cid=abc&ep=v1_gifs_search&rid=giphy.gif";
    assert_eq!(&*pic(v1).unwrap().url, "https://media4.giphy.com/media/v1.Y2lkPTc5MGI3NjEx/giphy.gif");
    assert_eq!(
      &*pic("https://media.tenor.com/ab12/tenor.gif#x").unwrap().url,
      "https://media.tenor.com/ab12/tenor.gif"
    );
    // a scheme and host in capitals read as the lowercase ones
    assert_eq!(
      &*pic("HTTPS://Media.Giphy.com/media/a/GIPHY.GIF").unwrap().url,
      "https://media.giphy.com/media/a/GIPHY.GIF"
    );
    for file in ["a.webp", "a.png", "a.jpg", "a.jpeg", "a.avif"] {
      assert!(pic(&format!("https://i.giphy.com/{file}")).is_some(), "{file}");
    }
  }

  #[test]
  fn an_untrusted_url_is_refused() {
    for bad in [
      "http://media.giphy.com/media/abc/giphy.gif",
      "https://media.giphy.com.attacker.net/media/abc/giphy.gif",
      "https://attacker-media.giphy.com/media/abc/giphy.gif",
      "https://evil.example/media/abc/giphy.gif",
      "https://user:pw@media.giphy.com/media/abc/giphy.gif",
      "https://media.giphy.com@evil.example/abc/giphy.gif",
      "https://media.giphy.com:8443/media/abc/giphy.gif",
      "https://media.giphy.com\\@evil.example/abc.gif",
      "https://giphy.com/gifs/abc.gif",
      "javascript:alert(1)",
      "data:image/gif;base64,R0lGODlhAQABAAAAACw=",
      "//media.giphy.com/media/abc/giphy.gif",
      "not a url",
      "",
    ] {
      assert!(pic(bad).is_none(), "{bad}");
    }
  }

  #[test]
  fn a_url_that_is_not_a_picture_file_is_refused() {
    for bad in [
      "https://media.giphy.com/media/abc/",
      "https://media.giphy.com/media/abc/giphy.mp4",
      "https://media.giphy.com/media/abc/giphy.gif.exe",
      "https://media.giphy.com/media/abc/giphy?x=a.gif",
      "https://media.giphy.com",
      "https://media.giphy.com?a.gif",
    ] {
      assert!(pic(bad).is_none(), "{bad}");
    }
  }

  #[test]
  fn a_url_a_parser_would_mend_is_refused_whole() {
    for bad in [
      "https://media.giphy.com/media/../x/giphy.gif",
      "https://media.giphy.com/media/%2e%2E/giphy.gif",
      "https://media.giphy.com/media/a b/giphy.gif",
      "https://media.giphy.com/media/a\tb/giphy.gif",
      "https://media.giphy.com/media/a\nb/giphy.gif",
      "https://media.giphy.com/media\\a/giphy.gif",
    ] {
      assert!(pic(bad).is_none(), "{bad:?}");
    }
    let long = format!("https://media.giphy.com/media/{}/giphy.gif", "a".repeat(URL_MAX));
    assert!(pic(&long).is_none(), "a url the proxy form could not hold");
  }

  #[test]
  fn the_caption_is_one_to_five_hundred_characters() {
    assert!(one(GIPHY, "").is_none());
    assert!(one(GIPHY, &"x".repeat(TEXT_MAX)).is_some());
    assert!(one(GIPHY, &"x".repeat(TEXT_MAX + 1)).is_none());
  }

  #[test]
  fn a_list_of_unknown_shape_keeps_the_good_ones_once_each_four_at_most() {
    let good = |t: &str, id: &str| json!({"text": t, "url": format!("https://media.giphy.com/media/{id}/giphy.gif")});
    assert!(pictures(None).is_empty() && pictures(Some(&json!("nope"))).is_empty());
    let mixed = json!([good("[a]", "a"), null, 7, {"text": "[b]"}, {"url": GIPHY}, good("[a]", "a2"), [1], {"text": 1, "url": GIPHY}]);
    let got = pictures(Some(&mixed));
    assert_eq!(got.len(), 1, "wrong shapes dropped, the same caption twice is one");
    assert_eq!(&*got[0].url, "https://media.giphy.com/media/a/giphy.gif");
    // one bad gif does not take the good one beside it
    let beside = json!([{"text": "[bad]", "url": "https://evil.example/x.gif"}, good("[good]", "g")]);
    assert_eq!(&*pictures(Some(&beside))[0].text, "[good]");
    let nine: Vec<Value> = (0..9).map(|i| good(&format!("[g{i}]"), &format!("id{i}"))).collect();
    assert_eq!(pictures(Some(&Value::Array(nine))).len(), MAX);
  }

  fn wire(over: Value) -> Value {
    let mut v = json!({"type": "chat:gif", "platform": "twitch", "channel": "Forsen", "messageId": "m-1",
      "gifs": [{"text": CAPTION, "url": GIPHY}]});
    for (k, x) in over.as_object().unwrap() {
      v[k] = x.clone();
    }
    v
  }

  #[test]
  fn a_frame_names_one_line_in_one_room() {
    let f = frame(&wire(json!({}))).unwrap();
    assert_eq!((&*f.key, &*f.id, &*f.pic.text, &*f.pic.url), ("forsen", "m-1", CAPTION, GIPHY));
    assert_eq!(&*frame(&wire(json!({"channel": "#Forsen"}))).unwrap().key, "forsen", "a # is not part of the room");
  }

  #[test]
  fn a_frame_that_names_nothing_trusted_is_nothing() {
    for over in [
      json!({"platform": "kick"}),
      json!({"platform": "twitch ", "channel": "forsen"}),
      json!({"channel": ""}),
      json!({"channel": "not a room"}),
      json!({"channel": 5}),
      json!({"messageId": ""}),
      json!({"messageId": "a b"}),
      json!({"messageId": 7}),
      json!({"gifs": []}),
      json!({"gifs": "x"}),
      json!({"gifs": [{"text": CAPTION, "url": "https://evil.example/a.gif"}]}),
      json!({"gifs": [{"text": CAPTION, "url": "http://media.giphy.com/a.gif"}]}),
    ] {
      assert!(frame(&wire(over.clone())).is_none(), "{over}");
    }
  }

  #[test]
  fn a_caption_alone_folds_and_a_line_with_words_does_not() {
    let at = |body: &str, reply: bool| fold(body, CAPTION, reply);
    assert_eq!(at(CAPTION, false), Some(0..CAPTION.len()));
    assert_eq!(at(&format!("  {CAPTION} "), false), Some(2..2 + CAPTION.len()));
    // twice is still only the caption
    let twice = format!("{CAPTION} {CAPTION}");
    assert_eq!(at(&twice, false), Some(0..twice.len()));
    assert_eq!(at(&format!("lol {CAPTION}"), false), None, "words keep the line, caption and all");
    assert_eq!(at(&format!("{CAPTION} lol"), false), None);
    assert_eq!(at("something else", false), None, "a caption the line does not hold");
    assert_eq!(fold("anything", "", false), None);
  }

  #[test]
  fn a_replys_at_name_is_not_something_said() {
    let body = format!("@bob {CAPTION}");
    assert_eq!(fold(&body, CAPTION, true), Some(5..5 + CAPTION.len()), "only the caption folds, the name stays");
    assert_eq!(fold(&body, CAPTION, false), None, "the same words on a line that answers nobody");
    assert_eq!(fold(&format!("@bob lol {CAPTION}"), CAPTION, true), None);
    assert_eq!(fold(&format!("@ {CAPTION}"), CAPTION, true), None, "a bare @ is a word");
  }

  #[test]
  fn the_ledger_holds_a_picture_by_id_and_forgets_the_oldest() {
    let mut l = Ledger::default();
    let p = |n: usize| Picture { text: format!("[{n}]").into(), url: GIPHY.into() };
    assert!(l.get("a").is_none());
    l.note("a", p(1));
    assert_eq!(l.get("a"), Some(&p(1)));
    assert!(l.get("b").is_none(), "by id, never by words");
    for i in 0..HELD + 10 {
      l.note(&format!("id{i}"), p(i));
    }
    assert!(l.get("a").is_none() && l.get("id0").is_none(), "the oldest are forgotten");
    assert!(l.get(&format!("id{}", HELD + 9)).is_some());
    assert_eq!((l.map.len(), l.order.len()), (HELD, HELD), "bounded");
    // said again, a line is the newest, and says what it said last
    l.note("id20", p(99));
    for i in 0..HELD - 1 {
      l.note(&format!("n{i}"), p(i));
    }
    assert_eq!(l.get("id20"), Some(&p(99)));
    assert_eq!((l.map.len(), l.order.len()), (HELD, HELD));
  }
}
