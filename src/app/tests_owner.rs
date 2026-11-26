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
  // a youtube line reads youtube's channel only, never the twitch channel's ffz: none here, so the global
  assert_eq!(row(&mut app, Platform::Youtube, "W"), Some(url7("GLOBAL7")));
  // the box: a typed word is the room's, and the room's first answer is the streamer's (never your own W)
  assert_eq!(app.box_url("W").as_deref(), Some(&*url7("OWNER")));
  assert_eq!(app.box_url("Only").as_deref(), Some(&*url7("ONLY")));
  // tab: the streamer's W is the one W, and says whose it is: ffz's and yours lost the name here
  let c = app.candidates("W", false);
  let urls: Vec<&str> = c.iter().filter_map(|p| p.pic.as_ref().map(|r| &*r.url)).collect();
  assert_eq!(urls, [&*url7("OWNER")], "the room's: the other pictures of W are not offered");
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

// a streamer who changed their emotes while the heatsync socket was down told nobody here. when it comes
// back every set already read is read again; one still being asked for the first time is left to that ask
#[test]
fn the_socket_coming_back_reads_the_sets_it_has_again() {
  let (mut app, mut rx) = boot(&["forsen kick:xqc", "lirik"]);
  asked(&mut rx);
  app.on_net(NetEvent::HsUp);
  assert_eq!(asked(&mut rx), [], "nothing has answered yet: the first asks are still out");
  app.on_net(NetEvent::OwnerEmotes(leg(Platform::Twitch, "forsen"), set(&[("W", "OWNER")])));
  app.on_net(NetEvent::OwnerEmotes(leg(Platform::Kick, "xqc"), Set::new()));
  app.on_net(NetEvent::HsUp);
  assert_eq!(
    asked(&mut rx),
    [(leg(Platform::Kick, "xqc"), true), (leg(Platform::Twitch, "forsen"), true)],
    "the two that answered, an empty set included; lirik's first ask is still its own"
  );
}

// what the network was asked about kick and youtube channels' 7tv and bttv emotes since the last look
fn rooms_asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<(Leg, bool)> {
  let all = std::iter::from_fn(|| rx.try_recv().ok());
  let mut v: Vec<(Leg, bool)> =
    all.filter_map(|c| if let Cmd::LegEmotes(l, on) = c { Some((l, on)) } else { None }).collect();
  v.retain(|(l, _)| l.0 != Platform::Twitch);
  v.sort_by(|a, b| (a.0.0.name(), &a.0.1).cmp(&(b.0.0.name(), &b.0.1)));
  v
}

// a kick channel has its own 7tv emotes (a youtube one 7tv and bttv). the app asked for none, so a kick line
// in a twitch + kick tab drew the twitch channel's emote of a name, and a kick-only tab drew no 7tv at all
#[test]
fn a_kick_line_draws_its_own_channels_7tv_emotes_and_never_the_twitch_channels() {
  let (mut app, mut rx) = boot(&["forsen kick:xqc", "kick:trainwreckstv"]);
  let (xqc, train) = (leg(Platform::Kick, "xqc"), leg(Platform::Kick, "trainwreckstv"));
  assert_eq!(rooms_asked(&mut rx), [(train.clone(), true), (xqc.clone(), true)], "each kick channel, as the app opens");
  app.res.emotes.set(Some("forsen"), Provider::SevenTv, set(&[("KW", "TWITCH7"), ("TwOnly", "TWONLY")]));
  let row = |app: &mut App, room: &str, p: Platform, word: &str| {
    let mut m = Message::system(now_ms(), p, word);
    (m.kind, m.user_id, m.login) = (MsgKind::Chat, "7".into(), "bob".into());
    crate::emotes::resolve::find_word(&mut app.res.sources(), room, &m, 0, word.len(), word).map(|f| f.0.to_string())
  };
  assert_eq!(row(&mut app, "forsen", Platform::Kick, "KW"), None, "the kick channel has not answered: a word");
  app.on_net(NetEvent::LegEmotes(
    xqc.clone(),
    vec![(Provider::SevenTv, set(&[("KW", "KICK7"), ("KickOnly", "KONLY")]))],
  ));
  assert_eq!(row(&mut app, "forsen", Platform::Kick, "KW"), Some(url7("KICK7")));
  assert_eq!(row(&mut app, "forsen", Platform::Kick, "TwOnly"), None, "the twitch channel's emote, a kick line");
  assert_eq!(row(&mut app, "forsen", Platform::Twitch, "KW"), Some(url7("TWITCH7")));
  assert_eq!(row(&mut app, "forsen", Platform::Twitch, "KickOnly"), None, "the kick channel's emote, a twitch line");
  // a kick-only tab: the box, tab and the picker read the kick channel
  app.on_net(NetEvent::LegEmotes(train.clone(), vec![(Provider::SevenTv, set(&[("TrainW", "TRAIN7")]))]));
  app.active = app.tabs.iter().position(|t| t.sources.contains(&train)).unwrap();
  assert_eq!(app.box_url("TrainW").as_deref(), Some(&*url7("TRAIN7")));
  assert!(app.candidates("TrainW", false).iter().any(|p| p.pic.as_ref().is_some_and(|r| *r.url == *url7("TRAIN7"))));
  app.toggle_picker();
  let p = app.picker.as_ref().unwrap();
  let tiles: Vec<&str> =
    (0..p.len()).filter_map(|i| p.item(i)).filter(|i| i.sec == Sec::Channel).map(|i| &*i.name).collect();
  assert_eq!(tiles, ["TrainW"]);
  app.close_picker();
  // nothing pushes a change to these: the socket coming back reads the ones that answered again
  app.on_net(NetEvent::HsUp);
  assert_eq!(rooms_asked(&mut rx), [(train.clone(), true), (xqc.clone(), true)]);
  // a closed tab's channel is let go, and an answer that comes after is nobody's
  let i = app.active;
  app.remove_tab(i);
  assert_eq!(rooms_asked(&mut rx), [(train.clone(), false)]);
  app.on_net(NetEvent::LegEmotes(train, vec![(Provider::SevenTv, set(&[("TrainW", "LATE")]))]));
  assert!(app.res.emotes.legs_read() == [xqc]);
}

// a youtube tab opened by `@handle` has no channel id: 7tv and bttv were never asked, so its lines drew
// none of that channel's emotes. the read learns the id from the live stream, so it is asked for when
// the tab opens and again when its feed connects
#[test]
fn a_youtube_tab_opened_by_handle_asks_for_its_channels_emotes_when_its_feed_connects() {
  let uc = "UCabcdefghijklmnopqrstuv";
  let (mut app, mut rx) = boot(&["yt:@some.one", &format!("yt:{uc}")]);
  let (handle, by_id) = (leg(Platform::Youtube, "@some.one"), leg(Platform::Youtube, uc));
  assert_eq!(rooms_asked(&mut rx), [(handle.clone(), true), (by_id.clone(), true)], "both, as the app opens");
  app.on_net(NetEvent::YtStatus { key: "@some.one".into(), status: "connected".into() });
  assert_eq!(rooms_asked(&mut rx), [(handle.clone(), true)], "live now: the id can be learned");
  // a tab that has its id was asked by it already; a feed no tab shows asks nothing; not live asks nothing
  app.on_net(NetEvent::YtStatus { key: uc.into(), status: "connected".into() });
  app.on_net(NetEvent::YtStatus { key: "@no.tab".into(), status: "connected".into() });
  app.on_net(NetEvent::YtStatus { key: "@some.one".into(), status: "not-live".into() });
  assert_eq!(rooms_asked(&mut rx), []);
  // and the answer is that tab's room: a line in it draws the channel's emote
  app.on_net(NetEvent::LegEmotes(handle.clone(), vec![(Provider::SevenTv, set(&[("YtW", "YT7")]))]));
  let mut m = Message::system(now_ms(), Platform::Youtube, "YtW");
  (m.kind, m.user_id, m.login) = (MsgKind::Chat, "7".into(), "bob".into());
  let room = app.tabs.iter().find(|t| t.sources.contains(&handle)).unwrap().view.channel.clone();
  let drawn =
    crate::emotes::resolve::find_word(&mut app.res.sources(), &room, &m, 0, 3, "YtW").map(|f| f.0.to_string());
  assert_eq!(drawn, Some(url7("YT7")));
}

// a line brought back with the up arrow. a copy picked over the room's comes back with it (it re-sent the
// room's emote instead); a word that was tab-completed is the room's own emote, no choice, so brought back
// in another channel it is that channel's emote and never the first one's
#[test]
fn a_recalled_line_keeps_a_copy_you_chose_and_never_carries_one_channels_emote_into_another() {
  use crate::model::catalog::Rec;
  let (mut app, _rx) = boot(&["forsen", "lirik"]);
  app.res.emotes.set(Some("forsen"), Provider::SevenTv, set(&[("KW", "FORSEN7")]));
  app.res.emotes.set(Some("lirik"), Provider::SevenTv, set(&[("KW", "LIRIK7")]));
  let tab = |app: &App, ch: &str| app.tabs.iter().position(|t| &*t.view.channel == ch).unwrap();
  let pick = |id: &str| Rec { url: url7(id).into(), zero_width: false, provider: "7tv" };
  // tab's first answer in forsen's channel is forsen's KW
  app.active = tab(&app, "forsen");
  app.line.insert("KW");
  app.line.picks.put("KW", Some(pick("FORSEN7")));
  app.send();
  assert!(app.line.text.is_empty(), "sent");
  app.active = tab(&app, "lirik");
  app.line.older();
  assert_eq!((app.line.text.as_str(), app.line.picks.is_empty()), ("KW", true));
  assert_eq!(app.box_url("KW").as_deref(), Some(&*url7("LIRIK7")), "in lirik's channel the word is lirik's");
  app.line.newer();
  // a copy picked on purpose: not what the word sends here
  app.line.insert("KW again");
  app.line.picks.put("KW", Some(pick("CHOSEN")));
  app.send();
  app.line.older();
  assert_eq!(app.line.text, "KW again");
  assert_eq!(app.box_url("KW").as_deref(), Some(&*url7("CHOSEN")), "the line comes back as it was sent");
  // it goes where you go, as a pick does, and an edit of the word ends it
  app.active = tab(&app, "forsen");
  assert_eq!(app.box_url("KW").as_deref(), Some(&*url7("CHOSEN")));
  app.line.newer();
  assert!(app.line.picks.is_empty(), "back to an empty draft");
  assert_eq!(app.box_url("KW").as_deref(), Some(&*url7("FORSEN7")), "typed fresh, the name is the room's");
}

// an emote in the message box is an emote: a right-click acts on the picture drawn there, as on a chat
// line. the room's emote under a name you also own is blocked, your copy is never removed for it; your own
// copy, picked over the room's, is removed, and with it the pick, so the word is the room's again
#[test]
fn a_right_click_on_an_emote_in_the_box_acts_on_the_picture_it_shows() {
  use crate::model::catalog::Rec;
  use crate::model::settings::RightClick;
  use crate::net::emote_ops::EmoteCmd;
  let (mut app, mut rx) = boot(&["forsen"]);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  app.set.right_click = RightClick::Instant;
  app.res.emotes.set(Some("forsen"), Provider::SevenTv, set(&[("KW", "ROOM7")]));
  let mine = serde_json::json!([{"custom_name": "KW", "url": url7("MINE"), "width": 28, "height": 28,
    "added_at": "2026-10-30T22:26:51.099Z"}]);
  app.res.senders.lookup(1, "KW", 0);
  app.res.senders.put(1, crate::heatsync::senders::parse_set(&mine));
  let emote_cmds = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>| -> Vec<EmoteCmd> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| if let Cmd::Emote(e) = c { Some(e) } else { None })
      .collect()
  };
  // the pointer on the box's one picture, found where the box says it drew it
  let aim = |app: &mut App| {
    app.input_dirty = true;
    app.redraw();
    let r = app.input_area();
    let mid = r.y + r.h / 2;
    let hit = app.input_hit.as_ref().expect("the box remembers where its text is");
    let x = (r.x..r.right()).find(|&x| hit.chip_at(x, mid).is_some());
    if let Some(x) = x {
      app.pointer = (x + 2, mid);
    }
    x.is_some()
  };
  app.line.insert("hi KW ");
  app.line.chips.insert("KW".into());
  assert_eq!(app.box_url("KW").as_deref(), Some(&*url7("ROOM7")), "typed by hand, the name is the room's");
  assert!(aim(&mut app), "the room's KW is a picture in the box");
  let tip = app.emote_under_pointer().expect("the pointer is on an emote");
  assert_eq!((&*tip.name, &*tip.url), ("KW", &*url7("ROOM7")));
  assert!(!app.owned(&tip), "the room's picture under a name you own is not yours");
  emote_cmds(&mut rx);
  app.right_click();
  assert!(app.res.blocked.contains("KW", &url7("ROOM7")), "the picture the box showed is blocked");
  let sent = emote_cmds(&mut rx);
  assert!(sent.iter().all(|c| matches!(c, EmoteCmd::Block { .. })), "and nothing of yours was removed");
  assert_eq!(sent.len(), 1);
  assert_eq!(app.box_url("KW"), None, "a blocked picture is not drawn in the box");
  assert!(!aim(&mut app), "so there is no picture left to click");
  let scope = crate::emotes::block::Scope::Channel;
  app.res.blocked.unblock("KW", &url7("ROOM7"), scope);

  // your own copy, picked over the room's
  app.line.picks.put("KW", Some(Rec { url: url7("MINE").into(), zero_width: false, provider: "7tv" }));
  assert_eq!(app.box_url("KW").as_deref(), Some(&*url7("MINE")));
  assert!(aim(&mut app));
  let own = app.emote_under_pointer().expect("the pointer is on your copy");
  assert!(app.owned(&own));
  app.right_click();
  let sent = emote_cmds(&mut rx);
  assert!(
    matches!(&sent[..], [EmoteCmd::Remove { name }] if &**name == "KW"),
    "your copy is removed, and only that ({} sent)",
    sent.len()
  );
  assert!(app.line.picks.get("KW").is_none(), "the removed copy is no longer this draft's pick");
  assert_eq!(app.box_url("KW").as_deref(), Some(&*url7("ROOM7")), "the word is read again: the room's");
  assert!(!app.res.blocked.contains("KW", &url7("MINE")), "and nothing was blocked");

  // off the pictures, a right-click in the box is not an emote's
  let r = app.input_area();
  app.pointer = (r.right() - 4, r.y + r.h / 2);
  assert!(app.emote_under_pointer().is_none());
}

// the box is drawn again only when it is marked. a set that changes under a word in it with no keystroke
// must mark it, or the box keeps the old picture while enter ships the new one
#[test]
fn a_set_that_changes_under_a_word_in_the_box_redraws_the_box() {
  let (mut app, _rx) = boot(&["forsen"]);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  app.res.emotes.set(Some("forsen"), Provider::SevenTv, set(&[("KW", "ROOM7")]));
  app.line.insert("hi KW ");
  app.line.chips.insert("KW".into());
  app.input_dirty = true;
  app.redraw();
  assert!(!app.input_dirty, "the box is drawn");

  app.on_net(NetEvent::Emotes(Some("forsen".into()), Provider::SevenTv, set(&[("KW", "NEWER")])));
  assert!(app.input_dirty, "the room's set changed: the box is drawn again");
  assert_eq!(app.box_url("KW").as_deref(), Some(&*url7("NEWER")));
  app.redraw();

  let mine = serde_json::json!([{"custom_name": "Mine", "url": url7("MINE"), "width": 28, "height": 28,
    "added_at": "2026-10-30T22:26:51.099Z"}]);
  app.res.senders.lookup(1, "Mine", 0);
  app.on_net(NetEvent::Senders(vec![(1, crate::heatsync::senders::parse_set(&mine))]));
  assert!(app.input_dirty, "your own emotes landed: the box is drawn again");
  app.redraw();

  // a box with no emote in it has nothing to redraw
  app.line.take();
  app.input_dirty = true;
  app.redraw();
  app.on_net(NetEvent::Emotes(Some("forsen".into()), Provider::SevenTv, set(&[("KW", "ROOM7")])));
  assert!(!app.input_dirty);
}
