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
