// what plays as a video rather than opens as a page: youtube videos, twitch clips and vods, direct video
// files; and a channel's live stream page. channel and other pages stay links (the browser knows them)
use super::ident;
use super::message::Platform;

const FILES: [&str; 4] = [".mp4", ".webm", ".mov", ".mkv"];

// the lowercase host (no `www.` / `m.`) and the rest of a http(s) link
fn split(url: &str) -> Option<(String, &str)> {
  let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
  let rest = if lower.starts_with("https://") {
    &url[8..]
  } else if lower.starts_with("http://") {
    &url[7..]
  } else {
    return None;
  };
  let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let host = rest[..end].to_ascii_lowercase();
  let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).unwrap_or(&host).to_owned();
  Some((host, &rest[end..]))
}

// the 11-character video id of a youtube watch / youtu.be / shorts / live / embed link (ext
// extractYoutubeVideoId); a channel link has none
pub fn yt_id(url: &str) -> Option<&str> {
  let (host, rest) = split(url)?;
  let path = rest.split(['?', '#']).next().unwrap_or("");
  let query = rest.split_once('?').map_or("", |(_, q)| q.split('#').next().unwrap_or(""));
  let tail = match host.as_str() {
    "youtu.be" => path.strip_prefix('/')?,
    "youtube.com" | "music.youtube.com" if path == "/watch" => query.split('&').find_map(|kv| kv.strip_prefix("v="))?,
    "youtube.com" | "music.youtube.com" => {
      ["/shorts/", "/live/", "/embed/"].iter().find_map(|p| path.strip_prefix(p))?
    }
    _ => return None,
  };
  let id = &tail[..tail.bytes().position(|b| !ident::slug_byte(b)).unwrap_or(tail.len())];
  (id.len() == 11).then_some(id)
}

// a video file served as itself (no page around it): plays without yt-dlp
pub fn file(url: &str) -> bool {
  let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
  FILES.iter().any(|e| path.ends_with(e))
}

pub fn playable(url: &str) -> bool {
  let Some((host, rest)) = split(url) else { return false };
  let path = rest.split(['?', '#']).next().unwrap_or("");
  let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
  yt_id(url).is_some()
    || file(url)
    || (host == "clips.twitch.tv" && segs.len() == 1)
    || (host == "twitch.tv" && matches!(segs[..], ["videos", _] | [_, "clip", _]))
}

// a channel's live page, as the players take it
pub fn stream(platform: Platform, key: &str) -> Option<String> {
  let ok = |s: &str| !s.is_empty() && s.len() <= 100 && s.bytes().all(|b| ident::slug_byte(b) || b == b'.');
  match platform {
    Platform::Twitch => ok(key).then(|| format!("https://www.twitch.tv/{key}")),
    Platform::Kick => ok(key).then(|| format!("https://kick.com/{key}")),
    Platform::Youtube => match key.strip_prefix('@') {
      Some(h) => ok(h).then(|| format!("https://www.youtube.com/@{h}/live")),
      None => ok(key).then(|| format!("https://www.youtube.com/channel/{key}/live")),
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn youtube_ids() {
    let id = "dQw4w9WgXcQ";
    for u in [
      format!("https://www.youtube.com/watch?v={id}"),
      format!("https://youtube.com/watch?feature=x&v={id}&t=3"),
      format!("https://m.youtube.com/watch?v={id}#t=1"),
      format!("https://youtu.be/{id}?si=abc"),
      format!("https://www.youtube.com/shorts/{id}"),
      format!("https://www.youtube.com/live/{id}"),
      format!("http://youtu.be/{id}"),
    ] {
      assert_eq!(yt_id(&u), Some(id), "{u}");
    }
    for u in [
      "https://www.youtube.com/@someone/live",
      "https://www.youtube.com/watch?v=short",
      "https://www.youtube.com/watch?v=dQw4w9WgXcQQ",
      "https://evilyoutube.com/watch?v=dQw4w9WgXcQ",
      "https://youtube.com.evil.com/watch?v=dQw4w9WgXcQ",
      "https://example.com/?v=dQw4w9WgXcQ",
      "javascript:youtu.be/dQw4w9WgXcQ",
    ] {
      assert_eq!(yt_id(u), None, "{u}");
    }
  }

  #[test]
  fn what_plays() {
    for u in [
      "https://clips.twitch.tv/FunnyClipName-abc",
      "https://www.twitch.tv/someone/clip/FunnyClip",
      "https://www.twitch.tv/videos/123456",
      "https://heatsync.org/uploads/a.MP4?x=1",
      "https://youtu.be/dQw4w9WgXcQ",
    ] {
      assert!(playable(u), "{u}");
    }
    for u in [
      "https://www.twitch.tv/someone",
      "https://kick.com/someone",
      "https://example.com/a.mp4.html",
      "https://twitch.tv/directory",
    ] {
      assert!(!playable(u), "{u}");
    }
  }

  #[test]
  fn stream_pages() {
    assert_eq!(stream(Platform::Twitch, "xqc").as_deref(), Some("https://www.twitch.tv/xqc"));
    assert_eq!(stream(Platform::Kick, "some-one").as_deref(), Some("https://kick.com/some-one"));
    assert_eq!(stream(Platform::Youtube, "@h.x").as_deref(), Some("https://www.youtube.com/@h.x/live"));
    assert_eq!(stream(Platform::Youtube, "UCabc").as_deref(), Some("https://www.youtube.com/channel/UCabc/live"));
    for bad in ["", "a/b", "a b", "@", "x?y"] {
      assert_eq!(stream(Platform::Twitch, bad), None, "{bad}");
    }
  }
}
