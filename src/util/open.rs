// open a link from chat in the default browser. the text is hostile: only http(s), no spaces or control
// characters, bounded, and never through a shell (xdg-open gets the url as one argument)
use std::process::{Command, Stdio};

pub fn open_url(url: &str) -> Result<(), String> {
  let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
  let ok = (lower.starts_with("https://") || lower.starts_with("http://"))
    && url.len() <= 2048
    && !url.chars().any(|c| c.is_control() || c.is_whitespace());
  if !ok {
    return Err("not a link i can open".into());
  }
  let mut child = Command::new("xdg-open")
    .arg(url)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| format!("couldn't open the link (xdg-open: {e})"))?;
  // reap it so it never lingers as a zombie
  std::thread::spawn(move || _ = child.wait());
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_plain_web_links_pass() {
    for bad in ["file:///etc/passwd", "javascript:alert(1)", "https://a b", "https://a\n", "ftp://x", "-h", ""] {
      assert!(open_url(bad).is_err(), "{bad}");
    }
    assert!(open_url(&format!("https://a.com/{}", "x".repeat(3000))).is_err());
  }
}
