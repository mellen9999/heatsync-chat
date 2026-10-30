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
  let ok = !host.is_empty()
    && host
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
  ok.then_some(host)
}

pub fn allowed(url: &str) -> bool {
  if url.len() > MAX_URL || url.bytes().any(|b| b.is_ascii_control() || b == b' ') {
    return false;
  }
  let Some(h) = host(url) else { return false };
  let h = h.to_ascii_lowercase();
  HOSTS
    .iter()
    .any(|a| h == *a || h.strip_suffix(a).is_some_and(|p| p.ends_with('.')))
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
}
