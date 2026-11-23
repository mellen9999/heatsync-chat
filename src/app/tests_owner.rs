// the whole app headless, the streamer's own heatsync emotes: asked for once per channel a tab is, the
// room's first answer on that platform's lines, in the box, in tab and in the picker, read again when the
// streamer changes them, and gone with the tab

use super::*;
use crate::emotes::{Emote, Leg, Provider, Set};
use crate::model::chan::Spec;
use crate::model::message::{Kind as MsgKind, Message};
use crate::model::picker::Sec;
use crate::net::Cmd;
use crate::net::auth::Me;
use crate::net::irc::Chat;

fn url7(id: &str) -> String {
  format!("https://cdn.7tv.app/emote/{id}/1x.webp")
}

fn set(emotes: &[(&str, &str)]) -> Set {
  let e = |id: &str| Emote { url: url7(id).into(), w: 28, h: 28, zero_width: false };
  emotes.iter().map(|(n, id)| ((*n).into(), e(id))).collect()
}

fn leg(p: Platform, k: &str) -> Leg {
  (p, k.into())
}

// what the network was asked about streamers' sets since the last look, in a fixed order
fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(Leg, bool)> {
  let all = std::iter::from_fn(|| rx.try_recv().ok());
  let mut v: Vec<(Leg, bool)> =
    all.filter_map(|c| if let Cmd::Owner(l, on) = c { Some((l, on)) } else { None }).collect();
  v.sort_by(|a, b| (a.0.0.name(), &a.0.1).cmp(&(b.0.0.name(), &b.0.1)));
  v
}

fn boot(tabs: &[&str]) -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, rx) = Net::tap();
  let mut app = App::new(net, tabs.iter().map(|t| (*t).into()).collect(), false);
  app.me = Some(Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: Some(1),
    names: vec!["mellen".into()],
    kick: Some("mellenk".into()),
    youtube: None,
    packs: vec![],
  });
  (app, rx)
}

#[test]
fn a_tab_asks_for_each_of_its_channels_streamers_sets_once_and_stops_when_the_channel_leaves() {
  let (mut app, mut rx) = boot(&["forsen kick:xqc yt:@abc", "lirik"]);
  let (tw, lirik) = (leg(Platform::Twitch, "forsen"), leg(Platform::Twitch, "lirik"));
  let (kick, yt) = (leg(Platform::Kick, "xqc"), leg(Platform::Youtube, "@abc"));
  assert_eq!(
    asked(&mut rx),
    [(kick.clone(), true), (tw.clone(), true), (lirik.clone(), true), (yt.clone(), true)],
    "every channel of every tab, as the app opens"
  );
  // a new tab: its channel alone
  app.add_tab(&Spec::from_line("kick:trainwreckstv").unwrap());
  assert_eq!(asked(&mut rx), [(leg(Platform::Kick, "trainwreckstv"), true)]);
  // a closed tab's channel is let go, and an answer that comes after is nobody's
  let i = app.tabs.iter().position(|t| &*t.view.channel == "lirik").unwrap();
  app.remove_tab(i);
  assert_eq!(asked(&mut rx), [(lirik.clone(), false)]);
  app.on_net(NetEvent::OwnerEmotes(lirik, set(&[("W", "LATE")])));
  assert!(app.res.emotes.owner_on("lirik", Platform::Twitch, "W").is_none());
  // a leg taken off a tab goes the same way; the legs that stay are not asked again
  let i = app.tabs.iter().position(|t| &*t.view.channel == "forsen").unwrap();
  app.edit_tab(i, &Spec::from_line("forsen yt:@abc").unwrap());
  assert_eq!(asked(&mut rx), [(kick, false)]);
}

#[test]
fn the_streamers_emote_is_the_rooms_in_the_row_the_box_tab_and_the_picker() {
  let (mut app, _rx) = boot(&["forsen kick:xqc yt:@abc"]);
  app.res.emotes.set(Some("forsen"), Provider::Ffz, set(&[("W", "CHANFFZ")]));
  app.res.emotes.set(None, Provider::SevenTv, set(&[("W", "GLOBAL7")]));
  let mine = serde_json::json!([{"custom_name": "W", "url": url7("MINE"), "width": 32, "height": 32,
    "added_at": "2026-10-30T22:26:51.099Z"}]);
  app.res.senders.lookup(1, "W", 0);
  app.res.senders.put(1, crate::heatsync::senders::parse_set(&mine));
  app.on_net(NetEvent::OwnerEmotes(leg(Platform::Twitch, "forsen"), set(&[("W", "OWNER"), ("Only", "ONLY")])));
  app.on_net(NetEvent::OwnerEmotes(leg(Platform::Kick, "xqc"), set(&[("W", "KICKOWNER")])));
  let row = |app: &mut App, p: Platform, word: &str| {
    let mut m = Message::system(now_ms(), p, word);
    (m.kind, m.user_id, m.login) = (MsgKind::Chat, "7".into(), "bob".into());
    let n = word.len();
    crate::emotes::resolve::find_word(&mut app.res.sources(), "forsen", &m, 0, n, word).map(|f| f.0.to_string())
  };
  // a line reads its own platform's streamer: over the channel's ffz, 7tv and bttv, and nobody else's
  assert_eq!(row(&mut app, Platform::Twitch, "W"), Some(url7("OWNER")));
  assert_eq!(row(&mut app, Platform::Twitch, "Only"), Some(url7("ONLY")), "a name only the streamer has");
  assert_eq!(row(&mut app, Platform::Kick, "W"), Some(url7("KICKOWNER")));
  assert_eq!(row(&mut app, Platform::Kick, "Only"), None, "the twitch streamer's emote, a kick line");
  assert_eq!(row(&mut app, Platform::Youtube, "W"), Some(url7("CHANFFZ")), "no youtube streamer's set here");
  // the box: a typed word is the room's, and the room's first answer is the streamer's (never your own W)
  assert_eq!(app.box_url("W").as_deref(), Some(&*url7("OWNER")));
  assert_eq!(app.box_url("Only").as_deref(), Some(&*url7("ONLY")));
  // tab: the streamer's W leads the copies of the name, and says whose it is
  let c = app.candidates("W", false);
  let urls: Vec<&str> = c.iter().filter_map(|p| p.pic.as_ref().map(|r| &*r.url)).collect();
  assert_eq!(urls[..2], [&*url7("OWNER"), &*url7("MINE")], "the room's, then yours: ffz's W lost the name here");
  assert_eq!(c[0].what, t("app.complete.channel_hs"));
  // the picker's channel section holds the streamer's emotes, one tile per name
  app.toggle_picker();
  let p = app.picker.as_ref().unwrap();
  let tiles: Vec<(&str, &str)> =
    (0..p.len()).filter_map(|i| p.item(i)).filter(|i| i.sec == Sec::Channel).map(|i| (&*i.name, &*i.url)).collect();
  assert_eq!(tiles, [("Only", &*url7("ONLY")), ("W", &*url7("OWNER"))]);
  app.close_picker();
  // heatsync emotes switched off: nothing that came through heatsync draws, the streamer's included
  app.res.look.inventory_emotes = false;
  assert_eq!(row(&mut app, Platform::Twitch, "W"), Some(url7("CHANFFZ")));
  assert_eq!(row(&mut app, Platform::Twitch, "Only"), None);
  assert_eq!(app.box_url("W").as_deref(), Some(&*url7("CHANFFZ")));
  assert!(app.candidates("Only", false).iter().all(|p| !p.emote));
}

#[test]
fn a_streamer_changing_their_emotes_reads_the_set_again() {
  let (mut app, mut rx) = boot(&["forsen kick:xqc yt:@abc"]);
  app.on_net(NetEvent::Chat("forsen".into(), Chat::Room { id: "22484632".into(), modes: Default::default() }));
  asked(&mut rx);
  let mut stale = |app: &mut App, ids: Vec<u64>, chans: Vec<Leg>| {
    app.on_net(NetEvent::SendersStale { ids, chans, ver: None, removed: None });
    asked(&mut rx)
  };
  assert_eq!(stale(&mut app, vec![22484632], vec![]), [(leg(Platform::Twitch, "forsen"), true)]);
  // kick and youtube accounts come by name, as the server spells them: lowercase, no `@`
  assert_eq!(stale(&mut app, vec![], vec![leg(Platform::Kick, "xqc")]), [(leg(Platform::Kick, "xqc"), true)]);
  assert_eq!(stale(&mut app, vec![], vec![leg(Platform::Youtube, "abc")]), [(leg(Platform::Youtube, "@abc"), true)]);
  // someone who is no channel here: nothing is read
  assert_eq!(stale(&mut app, vec![5], vec![leg(Platform::Kick, "forsen"), leg(Platform::Youtube, "xqc")]), []);
}
