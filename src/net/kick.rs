// a joined kick channel's own art and emotes, fetched once it is joined
use super::{NetEvent, Out, retry};
use crate::badges;
use crate::emotes::providers;

// a kick channel's subscriber art, a few tries (a channel with no art of its own answers fine and
// sends nothing; lines then wear the "sub" word, like the ext)
pub async fn sub_art(http: reqwest::Client, slug: Box<str>, out: Out) {
  let what = format!("{slug}'s kick badges");
  if let Some(art) = retry::until_n(&out, &what, 3, || true, || badges::fetch::kick_channel(&http, &slug)).await
    && !art.is_empty()
  {
    out.send(NetEvent::KickBadges(slug, art));
  }
}

// a kick channel's emotes, until they arrive or no tab merges the channel any more
pub async fn emotes(http: reqwest::Client, slug: Box<str>, tab: retry::Watch, out: Out) {
  let (what, url) = (format!("{slug}'s kick emotes"), providers::kick_url(&slug));
  if let Some(v) = retry::until(&out, &what, || tab.alive(), || providers::fetch(&http, &url)).await {
    out.send(NetEvent::KickEmotes(slug, providers::kick(&v)));
  }
}
