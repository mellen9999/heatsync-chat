// heatsync's "this changed" pushes, headless: a colour recolours rows already on screen, a hidden / edited /
// renumbered / moderated post changes in the list, the open thread and the chat line, lists asked again are
// rate-limited, plus and flairs go stale, a lifted block leaves. HSC_DUMP not needed: state + rows are asserted

use super::*;
use crate::model::livegate::GAP;
use crate::model::message::flag;
use crate::model::post::parse;
use crate::model::tabs::Kind;
use crate::net::Cmd;
use crate::net::emote_ops::EmoteCmd;
use serde_json::json;

type Asked = tokio::sync::mpsc::UnboundedReceiver<Cmd>;

fn post(id: &str, user: &str, reply_to: Option<&str>) -> crate::model::post::Post {
  parse(&json!({"base36_id": id, "username": user, "user_id": "9", "content": "hello feed",
    "user_color": "#ff0000", "reply_to": reply_to, "created_at": "2026-11-05T00:00:00Z"}))
  .unwrap()
}

fn boot() -> (App, Asked, usize) {
  let (net, asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.set = Settings::default();
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.canvas.resize(720, 360);
  for t in &mut app.tabs {
    t.view.resize(720);
  }
  app.me = Some(crate::net::auth::Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: None,
    twitch_id: Some(77),
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  });
  app.feed.posts = vec![post("0000a1", "Bob", None), post("0000a2", "carol", None)];
  app.feed.loaded = true;
  let feed = app.tabs.iter().position(|t| t.kind == Kind::Feed).expect("a feed tab");
  app.select(feed);
  app.feed_show();
  app.redraw();
  (app, asked, feed)
}

fn frame(app: &mut App, v: serde_json::Value) {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = crate::net::Out::Pipe(tx);
  crate::net::hs_ws::feed_frame(&v, &out);
  for ev in rx.try_iter() {
    app.on_net(ev);
  }
}

fn sent(asked: &mut Asked) -> Vec<Cmd> {
  std::iter::from_fn(|| asked.try_recv().ok()).collect()
}

fn row_colors(app: &App, tab: usize, id: &str) -> Vec<u32> {
  app.tabs[tab].view.msgs().filter(|m| &*m.id == id).map(|m| m.color).collect()
}

#[test]
fn a_colour_push_recolours_rows_already_shown_in_every_place_a_post_lives() {
  let (mut app, _asked, feed) = boot();
  // an inline line in the channel tab, and an open thread holding another of bob's posts
  app.on_feed_live(post("0000a3", "bob", None));
  app.feed.thread = Some(crate::model::feed::Thread {
    id: "0000a1".into(),
    op: Some(post("0000a1", "Bob", None)),
    replies: vec![post("0000a4", "bob", Some("0000a1"))],
    loading: false,
    error: false,
    focus: None,
  });
  app.feed_show();
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  let before = row_colors(&app, chat, "0000a3");
  assert_eq!(before.len(), 1);
  let want = crate::model::color::readable(0x00ff00);
  frame(
    &mut app,
    json!({"type": "profile:color", "userId": "9", "usernames": ["BOB", "bob_kick"], "color": "#00ff00"}),
  );
  assert_eq!(row_colors(&app, chat, "0000a3"), vec![want], "the chat line");
  assert_eq!(row_colors(&app, feed, "0000a1"), vec![want], "the feed list");
  assert!(app.feed.find("0000a4").is_some_and(|p| p.color == want), "the thread's reply");
  assert!(app.feed.find("0000a2").is_some_and(|p| p.color != want), "carol keeps hers");
  // a colour that is not #rrggbb, or more names than the bound, changes nothing
  frame(&mut app, json!({"type": "profile:color", "userId": "9", "usernames": ["bob"], "color": "javascript:1"}));
  assert!(app.feed.find("0000a1").is_some_and(|p| p.color == want));
}

#[test]
fn a_dm_row_wears_the_new_colour_too() {
  let (mut app, _asked, _) = boot();
  let w = crate::model::whisper::parse_dm(
    &json!({"id": 5, "from_user_id": "9", "from_display_name": "Bob", "from_color": "#ff0000", "content": "hi"}),
    None,
  )
  .unwrap();
  app.on_whispers(vec![w], true);
  let want = crate::model::color::readable(0x00ffff);
  frame(&mut app, json!({"type": "profile:color", "userId": "9", "usernames": ["bob"], "color": "#00ffff"}));
  assert!(app.whisper.timeline.iter().all(|w| w.who.color == want));
  assert_eq!(app.whisper.timeline.last.as_ref().map(|l| l.color), Some(want));
  let i = app.tab_of(Kind::Whispers).unwrap();
  assert!(app.tabs[i].view.msgs().any(|m| m.color == want), "the row was rebuilt");
}

#[test]
fn hidden_edited_updated_and_moderated_posts_change_everywhere() {
  let (mut app, _asked, feed) = boot();
  app.on_feed_live(post("0000a3", "bob", None));
  let chat = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
  // edit: new words and heatsync emote refs on the post
  frame(
    &mut app,
    json!({"type": "message-edited", "message_id": "0000a1", "content": "new &lt;words&gt;",
      "emote_refs": {"KEKW": "https://cdn.7tv.app/emote/1/1x.webp"}, "edited_at": "2026-11-05T00:01:00Z"}),
  );
  let p = app.feed.find("0000a1").unwrap();
  assert_eq!((&*p.text, p.edited, p.refs.len()), ("new <words>", true, 1));
  assert!(app.tabs[feed].view.msgs().any(|m| &*m.id == "0000a1" && m.body.contains("new <words>")));
  // the same edit twice is no change; a frame for a post we do not hold is nothing
  frame(&mut app, json!({"type": "message-edited", "message_id": "0000ff", "content": "x"}));
  assert!(app.feed.find("0000ff").is_none());
  // numbers: reply count and heat on the root
  frame(&mut app, json!({"type": "message-updated", "base36_id": "0000a2", "reply_count": 4, "heat": "12"}));
  let p = app.feed.find("0000a2").unwrap();
  assert_eq!((p.replies, p.heat), (4, 12));
  // moderation: flagged media hold, approved clears pending
  app.feed.posts[0].pending = true;
  frame(&mut app, json!({"type": "message:approved", "id": "0000a1"}));
  assert!(!app.feed.find("0000a1").unwrap().pending);
  frame(&mut app, json!({"type": "message:flagged", "id": "0000a2", "categories": ["nsfw"]}));
  assert_eq!(app.feed.find("0000a2").unwrap().cats.as_ref(), ["nsfw".into()]);
  // hidden: gone from the list, the chat line greyed
  frame(&mut app, json!({"type": "message:hidden", "id": "0000a3", "reason": "x"}));
  assert!(app.feed.find("0000a3").is_none());
  assert!(app.tabs[chat].view.msgs().any(|m| &*m.id == "0000a3" && m.flags & flag::CLEARED != 0));
  frame(&mut app, json!({"type": "message:hidden", "id": "0000a1"}));
  assert!(app.feed.posts.iter().all(|p| &*p.id != "0000a1"));
  // deleted (author, moderator or admin): the same ending, from `message_id`
  app.feed.posts.push(post("0000a4", "dave", None));
  frame(&mut app, json!({"type": "message-deleted", "message_id": "0000a4"}));
  assert!(app.feed.find("0000a4").is_none());
  frame(&mut app, json!({"type": "message-deleted", "message_id": "../x"}));
  frame(&mut app, json!({"type": "message-deleted"}));
  // hostile frames reach nothing
  for bad in [
    json!({"type": "message:hidden", "id": "../../etc"}),
    json!({"type": "message-edited", "message_id": "0000a2", "content": 7}),
    json!({"type": "message-updated", "base36_id": "0000a2", "reply_count": "lots"}),
    json!({"type": "message:flagged", "id": "0000a2", "categories": "nsfw"}),
  ] {
    frame(&mut app, bad);
  }
  assert!(app.feed.find("0000a2").is_some_and(|p| p.text.as_ref() == "hello feed"));
}

#[test]
fn bookmarks_and_the_unread_count_follow_the_account() {
  let (mut app, _asked, _) = boot();
  assert!(!app.feed.marks.get("0000a1"));
  frame(&mut app, json!({"type": "bookmarks:changed", "id": "0000A1", "on": true}));
  assert!(app.feed.marks.get("0000a1"));
  frame(&mut app, json!({"type": "bookmarks:changed", "id": "0000a1", "on": false}));
  assert!(!app.feed.marks.get("0000a1"));
  // a press of ours still out wins over the push
  assert_eq!(app.feed.marks.begin("0000a2"), Some(true));
  frame(&mut app, json!({"type": "bookmarks:changed", "id": "0000a2", "on": false}));
  assert!(app.feed.marks.get("0000a2"));
  frame(&mut app, json!({"type": "bookmarks:changed", "id": "zz zz", "on": false}));
  // read elsewhere: nothing left to mark here; a count above 0 or a negative one invents nothing
  app.notif_unread = true;
  frame(&mut app, json!({"type": "notifications:read", "unread": 3}));
  assert!(app.notif_unread);
  frame(&mut app, json!({"type": "notifications:read", "unread": -4}));
  assert!(!app.notif_unread);
}

#[test]
fn lists_asked_again_by_a_push_are_one_request_per_gap() {
  let (mut app, mut asked, _) = boot();
  sent(&mut asked);
  let blocked = |c: &Cmd| matches!(c, Cmd::Emote(EmoteCmd::Blocked));
  let follows = |c: &Cmd| matches!(c, Cmd::Following);
  for _ in 0..300 {
    frame(&mut app, json!({"type": "follows:changed"}));
    frame(&mut app, json!({"type": "emote:blocked", "hash": "ab"}));
    frame(&mut app, json!({"type": "user:unblocked", "blocked_user_id": "5", "timestamp": 1}));
  }
  let got = sent(&mut asked);
  assert_eq!(got.iter().filter(|c| follows(c)).count(), 1, "one follows ask at once");
  assert_eq!(got.iter().filter(|c| blocked(c)).count(), 1);
  assert_eq!(got.iter().filter(|c| matches!(c, Cmd::Lists)).count(), 1);
  // the rest fold into one trailing ask each, when the gap ends
  let due = app.pushed_due().expect("a trailing ask waits");
  app.pushed_tick(due - std::time::Duration::from_millis(10));
  assert!(sent(&mut asked).is_empty());
  app.pushed_tick(due.max(Instant::now() + GAP));
  let got = sent(&mut asked);
  assert_eq!((got.iter().filter(|c| follows(c)).count(), got.iter().filter(|c| blocked(c)).count()), (1, 1));
  assert!(app.pushed_due().is_none());
  // signed out: a push asks nothing
  app.me = None;
  frame(&mut app, json!({"type": "follows:changed"}));
  assert!(sent(&mut asked).is_empty());
}

#[test]
fn plus_and_flairs_are_asked_again() {
  let (mut app, _asked, _) = boot();
  let me = PaintId::Twitch(77);
  app.res.cosmetics.get(&me);
  app.res.cosmetics.take_wanted();
  app.res.flairs.get(5);
  app.res.flairs.put(5, None);
  app.res.flairs.take_wanted();
  // only the plus keys count: the site's own settings keys are not ours
  frame(&mut app, json!({"type": "user_settings:update", "show_nsfw_emotes": true, "ts": 1}));
  assert!(app.res.cosmetics.take_wanted().is_empty());
  frame(
    &mut app,
    json!({"type": "user_settings:update", "plus": true, "plus_expires_at": "2027-01-01T00:00:00Z", "ts": 1}),
  );
  assert!(app.res.cosmetics.take_wanted().contains(&me));
  // a cosmetic change names chatters: their paint and their flair are both asked again, once each
  for _ in 0..3 {
    frame(&mut app, json!({"type": "cosmetic:changed", "ids": ["5", "5"]}));
  }
  assert_eq!(app.res.flairs.take_wanted(), vec![5]);
  frame(&mut app, json!({"type": "cosmetic:changed", "ids": [5, "x y", "kick_9"]}));
  assert!(app.res.flairs.take_wanted().is_empty(), "kick ids and junk have no twitch flair");
}

#[test]
fn a_block_lifted_elsewhere_leaves_but_your_own_stay() {
  let (mut app, _asked, _) = boot();
  crate::model::filter::edit(|f| {
    f.block_synced("gone");
    f.block_synced("kept");
    f.block("mine");
  });
  let lists = |blocks: &[&str], known| crate::net::mutes::Lists {
    mutes: vec![],
    blocks: blocks.iter().map(|b| (*b).into()).collect(),
    blocks_known: known,
  };
  // a failed block read says nothing
  app.on_net(NetEvent::Lists(lists(&[], false)));
  assert!(crate::model::filter::with(|f| f.blocked("gone")));
  app.on_net(NetEvent::Lists(lists(&["kept"], true)));
  crate::model::filter::with(|f| {
    assert!(!f.blocked("gone") && f.blocked("kept") && f.blocked("mine"));
  });
}

// the site's five flagged-content switches follow the account, live; its older single switch, its chat-media
// gates, junk values and a signed-out window change nothing
#[test]
fn the_sites_content_switches_apply_here_and_nothing_else_does() {
  let (mut app, _asked, _) = boot();
  assert!(!app.set.show_nsfw && !app.set.show_violence && !app.set.show_hate);
  frame(
    &mut app,
    json!({"type": "user_settings:update", "show_sexual_emotes": true, "show_gore_emotes": true, "ts": 1}),
  );
  assert!(app.set.show_nsfw && app.set.show_violence && !app.set.show_weapons);
  assert!(Settings::load().show_nsfw, "saved");
  assert!(crate::heatsync::content::shown(crate::heatsync::content::Cat::Sexual), "the picture rule follows");
  frame(&mut app, json!({"type": "user_settings:update", "show_sexual_emotes": false, "show_hate_emotes": true}));
  assert!(!app.set.show_nsfw && app.set.show_violence && app.set.show_hate);
  // not ours: the older switch, the chat-media gates, wrong types
  let before = app.set;
  for junk in [
    json!({"show_nsfw_emotes": false, "allow_nsfw_images": true, "allow_weapons": true}),
    json!({"show_weapon_emotes": "yes", "show_drug_emotes": 1, "show_violence": false}),
  ] {
    let mut f = junk;
    f["type"] = json!("user_settings:update");
    frame(&mut app, f);
  }
  assert!(app.set == before);
  // signed out: a stray frame changes nothing
  app.me = None;
  frame(&mut app, json!({"type": "user_settings:update", "show_drug_emotes": true}));
  assert!(!app.set.show_drugs);
}

// a channel emote broadcast: the sender's set is asked again now (with a cache-bust), only for a chatter whose
// set we hold, and a stranger asks nothing
#[test]
fn an_emote_broadcast_reads_the_senders_set_again() {
  use crate::cosmetics::id::PaintId;
  let (mut app, mut asked, _) = boot();
  app.res.chatters.learned(crate::model::message::Platform::Twitch, "bob", 0xff0000, Some(PaintId::Twitch(5)));
  let _ = app.res.senders.lookup(5, "KEKW", 0);
  sent(&mut asked);
  let url = "https://heatsync.org/uploads/emotes/a.webp";
  frame(
    &mut app,
    json!({"type": "emote:broadcast", "username": "Bob", "emoteName": "KEKW", "emoteData": {"url": url}}),
  );
  assert!(matches!(sent(&mut asked).as_slice(), [Cmd::Sender(5, Some(_))]));
  frame(
    &mut app,
    json!({"type": "emote:broadcast", "username": "carol", "emoteName": "KEKW", "emoteData": {"url": url}}),
  );
  assert!(sent(&mut asked).is_empty(), "never seen: nothing to refetch");
}

// the open profile card is read again when a colour, a flair or a follow changes for its person (and only
// then), the rows stay up until the answer lands, and a burst of pushes is one request
#[test]
fn the_open_card_is_read_again_when_its_person_changes() {
  use crate::cosmetics::id::PaintId;
  use crate::model::message::{Kind as MsgKind, Message, Platform};
  use crate::net::social;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  let mut m = Message::system(crate::util::time::now_ms(), Platform::Twitch, "hi");
  m.kind = MsgKind::Chat;
  (m.login, m.nick, m.user_id) = ("alice".into(), "alice".into(), "42".into());
  app.open_card(app.subject_of(&m).expect("a chatter"));
  let cards =
    |asked: &mut Asked| sent(asked).into_iter().filter(|c| matches!(c, Cmd::Social(social::Cmd::Card { .. }))).count();
  assert_eq!(cards(&mut asked), 1, "asked on open");
  let age = |app: &mut App| app.cards.backdate(Platform::Twitch, "alice", std::time::Duration::from_secs(60));
  // inside the gap nothing is asked again
  frame(&mut app, json!({"type": "profile:color", "userId": "1", "usernames": ["Alice"], "color": "#00ff00"}));
  assert_eq!(cards(&mut asked), 0, "a push right after the open is the same answer");
  age(&mut app);
  // someone else's colour: nothing
  frame(&mut app, json!({"type": "profile:color", "userId": "1", "usernames": ["carol"], "color": "#00ff00"}));
  assert_eq!(cards(&mut asked), 0);
  // hers: asked, and a second push in a burst is not a second request
  frame(&mut app, json!({"type": "profile:color", "userId": "1", "usernames": ["alice"], "color": "#00ff00"}));
  assert_eq!(cards(&mut asked), 1);
  frame(&mut app, json!({"type": "profile:color", "userId": "1", "usernames": ["alice"], "color": "#ff0000"}));
  assert_eq!(cards(&mut asked), 0);
  // a paint change by id: hers asks, another's does not
  age(&mut app);
  app.on_net(NetEvent::CosmeticsChanged(vec![PaintId::Twitch(7)]));
  assert_eq!(cards(&mut asked), 0);
  app.on_net(NetEvent::CosmeticsChanged(vec![PaintId::Twitch(42)]));
  assert_eq!(cards(&mut asked), 1);
  // a follow changed on another device: the card's follow row may have flipped
  age(&mut app);
  app.me = boot().0.me;
  frame(&mut app, json!({"type": "follows:changed"}));
  app.pushed_tick(std::time::Instant::now() + std::time::Duration::from_secs(10));
  assert_eq!(cards(&mut asked), 1);
  // no card open: nothing asked
  app.close_card();
  frame(&mut app, json!({"type": "profile:color", "userId": "1", "usernames": ["alice"], "color": "#00ff00"}));
  assert_eq!(cards(&mut asked), 0);
}

// the live `new` room: joined while you are signed in with the feed tab up, let go otherwise, and its posts are
// taken only from the people you follow, you, and the thread open now
#[test]
fn live_feed_posts_are_the_ones_you_follow() {
  use crate::net::hs_ws::Cmd as HsCmd;
  let (mut app, mut asked, _) = boot();
  let rooms = |asked: &mut Asked| {
    sent(asked)
      .into_iter()
      .filter_map(|c| if let Cmd::Hs(HsCmd::Feed(k, on)) = c { Some((k.to_string(), on)) } else { None })
      .collect::<Vec<_>>()
  };
  app.feed_room_sync();
  assert_eq!(rooms(&mut asked), [("new".to_string(), true)]);
  app.feed_room_sync();
  assert!(rooms(&mut asked).is_empty(), "once");
  // nothing is taken before the follow list lands
  let live = |app: &mut App, id: &str, user: &str, to: Option<&str>| {
    app.on_net(NetEvent::FeedLive(post(id, user, to)));
    app.feed.find(id).is_some()
  };
  assert!(!live(&mut app, "0000b1", "dave", None));
  app.on_net(NetEvent::FollowedUsers(vec!["dave".into(), "erin".into()]));
  assert!(live(&mut app, "0000b2", "Dave", None), "a followed name, any case");
  assert!(!live(&mut app, "0000b3", "frank", None), "not followed");
  assert!(live(&mut app, "0000b4", "mellen", None), "your own");
  // a reply in the thread open now comes whoever wrote it
  app.feed.open("0000a1", None);
  assert!(live(&mut app, "0000b5", "frank", Some("0000a1")));
  assert!(!live(&mut app, "0000b6", "frank", Some("0000a2")), "another thread");
  // the tab put away, or signed out: the room is let go
  app.set.show(Kind::Feed, false);
  app.fix_active();
  assert_eq!(rooms(&mut asked), [("new".to_string(), false)]);
  app.set.show(Kind::Feed, true);
  app.fix_active();
  app.me = None;
  app.feed_room_sync();
  assert_eq!(rooms(&mut asked), [("new".to_string(), true), ("new".to_string(), false)]);
}

// a profile change for the open card's person reads it again, and its `feed:author` room is joined while it is up
#[test]
fn an_open_card_follows_its_profile_room() {
  use crate::model::card::Info;
  use crate::model::message::{Kind as MsgKind, Message, Platform};
  use crate::net::hs_ws::Cmd as HsCmd;
  use crate::net::social;
  let (net, mut asked) = Net::tap();
  let mut app = App::new(net, vec!["forsen".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(720, 400);
  let info = Info { registered: true, profile_id: Some("42".into()), ..Info::default() };
  app.cards.put(Platform::Twitch, "alice", Ok(info));
  let mut m = Message::system(crate::util::time::now_ms(), Platform::Twitch, "hi");
  m.kind = MsgKind::Chat;
  (m.login, m.nick, m.user_id) = ("alice".into(), "alice".into(), "7".into());
  app.open_card(app.subject_of(&m).expect("a chatter"));
  app.redraw();
  let room = |asked: &mut Asked| {
    sent(asked)
      .into_iter()
      .filter_map(|c| if let Cmd::Hs(HsCmd::Feed(k, on)) = c { Some((k.to_string(), on)) } else { None })
      .collect::<Vec<_>>()
  };
  assert_eq!(room(&mut asked), [("author:42".to_string(), true)]);
  app.cards.backdate(Platform::Twitch, "alice", std::time::Duration::from_secs(60));
  frame(&mut app, json!({"type": "profile:updated", "userId": "99"}));
  assert!(sent(&mut asked).is_empty(), "someone else's profile");
  frame(&mut app, json!({"type": "profile:updated", "userId": "42"}));
  assert!(sent(&mut asked).iter().any(|c| matches!(c, Cmd::Social(social::Cmd::Card { .. }))));
  frame(&mut app, json!({"type": "profile:updated", "userId": "4x"}));
  app.close_card();
  app.redraw();
  assert_eq!(room(&mut asked), [("author:42".to_string(), false)]);
}
