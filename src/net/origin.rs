// `chat:origin_tag`: told to heatsync after a line goes out, exactly as heatsync.org does. it flags the
// line `[H]` (sent through heatsync) for your other devices and the archive, and carries the emotes you
// picked in it so the server stores your pick on the line for good (it checks every ref itself: the
// sender must own an hs emote, ids must fit their provider, names must be words of the text).
// the server's own bounds are ws-handlers.ts handleChatOriginTag + lib/pending-origins.ts + lib/message-emote-refs.ts
use serde_json::{Map, Value, json};

use super::hs_ws::Vouch;

const MAX_TEXT: usize = 500;
const MAX_REFS: usize = 40;
// pending-origins.ts drops refs whose json is past this, whole
const MAX_REFS_JSON: usize = 4096;

// the server's channel check: ^[a-z0-9_/-]{1,64}$ (case-insensitive)
fn channel_ok(c: &str) -> bool {
  (1..=64).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_alphanumeric() || b"_/-".contains(&b))
}

// the server's sendId check: ^[A-Za-z0-9-]{1,64}$
fn send_id_ok(s: &str) -> bool {
  (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn all(s: &str, ok: impl Fn(u8) -> bool) -> bool {
  s.bytes().all(ok)
}

// (provider, id) from the provider's own cdn path, in the id shapes the server accepts (message-emote-refs.ts ID_RE)
fn ref_of(url: &str) -> Option<(&'static str, &str)> {
  let word = |rest: &'static str| url.strip_prefix(rest).and_then(|r| r.split('/').next());
  if let Some(id) = word("https://cdn.7tv.app/emote/") {
    let crockford = |b: u8| b.is_ascii_digit() || (b.is_ascii_alphabetic() && !b"ILOUilou".contains(&b));
    let ok = (id.len() == 26 && all(id, crockford)) || (id.len() == 24 && all(id, |b| b.is_ascii_hexdigit()));
    return ok.then_some(("7tv", id));
  }
  if let Some(id) = word("https://cdn.betterttv.net/emote/") {
    return (id.len() == 24 && all(id, |b| b.is_ascii_hexdigit())).then_some(("bttv", id));
  }
  let id = word("https://cdn.frankerfacez.com/emote/").or_else(|| word("https://cdn.frankerfacez.com/emoticon/"))?;
  ((1..=9).contains(&id.len()) && all(id, |b| b.is_ascii_digit())).then_some(("ffz", id))
}

// the line's own words that are picked emotes, as `{name: {p, id}}`; only a pick that is a word of `text`
fn refs(text: &str, picks: &[Vouch]) -> Map<String, Value> {
  let mut out = Map::new();
  for v in picks {
    if out.len() >= MAX_REFS || !text.split_whitespace().any(|w| w == &*v.name) {
      continue;
    }
    if let Some((p, id)) = ref_of(&v.url) {
      out.insert(v.name.to_string(), json!({"p": p, "id": id}));
    }
  }
  // a set the server would drop whole is not worth sending
  while serde_json::to_string(&out).map_or(0, |s| s.len()) > MAX_REFS_JSON {
    let Some(last) = out.keys().next_back().cloned() else { break };
    out.remove(&last);
  }
  out
}

// None = nothing the server would take (it ignores a bad frame without a word, so we don't send it)
pub fn frame(chan: &str, text: &str, send_id: &str, picks: &[Vouch]) -> Option<Value> {
  if text.is_empty() || text.len() > MAX_TEXT || !channel_ok(chan) || !send_id_ok(send_id) {
    return None;
  }
  let mut f = json!({"type": "chat:origin_tag", "text": text, "channel": chan, "sendId": send_id});
  let r = refs(text, picks);
  if !r.is_empty() {
    f["refs"] = Value::Object(r);
  }
  Some(f)
}

#[cfg(test)]
mod tests {
  use super::*;

  const ID7: &str = "01GA1QJ0Y0000BDEF2V3M7Q4TP";

  fn pick(name: &str, url: &str) -> Vouch {
    Vouch { name: name.into(), url: url.into(), provider: "7tv" }
  }

  #[test]
  fn the_frame_has_the_sites_shape_and_refs_only_for_words_in_the_line() {
    let url = format!("https://cdn.7tv.app/emote/{ID7}/1x.webp");
    let picks = [pick("catJAM", &url), pick("gone", &url)];
    let f = frame("forsen", "hi catJAM", "hsc-1", &picks).unwrap();
    assert_eq!(
      f,
      json!({"type": "chat:origin_tag", "text": "hi catJAM", "channel": "forsen", "sendId": "hsc-1",
             "refs": {"catJAM": {"p": "7tv", "id": ID7}}})
    );
    let bare = frame("forsen", "hi", "hsc-1", &picks).unwrap();
    assert!(bare.get("refs").is_none(), "no pick in the line, no refs key");
    let part = frame("forsen", "catJAMs", "hsc-1", &picks).unwrap();
    assert!(part.get("refs").is_none(), "a pick is a whole word");
  }

  #[test]
  fn ids_are_read_from_the_cdn_path_in_the_servers_grammar() {
    let hex = "a".repeat(24);
    assert_eq!(ref_of(&format!("https://cdn.betterttv.net/emote/{hex}/1x.webp")), Some(("bttv", hex.as_str())));
    assert_eq!(ref_of("https://cdn.frankerfacez.com/emoticon/123456/1"), Some(("ffz", "123456")));
    assert_eq!(ref_of("https://cdn.frankerfacez.com/emote/1234567890/1"), None, "ffz ids stop at 9 digits");
    assert_eq!(ref_of(&format!("https://cdn.7tv.app/emote/{hex}/1x.webp")), Some(("7tv", hex.as_str())));
    assert_eq!(ref_of("https://cdn.7tv.app/emote/short/1x.webp"), None);
    assert_eq!(ref_of(&format!("https://evil.example/emote/{hex}/1x")), None, "only the provider's own host");
    assert_eq!(ref_of(&format!("http://cdn.betterttv.net/emote/{hex}/1x")), None);
  }

  #[test]
  fn what_the_server_would_reject_is_not_sent() {
    assert!(frame("forsen", "", "hsc-1", &[]).is_none());
    assert!(frame("forsen", &"a".repeat(501), "hsc-1", &[]).is_none());
    assert!(frame("forsen", &"a".repeat(500), "hsc-1", &[]).is_some());
    assert!(frame("", "hi", "hsc-1", &[]).is_none());
    assert!(frame(&"a".repeat(65), "hi", "hsc-1", &[]).is_none());
    assert!(frame("@yt.handle", "hi", "hsc-1", &[]).is_none(), "a youtube handle is no channel to the server");
    assert!(frame("forsen", "hi", "bad id", &[]).is_none());
    assert!(frame("forsen", "hi", &"a".repeat(65), &[]).is_none());
  }

  #[test]
  fn refs_are_capped_in_count_and_size() {
    let picks: Vec<Vouch> =
      (0..60).map(|i| pick(&format!("e{i}"), &format!("https://cdn.7tv.app/emote/{ID7}/1x.webp"))).collect();
    let text = picks.iter().map(|p| &*p.name).collect::<Vec<_>>().join(" ");
    let f = frame("forsen", &text, "hsc-1", &picks).unwrap();
    let n = f["refs"].as_object().unwrap().len();
    assert!(n <= MAX_REFS, "{n}");
    assert!(serde_json::to_string(&f["refs"]).unwrap().len() <= MAX_REFS_JSON);
  }
}
