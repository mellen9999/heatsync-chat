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

// each provider's emote files, smallest first: the file's name and how tall the picture is
const RUNGS: [(&str, &[(&str, u32)]); 4] = [
  ("7tv.app", &[("1x", 32), ("2x", 64), ("3x", 96), ("4x", 128)]),
  ("betterttv.net", &[("1x", 28), ("2x", 56), ("3x", 112)]),
  ("frankerfacez.com", &[("1", 32), ("2", 64), ("4", 128)]),
  ("jtvnw.net", &[("1.0", 28), ("2.0", 56), ("3.0", 112)]),
];

// the file of the same emote that is at least `px` tall, or the biggest it has: the next rung up from the
// one the url names. None when the url has no bigger rung (heatsync and kick emotes are one file)
pub fn rung(url: &str, px: u32) -> Option<String> {
  let h = host(url)?.to_ascii_lowercase();
  let (dir, file) = url.rsplit_once('/')?;
  let (_, rungs) = RUNGS.iter().find(|(d, _)| h == *d || h.strip_suffix(d).is_some_and(|p| p.ends_with('.')))?;
  // the file is a rung's name, then its extension if it has one
  let (now, ext) = rungs.iter().enumerate().find_map(|(i, (name, _))| {
    file.strip_prefix(name).filter(|e| e.is_empty() || e.starts_with('.')).map(|e| (i, e))
  })?;
  let to = rungs.iter().position(|r| r.1 >= px).unwrap_or(rungs.len() - 1);
  (to > now).then(|| format!("{dir}/{}{ext}", rungs[to].0))
}

// the biggest rung, like the ext's getHighResUrl: 7tv 4x, bttv 3x, ffz 4, twitch 3.0
pub fn hires(url: &str) -> Option<String> {
  rung(url, u32::MAX)
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

  // chat's 1x file, asked for at a height: the smallest rung that is that tall, never the same file
  #[test]
  fn the_rung_for_a_height_is_the_next_one_that_is_tall_enough() {
    let r = rung;
    // a big line at 1x display: 56 high
    assert_eq!(
      r("https://cdn.7tv.app/emote/01ABC/1x.webp", 56).as_deref(),
      Some("https://cdn.7tv.app/emote/01ABC/2x.webp")
    );
    assert_eq!(
      r("https://cdn.betterttv.net/emote/5/1x.webp", 56).as_deref(),
      Some("https://cdn.betterttv.net/emote/5/2x.webp")
    );
    assert_eq!(
      r("https://cdn.frankerfacez.com/emote/9/animated/1", 56).as_deref(),
      Some("https://cdn.frankerfacez.com/emote/9/animated/2")
    );
    assert_eq!(
      r("https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0", 56).as_deref(),
      Some("https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0")
    );
    // 112 high (a 2x display): the one that holds it, and the biggest when none does
    assert_eq!(
      r("https://cdn.7tv.app/emote/01ABC/1x.webp", 112).as_deref(),
      Some("https://cdn.7tv.app/emote/01ABC/4x.webp")
    );
    assert_eq!(
      r("https://cdn.betterttv.net/emote/5/1x.webp", 112).as_deref(),
      Some("https://cdn.betterttv.net/emote/5/3x.webp")
    );
    assert_eq!(
      r("https://cdn.7tv.app/emote/01ABC/1x.webp", 900).as_deref(),
      Some("https://cdn.7tv.app/emote/01ABC/4x.webp")
    );
    // already tall enough, one file only, another host, junk: nothing to swap
    for (u, px) in [
      ("https://cdn.7tv.app/emote/01ABC/2x.webp", 56),
      ("https://cdn.7tv.app/emote/01ABC/4x.webp", 900),
      ("https://cdn.7tv.app/emote/01ABC/1x.webp", 16),
      ("https://cdn.heatsync.org/e/1x.png", 56),
      ("https://kick.com/emotes/37225/fullsize", 56),
      ("https://evil7tv.app/emote/01ABC/1x.webp", 56),
      ("https://cdn.7tv.app/emote/01ABC/10x.webp", 56),
      ("x", 56),
    ] {
      assert_eq!(r(u, px), None, "{u} at {px}");
    }
    // the swap never leaves the allowlist
    assert!(allowed(&r("https://cdn.7tv.app/emote/01ABC/1x.webp", 56).unwrap()));
  }
}
