// the frames the socket reads past the main match in hs_ws.rs: news the site and the ext take live that
// this client used to drop on the floor, and the count of every type nothing here reads at all
use serde_json::Value;

use super::{End, Out, State};
use crate::heatsync::valid_name;
use crate::images::url::allowed;
use crate::model::ident;
use crate::net::NetEvent;

// distinct unknown types kept: a hostile socket must not grow the table without end
pub(super) const MAX_UNKNOWN: usize = 64;

// the most feed rooms (beyond the thread) a socket holds: `new` and the logs of the person on screen
const MAX_FEEDS: usize = 8;

// a feed room the server takes (ws-handlers validateFeedKey): `new`, or `ulog:<platform>:<login>` for one person's
// archive lines. the login is lowercase `[a-z0-9_]`, or a youtube `@handle`
pub(crate) fn feed_key(key: &str) -> Option<Box<str>> {
  let key = key.to_ascii_lowercase();
  if key == "new" {
    return Some(key.into());
  }
  let (platform, login) = key.strip_prefix("ulog:")?.split_once(':')?;
  let ok = match (platform, login.strip_prefix('@')) {
    ("youtube", Some(h)) => {
      (3..=30).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    }
    ("twitch" | "kick", None) => {
      (1..=40).contains(&login.len()) && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    }
    _ => false,
  };
  ok.then(|| key.into())
}

// a feed room asked for or let go: the frame to send now (none when it already is as asked)
pub(super) fn feed_cmd(st: &mut State, key: Box<str>, on: bool) -> Vec<Value> {
  let Some(key) = feed_key(&key) else { return Vec::new() };
  let changed = if on { st.feeds.len() < MAX_FEEDS && st.feeds.insert(key.clone()) } else { st.feeds.remove(&key) };
  let ty = if on { "feed:join" } else { "feed:leave" };
  if changed { vec![serde_json::json!({"type": ty, "feed": &*key})] } else { Vec::new() }
}

// a frame with no arm: counted by type, and named on stderr the first time it is seen, so a missing arm shows
// up in a log without one line per frame
pub(super) fn unknown(st: &mut State, ty: &str) {
  if let Some(n) = st.unknown.get_mut(ty) {
    *n = n.saturating_add(1);
  } else if st.unknown.len() < MAX_UNKNOWN {
    eprintln!("heatsync socket: no arm for frame type {:?}", ty.chars().take(64).collect::<String>());
    st.unknown.insert(ty.into(), 1);
  }
}

pub(super) fn more(ty: &str, v: &Value, st: &mut State, out: &Out) -> Option<End> {
  let send = |ev: NetEvent| (!out.send(ev)).then_some(End::Quit);
  match ty {
    // someone in a room we are in used a heatsync emote of theirs (`username`, `emoteName`, `emoteData`): their
    // set is read again at once, so the word draws as the picture now and not when the set ages out. the
    // frame itself is never trusted as the emote: ownership comes from their set (emote rule: the sender owns it)
    "emote:broadcast" => {
      let login: Box<str> =
        v.get("username").and_then(Value::as_str).filter(|u| ident::relay_key(u))?.to_lowercase().into();
      v.get("emoteName").and_then(Value::as_str).filter(|n| valid_name(n))?;
      v.pointer("/emoteData/url").and_then(Value::as_str).filter(|u| allowed(u))?;
      send(NetEvent::SenderUsed(login))
    }
    _ => {
      unknown(st, ty);
      None
    }
  }
}
