// a twitch channel's 7tv, bttv and ffz emotes: one retrying task per provider, and heatsync's copy of what
// each last said standing in while one is down. a kick or youtube channel's: heatsync's read of them
use tokio::sync::mpsc;

use super::{NetEvent, ORIGIN, Out, provider_name, retry, seventv};
use crate::badges;
use crate::emotes::{Leg, Provider, Set, providers};
use crate::model::ident;
use crate::model::message::Platform;

// the providers that know a channel on each platform (the server asks the same ones): 7tv by a kick
// channel's kick id, 7tv and bttv by a youtube channel's id. ffz is twitch only
const KICK: [Provider; 1] = [Provider::SevenTv];
const YOUTUBE: [Provider; 2] = [Provider::SevenTv, Provider::Bttv];

// the `Open` key of a channel's read: no twitch login and no `owner:` read can spell it
fn leg_key(leg: &Leg) -> String {
  format!("emotes:{}:{}", leg.0.name(), leg.1)
}

// heatsync's read of a kick or youtube channel's third-party emotes, and the providers it asks. the ids
// 7tv and bttv know such a channel by are heatsync's to look up: a kick channel goes by its address, a
// youtube one by its channel id only (a tab opened by @handle has none to give). None = nothing to ask
fn leg_url(origin: &str, (p, k): &Leg) -> Option<(String, &'static [Provider])> {
  let (ok, asked): (bool, &[Provider]) = match p {
    Platform::Kick => (ident::kick_slug(k), &KICK),
    Platform::Youtube => (ident::yt_channel_id(k), &YOUTUBE),
    Platform::Twitch => (false, &[]),
  };
  ok.then(|| (format!("{origin}/api/channel/{k}/emotes?platform={}", p.name()), asked))
}

// heatsync could not reach a provider for this answer: `degraded` (it holds a copy, or nothing), `missing`
// (the providers it has nothing for). not a whole answer: an empty list here is not "the channel has none"
fn held(v: &serde_json::Value) -> bool {
  let missing = v.get("missing").and_then(serde_json::Value::as_array).is_some_and(|m| !m.is_empty());
  missing || v.get("degraded").and_then(serde_json::Value::as_bool) == Some(true)
}

// `on`: read a kick or youtube channel's 7tv and bttv emotes, until they arrive or no tab is the channel
// any more; asked again, the newer read takes over. off: stop asking. a kick line draws its own channel's
// 7tv emotes from this, as kick viewers with 7tv see them; before it, a kick or youtube room had none
pub(super) fn leg(http: &reqwest::Client, open: &retry::Open, out: &Out, leg: Leg, on: bool) {
  let key = leg_key(&leg);
  if !on {
    return open.part(&key);
  }
  let Some((url, asked)) = leg_url(ORIGIN, &leg) else { return };
  let (http, tab, to) = (http.clone(), open.join(&key), out.clone());
  let read = async move {
    let what = format!("{}'s {} emotes", leg.1, leg.0.name());
    let ask = || async {
      let v = providers::fetch(&http, &url).await?;
      let sets: Vec<(Provider, Set)> = asked.iter().map(|p| (*p, providers::kept(*p, &v, ORIGIN))).collect();
      if !held(&v) {
        return Ok(sets);
      }
      // a provider was not answering heatsync: its copy of what that one last said goes up now (nothing,
      // where it has none: what is drawn stays), and heatsync is asked again until the answer is whole
      let have: Vec<(Provider, Set)> = sets.into_iter().filter(|s| !s.1.is_empty()).collect();
      if !have.is_empty() && tab.alive() {
        to.send(NetEvent::LegEmotes(leg.clone(), have));
      }
      Err("a provider was not answering heatsync".to_owned())
    };
    if let Some(sets) = retry::until(&to, &what, || tab.alive(), ask).await {
      to.send(NetEvent::LegEmotes(leg.clone(), sets));
    }
  };
  super::watch("channel emotes", read, out.clone());
}

// one provider's emotes for a channel out of heatsync's copy. None when there is nothing to stand in: the
// copy did not answer, or holds none of that provider's (a channel nobody opened on heatsync has no copy)
async fn kept_emotes(http: &reqwest::Client, ch: &str, p: Provider) -> Option<Set> {
  let v = providers::fetch(http, &providers::kept_url(ORIGIN, ch)?).await.ok()?;
  Some(providers::kept(p, &v, ORIGIN)).filter(|s| !s.is_empty())
}

// a channel's emote sets, one retrying task per provider so a down one doesn't hold up the others.
// they stop when the tab closes
pub(super) async fn channel_emotes(
  http: reqwest::Client,
  ch: Box<str>,
  room: Box<str>,
  stv: mpsc::UnboundedSender<seventv::Cmd>,
  open: retry::Open,
  out: Out,
) {
  if !room.bytes().all(|b| b.is_ascii_digit()) {
    return;
  }
  for (p, u) in providers::channel_urls(&room) {
    let Some(tab) = open.watch(&ch) else { return };
    let (http, ch, room, stv, out) = (http.clone(), ch.clone(), room.clone(), stv.clone(), out.clone());
    tokio::spawn(async move {
      let what = format!("{ch}'s {} emotes", provider_name(p));
      // a provider that is not answering: heatsync's copy of what it last said here stands in, so the room
      // keeps its emotes through the outage. the provider is asked until it answers (the status line says
      // so), and its own answer replaces the copy
      let stand_in = || async {
        if let Some(set) = kept_emotes(&http, &ch, p).await
          && tab.alive()
        {
          out.send(NetEvent::Emotes(Some(ch.clone()), p, set));
        }
      };
      let Some(v) = retry::until_or(&out, &what, || tab.alive(), stand_in, || providers::fetch(&http, &u)).await else {
        return;
      };
      if p == Provider::SevenTv
        && let Some(set) = providers::seventv_set_id(&v)
      {
        // live edits for this channel's set; the broadcaster's own cosmetics too
        let user = providers::seventv_user_id(&v).zip(room.parse().ok());
        let _ = stv.send(seventv::Cmd::Watch { ch: ch.clone(), set, user });
      }
      if p == Provider::Ffz {
        out.send(NetEvent::FfzRoomBadges(ch.clone(), badges::fetch::ffz_room(&v)));
      }
      out.send(NetEvent::Emotes(Some(ch), p, providers::parse_channel(p, &v)));
    });
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_kick_or_youtube_channels_read_names_the_channel_its_platform_and_who_is_asked() {
    let at = |p, k: &str| leg_url("https://h", &(p, k.into()));
    let (url, asked) = at(Platform::Kick, "x-qc").unwrap();
    assert_eq!((url.as_str(), asked), ("https://h/api/channel/x-qc/emotes?platform=kick", &[Provider::SevenTv][..]));
    let uc = "UCabcdefghijklmnopqrstuv";
    let (url, asked) = at(Platform::Youtube, uc).unwrap();
    assert_eq!(url, format!("https://h/api/channel/{uc}/emotes?platform=youtube"));
    assert_eq!(asked, [Provider::SevenTv, Provider::Bttv]);
    // a youtube tab opened by @handle has no channel id to ask by; a twitch channel asks the providers
    assert!(at(Platform::Youtube, "@some.one").is_none() && at(Platform::Twitch, "forsen").is_none());
    for bad in ["", "..", "a/b", "a?b", "a b", "a#b", "a%2f", "XQC"] {
      assert!(at(Platform::Kick, bad).is_none() && at(Platform::Youtube, bad).is_none(), "{bad}");
    }
    assert_ne!(leg_key(&(Platform::Kick, "xqc".into())), leg_key(&(Platform::Youtube, "xqc".into())));
  }

  // 11-23: heatsync answered a kick channel with no emotes and `degraded` while 7tv timed out, and 969 a
  // moment later. an answer like that is asked again, never taken as "this channel has none"
  #[test]
  fn an_answer_heatsync_could_not_complete_is_not_the_channels_emotes() {
    use serde_json::json;
    assert!(held(&json!({"emotes": [], "degraded": true})));
    assert!(held(&json!({"emotes": [], "missing": ["7tv"]})));
    assert!(!held(&json!({"emotes": [], "count": 0})), "a channel with no 7tv emotes: a whole answer");
    assert!(!held(&json!({"emotes": [], "missing": [], "degraded": false})) && !held(&serde_json::Value::Null));
  }
}
