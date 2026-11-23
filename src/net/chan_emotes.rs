// a twitch channel's 7tv, bttv and ffz emotes: one retrying task per provider, and heatsync's copy of what
// each last said standing in while one is down
use tokio::sync::mpsc;

use super::{NetEvent, ORIGIN, Out, provider_name, retry, seventv};
use crate::badges;
use crate::emotes::{Provider, Set, providers};

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
