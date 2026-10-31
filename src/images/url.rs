// image host allowlist: https only, no userinfo, bare host, exact or dot-suffix match.
// (the `kick.com:@evil.com` bypass is why userinfo is rejected outright)
const HOSTS: [&str; 10] = [
  "7tv.app",
  "7tv.io",
  "betterttv.net",
  "frankerfacez.com",
  "kick.com",
  "ggpht.com",
  "googleusercontent.com",
  "static-cdn.jtvnw.net",
  "heatsync.org",
  "fourtf.com",
];
const MAX_URL: usize = 2048;

pub fn host(url: &str) -> Option<&str> {
  let rest = url.strip_prefix("https://")?;
  let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let host = &rest[..end];
  let ok = !host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
  ok.then_some(host)
}

// the big rung of a provider's emote url, like the ext's getHighResUrl: 7tv 4x, bttv 3x, ffz 4,
// twitch 3.0. None when the url has no other rung (heatsync and kick emotes are one file)
pub fn hires(url: &str) -> Option<String> {
  let h = host(url)?.to_ascii_lowercase();
  let (dir, file) = url.rsplit_once('/')?;
  let (stem, ext) = file.split_once('.').map_or((file, ""), |(s, e)| (s, e));
  let dotted = |ext: &str| if ext.is_empty() { String::new() } else { format!(".{ext}") };
  let big = if h.ends_with("7tv.app") && matches!(stem, "1x" | "2x" | "3x") {
    format!("4x{}", dotted(ext))
  } else if h.ends_with("betterttv.net") && matches!(stem, "1x" | "2x") {
    format!("3x{}", dotted(ext))
  } else if h.ends_with("frankerfacez.com") && matches!(stem, "1" | "2") && ext.is_empty() {
    "4".into()
  } else if h.ends_with("jtvnw.net") && matches!(file, "1.0" | "2.0") {
    "3.0".into()
  } else {
    return None;
  };
  Some(format!("{dir}/{big}"))
}

pub fn allowed(url: &str) -> bool {
  if url.len() > MAX_URL || url.bytes().any(|b| b.is_ascii_control() || b == b' ') {
    return false;
  }
  let Some(h) = host(url) else { return false };
  let h = h.to_ascii_lowercase();
  HOSTS.iter().any(|a| h == *a || h.strip_suffix(a).is_some_and(|p| p.ends_with('.')))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn allowlist() {
    assert!(allowed("https://cdn.7tv.app/emote/x/1x.webp"));
    assert!(allowed("https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0"));
    assert!(allowed("https://cdn.heatsync.org/uploads/a.webp"));
    assert!(!allowed("http://cdn.7tv.app/x"));
    assert!(!allowed("https://kick.com:@evil.com/x"));
    assert!(!allowed("https://user@cdn.7tv.app/x"));
    assert!(!allowed("https://evil7tv.app/x"));
    assert!(!allowed("https://7tv.app.evil.com/x"));
    assert!(!allowed("https://cdn.7tv.app/x\n"));
    assert!(!allowed("https://cdn.7tv.app:8443/x"));
  }

  #[test]
  fn the_big_rung_per_provider() {
    let h = |u| hires(u);
    assert_eq!(
      h("https://cdn.7tv.app/emote/01ABC/1x.webp").as_deref(),
      Some("https://cdn.7tv.app/emote/01ABC/4x.webp")
    );
    assert_eq!(
      h("https://cdn.7tv.app/emote/01ABC/2x.avif").as_deref(),
      Some("https://cdn.7tv.app/emote/01ABC/4x.avif")
    );
    assert_eq!(
      h("https://cdn.betterttv.net/emote/5/1x.webp").as_deref(),
      Some("https://cdn.betterttv.net/emote/5/3x.webp")
    );
    assert_eq!(h("https://cdn.frankerfacez.com/emote/9/1").as_deref(), Some("https://cdn.frankerfacez.com/emote/9/4"));
    assert_eq!(
      h("https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0").as_deref(),
      Some("https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/3.0")
    );
    // already big, one-file hosts, other hosts, junk
    for u in [
      "https://cdn.7tv.app/emote/01ABC/4x.webp",
      "https://cdn.heatsync.org/e/1x.png",
      "https://evil.example/1x.webp",
      "x",
    ] {
      assert_eq!(h(u), None, "{u}");
    }
    // the rung swap never leaves the allowlist
    assert!(allowed(&h("https://cdn.7tv.app/emote/01ABC/1x.webp").unwrap()));
  }
}
