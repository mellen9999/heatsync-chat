// emoji shortcodes, the ext's table (chrome/emoji-data.js, gemoji names). one text file compiled in
// as `name glyph` lines: nothing is parsed into ram, a lookup walks the text. glyphs draw through the
// system color emoji font like any other text; there is no emoji image path
use std::borrow::Cow;

const TABLE: &str = include_str!("emoji.txt");
// a completion list never needs more than this (the ext stops at 50 shared with emotes)
pub const MAX: usize = 50;

pub fn rows() -> impl Iterator<Item = (&'static str, &'static str)> {
  TABLE.lines().filter_map(|l| l.split_once(' '))
}

pub fn lookup(name: &str) -> Option<&'static str> {
  rows().find(|(n, _)| *n == name).map(|(_, g)| g)
}

// shortcodes that start with the query, shorter first then a-z, then ones that hold it, a-z (ext
// findEmoteMatches). any case. empty query = nothing
pub fn search(query: &str, limit: usize) -> Vec<(&'static str, &'static str)> {
  let low = query.trim_start_matches(':').trim_end_matches(':').to_ascii_lowercase();
  if low.is_empty() {
    return Vec::new();
  }
  let (mut pre, mut held): (Vec<_>, Vec<_>) = (Vec::new(), Vec::new());
  for r in rows() {
    if r.0.starts_with(&low) {
      pre.push(r);
    } else if r.0.contains(&low) {
      held.push(r);
    }
  }
  pre.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(b.0)));
  held.sort_by(|a, b| a.0.cmp(b.0));
  pre.extend(held);
  pre.truncate(limit);
  pre
}

// the ext picker's fuzzy score (heatsync-button.js fuzzyScore) of a lowercase query against a name: a
// substring beats a scattered match, 0 = no match. higher is better
pub fn fuzzy(query: &str, name: &str) -> f32 {
  let lower = name.to_ascii_lowercase();
  if lower.contains(query) {
    return 2.0 + query.len() as f32 / lower.len() as f32;
  }
  let mut want = query.chars().peekable();
  for c in lower.chars() {
    if want.peek() == Some(&c) {
      want.next();
    }
  }
  if want.peek().is_some() { 0.0 } else { query.chars().count() as f32 / lower.chars().count() as f32 }
}

fn code_char(c: char) -> bool {
  c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'
}

// `:smile:` → 😄 on send (ext convertEmojiShortcodes). an unknown name stays as typed; a match is
// consumed either way, like the ext's regex, so `:nope:smile:` leaves the second colon pair alone
pub fn convert(text: &str) -> Cow<'_, str> {
  if !text.contains(':') {
    return Cow::Borrowed(text);
  }
  let mut out = String::with_capacity(text.len());
  let mut rest = text;
  let mut changed = false;
  while let Some(i) = rest.find(':') {
    let tail = &rest[i + 1..];
    let n = tail.find(|c| !code_char(c)).unwrap_or(tail.len());
    if n > 0 && tail[n..].starts_with(':') {
      out.push_str(&rest[..i]);
      match lookup(&tail[..n]) {
        Some(g) => {
          out.push_str(g);
          changed = true;
        }
        None => out.push_str(&rest[i..i + n + 2]),
      }
      rest = &tail[n + 1..];
    } else {
      out.push_str(&rest[..=i]);
      rest = tail;
    }
  }
  if !changed {
    return Cow::Borrowed(text);
  }
  out.push_str(rest);
  Cow::Owned(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn table_is_well_formed() {
    let n = rows().count();
    assert!(n > 1900, "{n}");
    assert_eq!(n, TABLE.lines().count(), "every line is `name glyph`");
    assert!(rows().all(|(n, g)| !n.is_empty() && !g.is_empty() && !g.contains(' ')));
    assert_eq!(lookup("100"), Some("💯"));
    assert_eq!(lookup("nope"), None);
  }

  #[test]
  fn search_prefix_first_shorter_first_then_holds() {
    let r: Vec<_> = search(":smil", 50).into_iter().map(|r| r.0).collect();
    assert_eq!(r[0], "smile");
    // an exact name leads (shortest prefix match), and a hold ranks after every prefix
    let r: Vec<_> = search("heart", 200).into_iter().map(|r| r.0).collect();
    assert_eq!(r[0], "heart");
    let first_hold = r.iter().position(|n| !n.starts_with("heart")).unwrap();
    assert!(r[..first_hold].windows(2).all(|w| w[0].len() < w[1].len() || (w[0].len() == w[1].len() && w[0] < w[1])));
    assert!(r[first_hold..].windows(2).all(|w| w[0] < w[1]));
    // any case, colons ignored, bounded, empty is nothing
    assert_eq!(search("SMILE", 3).len(), 3);
    assert_eq!(search(":smile:", 1)[0].0, "smile");
    assert!(search("", 5).is_empty() && search(":", 5).is_empty());
    assert!(search("zzzzzz", 5).is_empty());
  }

  #[test]
  fn fuzzy_matches_like_the_ext_picker() {
    // a substring beats a scattered match; no match is 0; an empty name never divides by zero
    assert!(fuzzy("smi", "smile") > fuzzy("sml", "smile"));
    assert!(fuzzy("sml", "smile") > 0.0 && fuzzy("sml", "smile") < 1.0);
    assert_eq!(fuzzy("xyz", "smile"), 0.0);
    assert_eq!(fuzzy("smile", "smile"), 3.0);
    assert!(fuzzy("e", "").is_nan() || fuzzy("e", "") == 0.0);
  }

  #[test]
  fn shortcodes_convert_on_send() {
    assert_eq!(convert("hi :smile: there"), "hi 😄 there");
    assert_eq!(convert(":100::100:"), "💯💯");
    // unknown, unclosed, empty and spaced stay as typed; nothing to change borrows
    assert!(matches!(convert("plain"), Cow::Borrowed(_)));
    assert!(matches!(convert("a :nope: b"), Cow::Borrowed(_)));
    assert_eq!(convert("::  :smile"), "::  :smile");
    assert_eq!(convert("12:30:45"), "12:30:45");
    assert_eq!(convert("a:b :smile: c"), "a:b 😄 c");
    // a match is consumed even when unknown (the ext's regex)
    assert_eq!(convert(":nope:smile:"), ":nope:smile:");
    // uppercase is not a shortcode; multibyte neighbors are safe
    assert_eq!(convert(":SMILE: é:smile:é"), ":SMILE: é😄é");
  }
}
