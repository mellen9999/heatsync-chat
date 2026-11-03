// link previews (ext chatEmbedForUrl + _buildFeedResolvedHtml): a reddit post, a tweet, a spotify track, a
// clip … in chat is asked of heatsync's resolver (`/api/embed/resolve`, which reads the provider's oembed),
// and its answer draws under the line: the picture, a video's still with a ▶, or a card (still, where,
// title, author). the answer is hostile: every string bounded, every url https and bounded. answers are
// kept per link for the session (capped); a failure is kept too, so a dead link is asked once
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use serde_json::Value;

const MAX_KEPT: usize = 500;
const MAX_TITLE: usize = 200;
const MAX_AUTHOR: usize = 80;
const MAX_URL: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
  Rich,
  Audio,
  Image,
  Video,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Preview {
  pub kind: Kind,
  // where it is from (`reddit`, `spotify`, …), from the link itself
  pub site: &'static str,
  pub title: Box<str>,
  pub author: Box<str>,
  pub thumb: Option<Box<str>>,
  pub media: Option<Box<str>>,
  // moderation categories of the media (held unless the viewer's switch is on)
  pub flagged: Box<[Box<str>]>,
}

// the providers the ext asks the resolver about, by the link's host and path (ext _CHAT_URL_RE card list)
pub fn site(url: &str) -> Option<&'static str> {
  let lower = url.to_ascii_lowercase();
  let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
  let (host, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
  let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).unwrap_or(host);
  let path = path.split(['?', '#']).next().unwrap_or("");
  let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
  let s = match (host, &segs[..]) {
    ("reddit.com" | "old.reddit.com", ["r", ..]) => "reddit",
    ("twitter.com" | "x.com", [_, "status", ..]) => "x",
    ("open.spotify.com", [k, ..]) if matches!(*k, "track" | "album" | "playlist" | "episode" | "show") => "spotify",
    ("vimeo.com", [n, ..]) if n.bytes().next().is_some_and(|b| b.is_ascii_digit()) => "vimeo",
    ("tiktok.com", [u, "video", ..]) if u.starts_with('@') => "tiktok",
    ("instagram.com", [k, ..]) if matches!(*k, "p" | "reel") => "instagram",
    ("soundcloud.com", [_, _, ..]) => "soundcloud",
    ("kick.com", [_, "clips", ..]) => "kick",
    ("clips.twitch.tv", [_, ..]) => "twitch",
    ("streamable.com", [_, ..]) => "streamable",
    _ => return None,
  };
  Some(s)
}

fn text(v: &Value, key: &str, max: usize) -> Box<str> {
  let s = v.get(key).and_then(Value::as_str).unwrap_or("");
  s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().into()
}

fn link(v: &Value, key: &str) -> Option<Box<str>> {
  let s = v.get(key)?.as_str()?;
  let ok = s.len() <= MAX_URL
    && s.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://"))
    && !s.bytes().any(|b| b.is_ascii_control() || b == b' ');
  ok.then(|| s.into())
}

// the resolver's answer for `url`; None = nothing to show (an error, an unknown type)
pub fn parse(url: &str, v: &Value) -> Option<Preview> {
  let kind = match v.get("type")?.as_str()? {
    "rich" => Kind::Rich,
    "audio" => Kind::Audio,
    "image" => Kind::Image,
    "video" => Kind::Video,
    _ => return None,
  };
  let p = Preview {
    kind,
    site: site(url)?,
    title: text(v, "title", MAX_TITLE),
    author: text(v, "author", MAX_AUTHOR),
    thumb: link(v, "thumbnail"),
    media: link(v, "mediaUrl"),
    flagged: v
      .get("flagged")
      .and_then(Value::as_array)
      .map(|a| {
        a.iter().filter_map(Value::as_str).take(8).map(|c| c.chars().take(32).collect::<String>().into()).collect()
      })
      .unwrap_or_default(),
  };
  // an image or a video with nothing to draw is a card; a card with nothing to say is nothing
  let empty = p.title.is_empty() && p.author.is_empty() && p.thumb.is_none();
  match p.kind {
    Kind::Image | Kind::Video if p.media.is_some() || p.thumb.is_some() => Some(p),
    _ if empty => None,
    Kind::Image | Kind::Video => Some(Preview { kind: Kind::Rich, ..p }),
    _ => Some(p),
  }
}

enum Slot {
  Asked,
  Done(Option<Arc<Preview>>),
}

#[derive(Default)]
pub struct Previews {
  slots: HashMap<Box<str>, Slot>,
  order: VecDeque<Box<str>>,
  wanted: Vec<Box<str>>,
}

impl Previews {
  // the preview of `url` once it is here; the first look asks for it
  pub fn get(&mut self, url: &str) -> Option<Arc<Preview>> {
    match self.slots.get(url) {
      Some(Slot::Done(p)) => p.clone(),
      Some(Slot::Asked) => None,
      None => {
        if self.order.len() >= MAX_KEPT
          && let Some(old) = self.order.pop_front()
        {
          self.slots.remove(&old);
        }
        self.slots.insert(url.into(), Slot::Asked);
        self.order.push_back(url.into());
        self.wanted.push(url.into());
        None
      }
    }
  }

  // true when it changes what draws
  pub fn answered(&mut self, url: &str, p: Option<Preview>) -> bool {
    let Some(s) = self.slots.get_mut(url) else { return false };
    let shows = p.is_some();
    *s = Slot::Done(p.map(Arc::new));
    shows
  }

  pub fn take_wanted(&mut self) -> Vec<Box<str>> {
    std::mem::take(&mut self.wanted)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn which_links_ask() {
    for (u, s) in [
      ("https://www.reddit.com/r/rust/comments/1c0rgn3/x/", "reddit"),
      ("https://x.com/someone/status/123", "x"),
      ("https://twitter.com/someone/status/123?s=20", "x"),
      ("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT", "spotify"),
      ("https://vimeo.com/76979871", "vimeo"),
      ("https://www.tiktok.com/@a.b/video/1", "tiktok"),
      ("https://www.instagram.com/reel/abc/", "instagram"),
      ("https://soundcloud.com/a/b", "soundcloud"),
      ("https://kick.com/someone/clips/clip_01", "kick"),
      ("https://clips.twitch.tv/Funny-abc", "twitch"),
      ("https://streamable.com/moo", "streamable"),
    ] {
      assert_eq!(site(u), Some(s), "{u}");
    }
    for u in [
      "https://reddit.com/",
      "https://x.com/someone",
      "https://vimeo.com/about",
      "https://kick.com/someone",
      "https://evilreddit.com/r/x",
      "https://reddit.com.evil.com/r/x",
      "ftp://reddit.com/r/x",
    ] {
      assert_eq!(site(u), None, "{u}");
    }
  }

  #[test]
  fn answers_are_bounded_and_checked() {
    let u = "https://open.spotify.com/track/x";
    let p = parse(
      u,
      &json!({"type": "audio", "title": "a\nb".repeat(300), "author": "Spotify", "thumbnail": "https://i.scdn.co/a",
        "mediaUrl": "javascript:alert(1)", "flagged": ["nsfw"]}),
    )
    .unwrap();
    assert_eq!((p.kind, p.site, &*p.author), (Kind::Audio, "spotify", "Spotify"));
    assert!(p.title.chars().count() <= MAX_TITLE && !p.title.contains('\n'));
    assert_eq!((p.thumb.as_deref(), p.media.as_deref()), (Some("https://i.scdn.co/a"), None));
    assert_eq!(&*p.flagged, &["nsfw".into()]);
    assert!(parse(u, &json!({"error": "unsupported_or_failed"})).is_none());
    assert!(parse(u, &json!({"type": "rich"})).is_none(), "nothing to say");
    assert!(parse("https://example.com/", &json!({"type": "rich", "title": "x"})).is_none(), "not a provider");
    let v = parse("https://clips.twitch.tv/a", &json!({"type": "video", "title": "t"})).unwrap();
    assert_eq!(v.kind, Kind::Rich, "a video with no file or still is a card");
  }

  #[test]
  fn asked_once_kept_capped() {
    let mut p = Previews::default();
    assert!(p.get("https://x.com/a/status/1").is_none());
    assert!(p.get("https://x.com/a/status/1").is_none());
    assert_eq!(p.take_wanted().len(), 1, "asked once");
    assert!(!p.answered("https://x.com/a/status/1", None));
    assert!(p.get("https://x.com/a/status/1").is_none());
    assert!(p.take_wanted().is_empty(), "a failure is not asked again");
    for i in 0..MAX_KEPT + 5 {
      p.get(&format!("https://x.com/a/status/{i}"));
    }
    assert!(p.slots.len() <= MAX_KEPT);
  }
}
