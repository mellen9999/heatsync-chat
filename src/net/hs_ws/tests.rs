use super::*;

// close 4001 = bad token (contract.md); every other close, and none, is a plain reconnect
#[test]
fn only_4001_is_a_bad_token() {
  assert!(matches!(closed(Some(4001)), End::BadToken));
  assert!(matches!(closed(Some(1000)), End::Reconnect));
  assert!(matches!(closed(Some(4000)), End::Reconnect));
  assert!(matches!(closed(None), End::Reconnect));
}

// every push frame is read into one event, and a hostile or short one into none
#[test]
fn push_frames_become_events_and_bad_ones_become_nothing() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  let mut got = |v: Value| {
    assert!(dispatch(&v, &mut st, &out).is_none());
    rx.try_iter().collect::<Vec<_>>()
  };
  let e = got(json!({"type": "profile:color", "userId": "1", "usernames": ["A"], "color": "#00ff00"}));
  assert!(matches!(e.as_slice(), [NetEvent::ProfileColor(c)] if *c.names == ["a".into()]));
  assert!(got(json!({"type": "profile:color", "userId": "1", "usernames": ["A"], "color": "green"})).is_empty());
  assert!(
    matches!(got(json!({"type": "message:hidden", "id": "4a"})).as_slice(), [NetEvent::PostHidden(i)] if &**i == "4a")
  );
  assert!(got(json!({"type": "message:hidden", "id": "4a4a4a4a4a"})).is_empty(), "an id past 8 is no id");
  assert!(matches!(
    got(json!({"type": "message-edited", "message_id": "4a", "content": "x"})).as_slice(),
    [NetEvent::PostEdited(_)]
  ));
  assert!(matches!(
    got(json!({"type": "message-updated", "base36_id": "4a", "reply_count": 2})).as_slice(),
    [NetEvent::PostMeta(_)]
  ));
  assert!(matches!(
    got(json!({"type": "message:approved", "id": "4a"})).as_slice(),
    [NetEvent::PostModerated(_, pushed::Mod::Approved)]
  ));
  assert!(matches!(
    got(json!({"type": "message:flagged", "id": "4a", "categories": ["nsfw"]})).as_slice(),
    [NetEvent::PostModerated(_, pushed::Mod::Flagged(_))]
  ));
  for (ty, ask) in [
    ("follows:changed", Ask::Follows),
    ("emote:blocked", Ask::EmoteBlocks),
    ("emote:unblocked", Ask::EmoteBlocks),
    ("user:blocked", Ask::Blocks),
    ("user:unblocked", Ask::Blocks),
    ("mention-rules:changed", Ask::MentionRules),
  ] {
    assert!(matches!(got(json!({"type": ty})).as_slice(), [NetEvent::Refetch(a)] if *a == ask), "{ty}");
  }
  // a rule match: checked, bounded, and nothing at all when it lacks what a row needs
  let hit = json!({"type": "mention:rule-match", "data": {"ruleId": 3, "pattern": "gg", "channel": "Forsen",
    "platform": "twitch", "username": "bob", "snippet": "gg wp"}});
  assert!(
    matches!(got(hit.clone()).as_slice(), [NetEvent::RuleHit(h)] if &*h.channel == "forsen" && &*h.user == "bob")
  );
  for bad in [
    json!({"type": "mention:rule-match"}),
    json!({"type": "mention:rule-match", "data": "x"}),
    json!({"type": "mention:rule-match", "data": {"channel": "a b", "platform": "twitch", "username": "u", "snippet": "s"}}),
    json!({"type": "mention:rule-match", "data": {"channel": "c", "platform": "nope", "username": "u", "snippet": "s"}}),
  ] {
    assert!(got(bad).is_empty());
  }
  assert!(matches!(got(json!({"type": "notifications:read", "unread": -1})).as_slice(), [NetEvent::Unread(0)]));
  assert!(got(json!({"type": "notifications:read"})).is_empty());
  assert!(matches!(
    got(json!({"type": "bookmarks:changed", "id": "4a", "on": true})).as_slice(),
    [NetEvent::Bookmark(_, true)]
  ));
  assert!(got(json!({"type": "bookmarks:changed", "id": "4a"})).is_empty());
  assert!(matches!(
    got(json!({"type": "user_settings:update", "plus": false, "ts": 1})).as_slice(),
    [NetEvent::PlusMoved]
  ));
  // the older single nsfw switch is the site's own; a category switch is ours, and rides with plus
  assert!(got(json!({"type": "user_settings:update", "show_nsfw_emotes": true, "ts": 1})).is_empty());
  assert!(matches!(
    got(json!({"type": "user_settings:update", "show_gore_emotes": true, "plus": true, "ts": 1})).as_slice(),
    [NetEvent::ContentPrefs(p), NetEvent::PlusMoved] if p.as_slice() == [(crate::heatsync::content::Cat::Violence, true)]
  ));
  // the frame cap (MAX_FRAME) keeps oversized text out before this point; a long name or edit is cut here
  let long = "x".repeat(100_000);
  assert!(got(json!({"type": "profile:color", "userId": "1", "usernames": [long], "color": "#00ff00"})).is_empty());
  let e = got(json!({"type": "message-edited", "message_id": "4a", "content": "y".repeat(100_000)}));
  assert!(matches!(e.as_slice(), [NetEvent::PostEdited(e)] if e.text.len() <= 4000));
}

#[test]
fn a_notification_frame_is_checked_and_an_inbox_look_is_heard() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  // the envelope's type is the event name; the kind is the notification's own
  let v =
    json!({"type": "notification:new", "kind": "reply", "message_id": "ab12", "content": "yo", "parent_is_op": false});
  dispatch(&v, &mut st, &out);
  let Ok(NetEvent::Notif(it)) = rx.try_recv() else { panic!("no notification") };
  assert_eq!((it.post.as_deref(), &*it.text), (Some("ab12"), "replied to your reply: yo"));
  // nothing usable, or hostile: nothing reaches the app
  for bad in [json!({"type": "notification:new"}), json!({"type": "notification:new", "kind": 7, "content": 7})] {
    dispatch(&bad, &mut st, &out);
  }
  assert!(rx.try_recv().is_err());
  dispatch(&json!({"type": "seen:update", "surface": "inbox", "at": 1_700_000_000_000i64}), &mut st, &out);
  assert!(rx.try_recv().is_err(), "the site's dm inbox time is not the notifications'");
}

#[test]
fn a_followed_channel_going_live_is_a_stream_line() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  st.gate.start(0);
  let v =
    json!({"type": "follow:stream:online", "platform": "twitch", "channel": "Ennortix", "game": "Chess", "title": "x"});
  assert!(dispatch(&v, &mut st, &out).is_none());
  // the followed list hears it first, then the chat gets its line
  let Ok(NetEvent::FollowChange(c)) = rx.try_recv() else { panic!("no list change") };
  assert_eq!((&*c.key, &*c.game, &*c.name), ("ennortix", "Chess", ""));
  let Ok(NetEvent::Stream(l)) = rx.try_recv() else { panic!("no line") };
  assert_eq!((&*l.channel, &*l.text), ("ennortix", "[ennortix] ◆ went live — Chess"));
  // the room reporting the same go-live is the same line: not shown twice
  apply(&mut st, Cmd::Join(Platform::Twitch, "ennortix".into()));
  let room = json!({"type": "stream:online", "platform": "twitch", "channel": "ennortix", "game": "Chess"});
  dispatch(&room, &mut st, &out);
  assert!(rx.try_recv().is_err());
  // a followed go-live inside the connect window (heatsync's catch-up of ones you just missed) still
  // shows; one for a channel you have a tab for is left to its room: the list hears it, the chat doesn't
  let mut st = State::default();
  st.gate.start(crate::util::time::now_ms());
  dispatch(&v, &mut st, &out);
  let got: Vec<_> = rx.try_iter().collect();
  assert!(got.iter().any(|e| matches!(e, NetEvent::Stream(_))), "the catch-up is not held");
  apply(&mut st, Cmd::Join(Platform::Twitch, "other".into()));
  let tabbed = json!({"type": "follow:stream:online", "platform": "twitch", "channel": "other", "game": "Chess"});
  dispatch(&tabbed, &mut st, &out);
  let got: Vec<_> = rx.try_iter().collect();
  assert!(got.iter().any(|e| matches!(e, NetEvent::FollowChange(_))));
  assert!(!got.iter().any(|e| matches!(e, NetEvent::Stream(_))), "its room speaks for it");
}

#[test]
fn a_frame_user_id_and_color_ride_the_line_and_hostile_ones_are_dropped() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  st.gate.start(0);
  let mut id = |n: &str, ch: &str, plat: &str| {
    let v = json!({"type": "follow:stream:online", "platform": plat, "channel": ch, "game": "Chess",
      "displayName": "MixedCase", "userId": n, "color": "#ff0000"});
    dispatch(&v, &mut st, &out);
    rx.try_iter()
      .find_map(|e| if let NetEvent::Stream(l) = e { Some(l) } else { None })
      .map(|l| (l.user_id, l.color, l.name))
  };
  let got = id("12345", "aaa", "twitch").unwrap();
  assert_eq!((got.0.as_deref(), got.1, &*got.2), (Some("12345"), Some(0xff0000), "aaa"));
  assert_eq!(id("77", "bbb", "kick").unwrap().0.as_deref(), Some("77"));
  for bad in ["12a", "kick_5", "yt_UCabcdefghijklmnopqrstuv", "", "0", &"9".repeat(25), "-1"] {
    assert_eq!(id(bad, &format!("c{}", bad.len()), "twitch").unwrap().0, None, "{bad}");
  }
  assert_eq!(
    super::stream_events::user_id(Platform::Youtube, Some(&json!("UCabcdefghijklmnopqrstuv"))).as_deref(),
    Some("UCabcdefghijklmnopqrstuv")
  );
  assert_eq!(super::stream_events::user_id(Platform::Youtube, Some(&json!("UCshort"))), None);
  assert_eq!(super::stream_events::user_id(Platform::Twitch, Some(&json!(5))), None);
}

#[test]
fn follow_history_keeps_recent_switched_and_offline_rows_only() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  let now = crate::util::time::now_ms();
  let row = |ty: &str, ago: i64| {
    json!({"type": ty, "platform": "twitch", "channel": "Ennortix", "game": "Chess", "prevGame": "Go",
    "time": now - ago, "color": "#00ff00", "userId": "99"})
  };
  let rows = vec![
    row("follow:stream:update", 60_000),
    row("follow:stream:offline", 120_000),
    row("follow:stream:online", 30_000),
    row("follow:stream:update", 20 * 60_000),
    json!({"type": "follow:stream:offline", "platform": "youtube", "channel": "x", "time": now}),
    json!(7),
  ];
  dispatch(&json!({"type": "follow:history", "events": rows}), &mut st, &out);
  let Ok(NetEvent::StreamHistory(got)) = rx.try_recv() else { panic!("no history") };
  let got: Vec<_> = got.iter().map(|(t, l)| (now - t, &*l.text, l.color, l.user_id.as_deref())).collect();
  assert_eq!(got.len(), 2);
  assert!(got[0].0 >= 120_000 && got[1].0 >= 60_000, "oldest first, at their own times");
  assert_eq!((got[0].1, got[0].2, got[0].3), ("[ennortix] ◆ went offline", Some(0x00ff00), Some("99")));
  assert_eq!(got[1].1, "[ennortix] ◆ switched to Chess");
  // bounded: 200 rows are read, no more
  let many: Vec<_> = (0..500).map(|_| row("follow:stream:offline", 1000)).collect();
  dispatch(&json!({"type": "follow:history", "events": many}), &mut st, &out);
  let Ok(NetEvent::StreamHistory(got)) = rx.try_recv() else { panic!("no history") };
  assert_eq!(got.len(), 200);
}

// the plain `stream:*` of a room is no follow: the list ignores it. the snapshot is bounded and checked
#[test]
fn the_followed_list_hears_follow_frames_and_the_snapshot_only() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  st.gate.start(0);
  apply(&mut st, Cmd::Join(Platform::Twitch, "ennortix".into()));
  let room = json!({"type": "stream:online", "platform": "twitch", "channel": "ennortix", "game": "Chess"});
  dispatch(&room, &mut st, &out);
  assert!(!rx.try_iter().any(|e| matches!(e, NetEvent::FollowChange(_))), "a room's own news is not a follow");
  for bad in [json!({"platform": "twitch", "channel": "a b"}), json!({"platform": "youtube", "channel": "x"})] {
    let mut v = bad;
    v["type"] = json!("follow:stream:offline");
    dispatch(&v, &mut st, &out);
  }
  assert!(rx.try_iter().next().is_none(), "junk makes nothing");
  let many: Vec<_> =
    (0..2000).map(|i| json!({"platform": "twitch", "username": format!("u{i}"), "viewerCount": i})).collect();
  dispatch(&json!({"type": "follow:live:snapshot", "streams": many}), &mut st, &out);
  let Ok(NetEvent::FollowSnapshot(rows)) = rx.try_recv() else { panic!("no snapshot") };
  assert_eq!(rows.len(), crate::model::following::MAX_ROWS);
  for junk in
    [json!({"type": "follow:live:snapshot"}), json!({"type": "follow:live:snapshot", "streams": [1, "x", null]})]
  {
    dispatch(&junk, &mut st, &out);
  }
  assert!(rx.try_recv().is_err(), "no rows, no event");
}

// kick-moderation: a timeout carries its length, a ban none; a junk name is no removal
#[test]
fn a_kick_removal_says_how_long() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  let f = |action: &str, user: &str| json!({"type": "kick-moderation", "channel": "xqc", "action": action, "username": user, "durationSec": 300});
  let got = |rx: &std::sync::mpsc::Receiver<NetEvent>| match rx.try_recv() {
    Ok(NetEvent::RelayClear { login, secs, .. }) => Some((login.to_string(), secs)),
    _ => None,
  };
  dispatch(&f("timeout", "Bob"), &mut st, &out);
  assert_eq!(got(&rx), Some(("bob".into(), Some(300))));
  dispatch(&f("ban", "bob"), &mut st, &out);
  assert_eq!(got(&rx), Some(("bob".into(), None)));
  dispatch(&f("ban", "a b\n"), &mut st, &out);
  assert_eq!(got(&rx), None);
}

#[test]
fn the_heat_push_becomes_scores_by_login_and_a_bad_one_is_dropped() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  let f = json!({"type": "user:heat_batch_update", "updates": [
    {"username": "Bob", "userHeat": 120, "op": 1, "re": 2}, {"username": "bad name", "userHeat": 5}, {"userHeat": 9}, 7]});
  dispatch(&f, &mut st, &out);
  let Ok(NetEvent::Heat(got)) = rx.try_recv() else { panic!("no heat") };
  assert_eq!(got, vec![("bob".into(), 120)]);
  dispatch(&json!({"type": "user:heat_batch_update", "updates": "x"}), &mut st, &out);
  dispatch(&json!({"type": "user:heat_batch_update"}), &mut st, &out);
  assert!(rx.try_recv().is_err(), "nothing readable: nothing sent");
}

#[test]
fn a_live_grid_broadcast_for_a_channel_without_a_tab_is_dropped() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  st.gate.start(0);
  let grid = json!({"type": "stream:online", "platform": "twitch", "channel": "zbra", "game": "Overwatch"});
  dispatch(&grid, &mut st, &out);
  assert!(rx.try_recv().is_err(), "not followed, not tabbed: no line");
  apply(&mut st, Cmd::Join(Platform::Twitch, "zbra".into()));
  dispatch(&grid, &mut st, &out);
  assert!(matches!(rx.try_recv(), Ok(NetEvent::Stream(_))), "its own tab still hears it");
}

#[test]
fn a_youtube_replay_fills_the_gap_after_a_drop_but_is_dropped_on_a_first_socket() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  apply(&mut st, Cmd::YtSub("@Chan".into()));
  let replay = json!({"type": "youtube:chat", "channelId": "@chan", "replay": true,
    "messages": [{"id": "m1", "user": "al", "text": "hi", "timestamp": 5}, {"user": "bo", "text": "no id"}]});
  // first socket: the backlog is history's job (/api/recent), not ours
  assert!(resubscribed(&st).is_empty());
  dispatch(&replay, &mut st, &out);
  assert!(rx.try_recv().is_err());
  // after a drop the same replay is what was said meanwhile: kept by id, the id-less line is not
  st.resumed = true;
  st.catching = resubscribed(&st);
  assert!(st.catching.contains("@chan"));
  dispatch(&replay, &mut st, &out);
  let Ok(NetEvent::CatchUp { platform, key, msgs }) = rx.try_recv() else { panic!("no catch-up") };
  assert_eq!((platform, &*key, msgs.len()), (Platform::Youtube, "@chan", 1));
  assert_eq!(&*msgs[0].id, "m1");
  // a feed that wasn't re-subscribed (added since) is still dropped, and live batches go as before
  let other = json!({"type": "youtube:chat", "channelId": "@new", "replay": true,
    "messages": [{"id": "x", "user": "al", "text": "hi"}]});
  dispatch(&other, &mut st, &out);
  assert!(rx.try_recv().is_err());
  let live =
    json!({"type": "youtube:chat", "channelId": "@chan", "messages": [{"id": "m2", "user": "al", "text": "yo"}]});
  dispatch(&live, &mut st, &out);
  assert!(matches!(rx.try_recv(), Ok(NetEvent::Relay { history: false, .. })));
}

#[test]
fn apply_validates_and_dedupes() {
  let mut st = State::default();
  assert_eq!(apply(&mut st, Cmd::Join(Platform::Kick, "xqc".into())).len(), 1);
  assert!(apply(&mut st, Cmd::Join(Platform::Kick, "xqc".into())).is_empty());
  assert!(apply(&mut st, Cmd::Join(Platform::Kick, "bad name".into())).is_empty());
  assert_eq!(apply(&mut st, Cmd::YtSub("@handle".into())).len(), 1);
  let f = apply(&mut st, Cmd::Leave(Platform::Kick, "xqc".into()));
  assert_eq!((f.len(), &f[0]["type"]), (1, &json!("channel:leave")));
}

#[test]
fn cosmetic_changed_keeps_each_id_in_its_own_namespace() {
  let v = json!({"type": "cosmetic:changed", "ids": ["37005319", "kick_5", "yt_UC1", 7, "x"]});
  // a bare id refreshes the twitch chatter and a native account of that id; kick_5 only kick 5
  assert_eq!(cosmetic_ids(&v), vec![PaintId::Native("37005319".into()), PaintId::Twitch(37005319), PaintId::Kick(5)]);
  assert!(cosmetic_ids(&json!({"type": "cosmetic:changed"})).is_empty());
}

#[test]
fn youtube_delete_and_emotes_refresh_are_bounded_and_the_site_only_frames_are_ignored() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  let del = json!({"type": "youtube:delete", "channelId": "@chan", "videoId": "v",
    "messageIds": ["m1", "bad id", 5], "authorChannelIds": []});
  dispatch(&del, &mut st, &out);
  let Ok(NetEvent::YtGone { key, gone }) = rx.try_recv() else { panic!("no delete") };
  assert_eq!((&*key, gone.ids.len(), gone.authors.len()), ("@chan", 1, 0));
  // no room, a hostile room, or nothing usable to remove: nothing
  for bad in [
    json!({"type": "youtube:delete", "messageIds": ["m1"]}),
    json!({"type": "youtube:delete", "channelId": "a b", "messageIds": ["m1"]}),
    json!({"type": "youtube:delete", "channelId": "@chan", "messageIds": "m1"}),
  ] {
    dispatch(&bad, &mut st, &out);
  }
  assert!(rx.try_recv().is_err());
  // emotes:refresh is a bare nudge; whatever rides along is never read
  dispatch(&json!({"type": "emotes:refresh", "junk": "x".repeat(10_000)}), &mut st, &out);
  assert!(matches!(rx.try_recv(), Ok(NetEvent::Inventory { removed: None })));
  // settings:delete is the site's (setting keys): nothing here uses it
  dispatch(&json!({"type": "settings:delete", "key": "theme", "ts": 1}), &mut st, &out);
  assert!(rx.try_recv().is_err());
}

// the site's heat beat reaches every client; a post's number is absolute and checked
#[test]
fn the_heat_beat_becomes_a_posts_number_and_a_bad_one_is_dropped() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  let beat =
    |id: Value, heat: Value| json!({"type": "heat:updated", "messageId": id, "newHeat": heat, "newMaxHeat": 99});
  dispatch(&beat(json!("00AB12"), json!(9.6)), &mut st, &out);
  let Ok(NetEvent::PostHeat { id, heat }) = rx.try_recv() else { panic!("no heat") };
  assert_eq!((&*id, heat), ("00ab12", 10), "lowercase id, the number rounded");
  dispatch(&beat(json!("ab12"), json!(1e30)), &mut st, &out);
  assert!(matches!(rx.try_recv(), Ok(NetEvent::PostHeat { heat: 1_000_000_000, .. })), "bounded");
  for bad in [
    beat(json!("ab 12"), json!(5)),
    beat(json!("a".repeat(9)), json!(5)),
    beat(json!(""), json!(5)),
    beat(json!(7), json!(5)),
    beat(json!("ab12"), json!(-3)),
    beat(json!("ab12"), json!("5")),
    beat(json!("ab12"), Value::Null),
    json!({"type": "heat:updated", "newHeat": 5}),
    json!({"type": "heat:updated", "messageId": "ab12"}),
  ] {
    dispatch(&bad, &mut st, &out);
  }
  assert!(rx.try_recv().is_err());
}

// a dm taken down by either of its people: the frame names it by id, flat like `dm:new`
#[test]
fn a_deleted_dm_names_its_row_and_a_bad_frame_is_dropped() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  let gone = |id: Value| json!({"type": "dm:message-deleted", "threadId": "3", "messageId": id});
  dispatch(&gone(json!("42")), &mut st, &out);
  let Ok(NetEvent::DmGone(key)) = rx.try_recv() else { panic!("no dm gone") };
  assert_eq!(&*key, "dm42");
  dispatch(&gone(json!(43)), &mut st, &out);
  assert!(matches!(rx.try_recv(), Ok(NetEvent::DmGone(k)) if &*k == "dm43"), "a number reads the same");
  for bad in [
    gone(json!("")),
    gone(json!("../x")),
    gone(json!("x".repeat(70))),
    gone(json!({"a": 1})),
    gone(Value::Null),
    json!({"type": "dm:message-deleted", "threadId": "3"}),
  ] {
    dispatch(&bad, &mut st, &out);
  }
  assert!(rx.try_recv().is_err());
}

#[test]
fn leaving_a_kick_room_forgets_its_resume_seq() {
  let mut st = State::default();
  apply(&mut st, Cmd::Join(Platform::Kick, "xqc".into()));
  st.kick_seq.insert("xqc".into(), 7);
  apply(&mut st, Cmd::Leave(Platform::Kick, "xqc".into()));
  assert!(st.kick_seq.is_empty());
}

// a twitch room joins heatsync's copy of its chat too, and leaves it with the room
#[test]
fn a_twitch_room_joins_and_parts_heatsyncs_irc_copy() {
  let mut st = State::default();
  let types = |f: &[Value]| f.iter().map(|f| f["type"].as_str().unwrap_or("").to_owned()).collect::<Vec<_>>();
  let f = apply(&mut st, Cmd::Join(Platform::Twitch, "forsen".into()));
  assert_eq!(types(&f), ["channel:join", "irc:join"]);
  assert_eq!(f[1]["channel"], "forsen");
  assert_eq!(types(&apply(&mut st, Cmd::Leave(Platform::Twitch, "forsen".into()))), ["channel:leave", "irc:part"]);
  assert_eq!(types(&apply(&mut st, Cmd::Join(Platform::Kick, "forsen".into()))), ["channel:join"]);
}

// a server copy of a twitch line hands over its refs by id; one without refs is never parsed, one for a
// room we're not in or with a hostile ref is dropped
#[test]
fn an_irc_copy_hands_over_its_refs() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  apply(&mut st, Cmd::Join(Platform::Twitch, "forsen".into()));
  let frame = |ch: &str, refs: Value| {
    json!({"type": "irc:message", "channel": ch, "message": {"type": "privmsg", "id": "abc", "username": "bob",
      "content": "kek hi", "hsEmotes": refs}})
  };
  let good = json!({"kek": {"url": "https://cdn.7tv.app/emote/x/1x.webp", "provider": "7tv", "zeroWidth": true},
    "evil": {"url": "https://evil.example/x.png"}});
  dispatch(&frame("forsen", good.clone()), &mut st, &out);
  let Ok(NetEvent::HsRefs { chan, id, refs }) = rx.try_recv() else { panic!("no refs") };
  assert_eq!((&*chan, &*id), ("forsen", "abc"));
  assert_eq!(&*refs, [("kek".into(), "https://cdn.7tv.app/emote/x/1x.webp".into(), true)]);
  dispatch(&frame("xqc", good), &mut st, &out);
  assert!(rx.try_recv().is_err(), "not our room");
  assert!(refless(r#"{"type":"irc:message","channel":"forsen","message":{"id":"x","content":"hi"}}"#));
  assert!(!refless(&frame("forsen", json!({})).to_string()));
  assert!(!refless(r#"{"type":"kick-chat-message"}"#));
}

// account:changed, and a later socket's sign-in (one sent while we were away is lost), re-read the account
#[test]
fn the_account_is_read_again_on_account_changed_and_on_a_later_sign_in() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  let auth = json!({"type": "authenticated"});
  let got =
    |rx: &std::sync::mpsc::Receiver<NetEvent>| rx.try_iter().filter(|e| matches!(e, NetEvent::AccountChanged)).count();
  dispatch(&auth, &mut st, &out);
  assert_eq!(got(&rx), 0, "the first sign-in: login just read it");
  dispatch(&json!({"type": "account:changed"}), &mut st, &out);
  assert_eq!(got(&rx), 1);
  dispatch(&auth, &mut st, &out);
  assert_eq!(got(&rx), 0, "the same socket signing in again is not a comeback");
  st.socket_authed = false;
  dispatch(&auth, &mut st, &out);
  assert_eq!(got(&rx), 1, "a new socket's sign-in");
}

// one thread room at a time (naming another leaves the last), hostile ids refused, and its frames only
// count while it is joined
#[test]
fn a_thread_room_is_joined_alone_and_its_frames_need_it() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  let react =
    json!({"type": "reaction:added", "message_id": "00004a", "user_id": "7", "emote_id": 1, "emote_name": "k"});
  dispatch(&react, &mut st, &out);
  assert!(rx.try_recv().is_err(), "no room joined: ignored");
  let f = apply(&mut st, Cmd::Thread(Some("00004A".into())));
  assert_eq!((f.len(), f[0]["type"].clone(), f[0]["feed"].clone()), (1, json!("feed:join"), json!("thread:00004a")));
  assert!(apply(&mut st, Cmd::Thread(Some("00004a".into()))).is_empty());
  let f = apply(&mut st, Cmd::Thread(Some("00004b".into())));
  assert_eq!((&f[0]["type"], &f[1]["type"]), (&json!("feed:leave"), &json!("feed:join")));
  assert!(apply(&mut st, Cmd::Thread(Some("bad id!".into()))).len() == 1, "a bad id only leaves");
  apply(&mut st, Cmd::Thread(Some("00004a".into())));
  dispatch(&react, &mut st, &out);
  assert!(matches!(rx.try_recv(), Ok(NetEvent::Reaction(_))));
  dispatch(&json!({"type": "vote:updated", "message_id": "00004a", "heat": 12.0}), &mut st, &out);
  assert!(matches!(rx.try_recv(), Ok(NetEvent::PostHeat { heat: 12, .. })));
  dispatch(&json!({"type": "vote:updated", "message_id": "no way", "heat": 1}), &mut st, &out);
  assert!(rx.try_recv().is_err());
  dispatch(&json!({"type": "authenticated", "userId": "u-9"}), &mut st, &out);
  assert!(matches!(rx.try_recv(), Ok(NetEvent::HsUser(u)) if &*u == "u-9"));
  assert!(matches!(rx.try_recv(), Ok(NetEvent::HsAuth(true))));
  let f = apply(&mut st, Cmd::Thread(None));
  assert_eq!(f[0]["type"], "feed:leave");
}

// a frame nothing reads is counted by type (and named once on stderr), and the table is bounded
#[test]
fn unknown_frames_are_counted_not_dropped_silently() {
  let (tx, _rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  for _ in 0..3 {
    assert!(dispatch(&json!({"type": "brand:new"}), &mut st, &out).is_none());
  }
  assert!(dispatch(&json!({"type": "other:new"}), &mut st, &out).is_none());
  assert_eq!((st.unknown["brand:new"], st.unknown["other:new"]), (3, 1));
  // a type this client reads never lands in the table
  dispatch(&json!({"type": "message:hidden", "id": "4a"}), &mut st, &out);
  assert!(!st.unknown.contains_key("message:hidden"));
  for i in 0..500 {
    dispatch(&json!({"type": format!("junk:{i}")}), &mut st, &out);
  }
  assert!(st.unknown.len() <= more::MAX_UNKNOWN);
}

// `stream:sub` is a line in the room's own tab as the gift bomb is: only for a room we are in, hostile names cleaned
#[test]
fn a_new_sub_is_a_line_in_its_own_room_only() {
  let (tx, rx) = std::sync::mpsc::channel();
  let out = Out::Pipe(tx);
  let mut st = State::default();
  st.rooms.insert(("twitch", "xqc".into()));
  let sub = |ch: &str, user: &str| json!({"type": "stream:sub", "platform": "twitch", "channel": ch, "user": user});
  assert!(dispatch(&sub("xqc", "Bob\nX"), &mut st, &out).is_none());
  let got: Vec<_> = rx.try_iter().collect();
  assert!(matches!(got.as_slice(), [NetEvent::Stream(l)]
    if l.tone == stream_events::Tone::Gift && &*l.text == "[xqc] ◆ Bob X subscribed"));
  dispatch(&sub("forsen", "bob"), &mut st, &out);
  dispatch(&sub("xqc", ""), &mut st, &out);
  assert!(rx.try_iter().next().is_none(), "not our room, and no name, are nothing");
  assert!(st.unknown.is_empty(), "stream:sub is read now");
}
