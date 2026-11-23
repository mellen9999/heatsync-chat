// a streamer's own heatsync emotes (the site's owner route), one read per channel a tab is: the room's
// first answer for a name on that platform's lines (emotes/resolve.rs, emotes/draft.rs). asked beside the
// channel's other emotes and never waited on: a room without the answer is the room every other app shows.
// no content category is opted in, so the server leaves every flagged emote out (as the sender batch does)
use std::time::Duration;

use super::{NetEvent, ORIGIN, Out, retry};
use crate::emotes::Leg;
use crate::emotes::providers::fetch;
use crate::heatsync::senders::parse_owner;
use crate::model::ident;
use crate::model::message::Platform;

// a set read again because it changed waits this long first: a streamer filling slots one after another
// costs one read, not one per emote
const SETTLE: Duration = Duration::from_secs(2);

// the `Open` key of a channel's read: no twitch login and no `kick:` room can spell it
fn key(leg: &Leg) -> String {
  format!("owner:{}:{}", leg.0.name(), leg.1)
}

// the route's address for a channel. None for a key that is no channel name: it goes into a path
fn url(origin: &str, (p, k): &Leg) -> Option<String> {
  let ok = match p {
    Platform::Twitch => ident::twitch_room(k),
    Platform::Kick | Platform::Youtube => ident::relay_key(k) && k.bytes().any(|b| b.is_ascii_alphanumeric()),
  };
  ok.then(|| format!("{origin}/api/emotes/user/{k}?platform={}", p.name()))
}

// `on`: read the channel's set, until it arrives or no tab is the channel any more. asked again (the
// streamer changed it), the newer read takes over from the older. off: stop asking
pub fn ask(http: &reqwest::Client, open: &retry::Open, out: &Out, leg: Leg, on: bool) {
  let key = key(&leg);
  if !on {
    return open.part(&key);
  }
  let Some(url) = url(ORIGIN, &leg) else { return };
  let again = open.watch(&key).is_some();
  let (http, tab, to) = (http.clone(), open.join(&key), out.clone());
  let read = async move {
    if again {
      tokio::time::sleep(SETTLE).await;
    }
    let what = format!("{}'s heatsync emotes", leg.1);
    if let Some(v) = retry::until(&to, &what, || tab.alive(), || fetch(&http, &url)).await {
      to.send(NetEvent::OwnerEmotes(leg, parse_owner(&v, ORIGIN)));
    }
  };
  super::watch("streamer emotes", read, out.clone());
}

// the channels a changed account's keys name outright (`emote:added|removed`, `senderKeys`): a kick name,
// a youtube handle or channel id. a twitch id is matched to its room by the app, which knows the ids
pub fn stale(v: &serde_json::Value) -> Vec<Leg> {
  let keys = v.get("senderKeys").and_then(serde_json::Value::as_array).into_iter().flatten();
  let leg = |k: &str| {
    let (p, name) = k.split_once(':')?;
    let p = match p {
      "kick" => Platform::Kick,
      "yt" | "ytc" => Platform::Youtube,
      _ => return None,
    };
    ident::relay_key(name).then(|| (p, name.into()))
  };
  keys.take(30).filter_map(|k| leg(k.as_str()?)).collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn the_address_names_the_channel_and_its_platform_and_nothing_else() {
    let at = |p, k: &str| url("https://h", &(p, k.into()));
    assert_eq!(at(Platform::Twitch, "forsen").unwrap(), "https://h/api/emotes/user/forsen?platform=twitch");
    assert_eq!(at(Platform::Kick, "x-qc").unwrap(), "https://h/api/emotes/user/x-qc?platform=kick");
    assert_eq!(at(Platform::Youtube, "@Some.One").unwrap(), "https://h/api/emotes/user/@Some.One?platform=youtube");
    for bad in ["", "..", ".", "a/b", "a?b", "a b", "a#b", "a%2f"] {
      assert!(at(Platform::Twitch, bad).is_none() && at(Platform::Kick, bad).is_none(), "{bad}");
      assert!(at(Platform::Youtube, bad).is_none(), "{bad}");
    }
  }

  #[test]
  fn a_changed_accounts_keys_name_its_kick_and_youtube_channels() {
    let v =
      json!({"senderKeys": ["twitch:12", "kick:99", "kick:xqc", "yt:some.one", "ytc:UCabc", "x:y", "kick:a/b", 5]});
    let leg = |p, k: &str| -> Leg { (p, k.into()) };
    assert_eq!(
      stale(&v),
      [
        leg(Platform::Kick, "99"),
        leg(Platform::Kick, "xqc"),
        leg(Platform::Youtube, "some.one"),
        leg(Platform::Youtube, "UCabc")
      ]
    );
    assert!(stale(&json!({})).is_empty());
  }
}
