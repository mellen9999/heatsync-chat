// the frames the socket reads past the main match in hs_ws.rs: news the site and the ext take live that
// this client used to drop on the floor, and the count of every type nothing here reads at all
use serde_json::Value;

use super::{End, Out, State};

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

pub(super) fn more(ty: &str, _v: &Value, st: &mut State, _out: &Out) -> Option<End> {
  unknown(st, ty);
  None
}
