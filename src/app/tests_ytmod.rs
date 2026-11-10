// the typed mod commands on youtube, headless: a merged tab's youtube feed joins /ban /timeout /unban /purge /delete
// /nuke only while it is live and we moderate it, and the person (or line) is found among the tab's own youtube
// lines. a feed left out is said; twitch and kick run as before

use super::*;
use crate::model::message::{Kind as MsgKind, Message, flag};
use crate::model::modcmd::Verb;
use crate::net::Cmd;
use crate::net::auth::Me;
use crate::net::irc::Chat;
use crate::net::modact::{Done, Fail, Req};
use winit::keyboard::{Key, NamedKey};

type Rx = tokio::sync::mpsc::UnboundedReceiver<Cmd>;

const UC: &str = "UCabcdefghijklmnopqrstuv";
const NOT_READY: &str = "youtube skipped: not live, or you haven't chatted there as a mod";

fn boot_tab(tab: &str) -> (App, Rx) {
  let (net, mut rx) = Net::tap();
  let mut app = App::new(net, vec![tab.into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(520, 300);
  app.on_net(NetEvent::Me(Some(Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: None,
    names: vec!["mellen".into()],
    kick: Some("mellen_k".into()),
    youtube: Some("mellenyt".into()),
    packs: vec![],
  })));
  app.on_chat("forsen", Chat::Mod(true));
  app.redraw();
  while rx.try_recv().is_ok() {}
  (app, rx)
}

// twitch, kick and youtube merged in one tab: the usual
fn boot() -> (App, Rx) {
  boot_tab("forsen kick:xqc yt:@abc")
}

fn yt_only() -> (App, Rx) {
  let (mut app, rx) = boot_tab("yt:@abc");
  app.select(app.tabs.iter().position(|t| t.sources.iter().any(|s| s.0 == Platform::Youtube)).unwrap());
  (app, rx)
}

fn live(app: &mut App) {
  app.on_net(NetEvent::YtStatus { key: "@abc".into(), status: "connected".into() });
}

// our own line there wears the moderator badge: the room is ours
fn modded(app: &mut App) {
  let mut me = line(Platform::Youtube, "@mellenyt", "me1", "UCmellenmellenmellenmell", "hi", 1);
  me.badges = vec![("moderator".into(), "1".into())].into();
  app.on_net(NetEvent::Relay { platform: Platform::Youtube, key: "@abc".into(), msgs: vec![me], history: false });
}

fn ready(app: &mut App) {
  live(app);
  modded(app);
}

fn line(p: Platform, login: &str, id: &str, uid: &str, body: &str, age_s: i64) -> Message {
  let mut m = Message::system(now_ms() - age_s * 1000, p, body);
  m.kind = MsgKind::Chat;
  m.login = login.into();
  m.nick = login.into();
  m.id = id.into();
  m.user_id = uid.into();
  m
}

// (youtube's lines drip out at their own pace: these go straight onto the tab)
fn yt(app: &mut App, login: &str, id: &str, body: &str, age_s: i64) {
  let m = line(Platform::Youtube, login, id, UC, body, age_s);
  let i = app.active;
  app.tabs[i].chat().push(m);
}

fn twitch(app: &mut App, login: &str, id: &str, body: &str, age_s: i64) {
  let mut m = line(Platform::Twitch, login, id, "77", body, age_s);
  m.nick = login.into();
  app.on_chat("forsen", Chat::Msg(m));
}

fn typed(app: &mut App, s: &str) {
  app.line.set_text(s.into());
  app.send();
}

fn sent(rx: &mut Rx) -> Vec<Req> {
  std::iter::from_fn(|| rx.try_recv().ok())
    .filter_map(|c| match c {
      Cmd::Mod(r) => Some(r),
      _ => None,
    })
    .collect()
}

fn who(reqs: &[Req]) -> Vec<(Platform, String, String, Verb)> {
  reqs.iter().map(|r| (r.platform, r.chan.to_string(), r.login.to_string(), r.verb)).collect()
}

fn lines(app: &App) -> Vec<String> {
  app.tabs[app.active].view.msgs().map(|m| m.body.to_string()).collect()
}

fn has(app: &App, text: &str) -> bool {
  lines(app).iter().any(|l| l == text)
}

#[test]
fn a_live_youtube_we_moderate_joins_every_command_like_twitch_and_kick() {
  let (mut app, mut rx) = boot();
  ready(&mut app);
  twitch(&mut app, "bob", "t1", "hi", 1);
  yt(&mut app, "@bob", "y1", "hi", 1);
  typed(&mut app, "/ban bob spam");
  let got = sent(&mut rx);
  assert_eq!(
    who(&got),
    [
      (Platform::Twitch, "forsen".into(), "bob".into(), Verb::Ban),
      (Platform::Kick, "xqc".into(), "bob".into(), Verb::Ban),
      (Platform::Youtube, "@abc".into(), "@bob".into(), Verb::Ban),
    ]
  );
  // youtube names its room by handle, the person by the channel id off their line; no twitch room id
  let y = &got[2];
  assert_eq!((y.user_id.as_deref(), y.room.as_deref(), y.msg_id.as_deref()), (Some(UC), None, None));
  assert_eq!(got[0].user_id.as_deref(), Some("77"), "twitch still resolves its own id");
  assert!(!has(&app, NOT_READY));
  // the rest of the verbs fan out to all three the same way
  for (cmd, verb) in [
    ("/timeout bob 30", Verb::Timeout(30)),
    ("/timeout bob", Verb::Timeout(600)),
    ("/unban bob", Verb::Unban),
    ("/unto bob", Verb::Unban),
    ("/untimeout bob", Verb::Unban),
    ("/purge bob", Verb::Purge),
  ] {
    typed(&mut app, cmd);
    let got = sent(&mut rx);
    let all: Vec<_> = got.iter().map(|r| (r.platform, r.verb, r.user_id.as_deref())).collect();
    assert_eq!(
      all,
      [(Platform::Twitch, verb, Some("77")), (Platform::Kick, verb, None), (Platform::Youtube, verb, Some(UC))],
      "{cmd}"
    );
  }
}

#[test]
fn a_typed_handle_matches_with_or_without_its_at_and_in_any_case() {
  let (mut app, mut rx) = boot();
  ready(&mut app);
  yt(&mut app, "@bob", "y1", "hi", 1);
  twitch(&mut app, "zed", "t1", "hi", 1);
  // a login stored without its @ is found by either spelling too
  let m = line(Platform::Youtube, "amy", "y2", "UCamyamyamyamyamyamyamy1", "hi", 1);
  let i = app.active;
  app.tabs[i].chat().push(m);
  for typed_name in ["/ban bob", "/ban @bob", "/ban @@Bob", "/ban BOB"] {
    typed(&mut app, typed_name);
    let got = sent(&mut rx);
    let y = got.iter().find(|r| r.platform == Platform::Youtube).expect(typed_name);
    assert_eq!((&*y.login, y.user_id.as_deref()), ("@bob", Some(UC)), "{typed_name}");
  }
  for typed_name in ["/ban amy", "/ban @amy"] {
    typed(&mut app, typed_name);
    let got = sent(&mut rx);
    let y = got.iter().find(|r| r.platform == Platform::Youtube).expect(typed_name);
    assert_eq!((&*y.login, y.user_id.as_deref()), ("amy", Some("UCamyamyamyamyamyamyamy1")), "{typed_name}");
  }
  // the lookup is youtube's own: a twitch chatter of the same name has no youtube id to lend
  let v = &app.tabs[app.active].view;
  assert_eq!(super::mods::yt_who(v.msgs(), "@BOB").map(|w| w.1), Some(UC.into()));
  assert!(super::mods::yt_who(v.msgs(), "zed").is_none(), "a twitch chatter has no youtube id to lend");
  assert!(super::mods::yt_who(v.msgs(), "nobody").is_none() && super::mods::yt_who(v.msgs(), "").is_none());
}

#[test]
fn a_youtube_not_live_or_not_ours_is_left_out_and_said_while_twitch_and_kick_run() {
  let platforms = |rx: &mut Rx| sent(rx).iter().map(|r| r.platform).collect::<Vec<_>>();
  let both = [Platform::Twitch, Platform::Kick];
  // modded, not live
  let (mut app, mut rx) = boot();
  modded(&mut app);
  yt(&mut app, "@bob", "y1", "hi", 1);
  typed(&mut app, "/ban bob");
  assert_eq!(platforms(&mut rx), both);
  assert!(has(&app, NOT_READY));
  // live, not a mod (never spoke there): the same
  let (mut app, mut rx) = boot();
  live(&mut app);
  yt(&mut app, "@bob", "y1", "hi", 1);
  typed(&mut app, "/timeout bob");
  assert_eq!(platforms(&mut rx), both);
  assert!(has(&app, NOT_READY));
  // our line there without the badge: unmodded again
  ready(&mut app);
  let m = line(Platform::Youtube, "@mellenyt", "me2", "UCmellenmellenmellenmell", "hi", 0);
  app.on_net(NetEvent::Relay { platform: Platform::Youtube, key: "@abc".into(), msgs: vec![m], history: false });
  typed(&mut app, "/ban bob");
  assert_eq!(platforms(&mut rx), both);
  // a youtube that ended (or has chat off) is not live
  let (mut app, mut rx) = boot();
  ready(&mut app);
  app.on_net(NetEvent::YtStatus { key: "@abc".into(), status: "ended".into() });
  yt(&mut app, "@bob", "y1", "hi", 1);
  typed(&mut app, "/ban bob");
  assert_eq!(platforms(&mut rx), both);
  // a tab with no youtube says nothing of it, and sends what it always did
  let (mut app, mut rx) = boot_tab("forsen kick:xqc");
  typed(&mut app, "/ban bob");
  assert_eq!(platforms(&mut rx), both);
  assert!(!lines(&app).iter().any(|l| l.contains("youtube")));
}

#[test]
fn a_name_not_seen_in_the_tabs_youtube_lines_skips_that_leg_only() {
  let (mut app, mut rx) = boot();
  ready(&mut app);
  twitch(&mut app, "zed", "t1", "hi", 1);
  yt(&mut app, "@bob", "y1", "hi", 1);
  typed(&mut app, "/ban zed");
  let got = sent(&mut rx);
  assert_eq!(got.iter().map(|r| r.platform).collect::<Vec<_>>(), [Platform::Twitch, Platform::Kick]);
  assert!(has(&app, "youtube skipped: zed not seen in this tab"));
  assert!(!has(&app, NOT_READY));
  // the status line after it is still the send
  assert_eq!(app.tabs[app.active].view.last_msg().map(|m| &*m.body), Some("ban zed\u{2026}"));
  // a person who only spoke on twitch has no channel id to lend: same
  typed(&mut app, "/unban @zed");
  assert_eq!(sent(&mut rx).len(), 2);
  // a youtube-only tab with nobody to name sends nothing and says why, once
  let (mut app, mut rx) = yt_only();
  ready(&mut app);
  typed(&mut app, "/ban zed");
  assert!(sent(&mut rx).is_empty());
  assert!(has(&app, "youtube skipped: zed not seen in this tab"));
  assert!(!lines(&app).iter().any(|l| l.contains("needs a twitch or kick")));
  // ready and named: it goes
  yt(&mut app, "@zed", "y9", "hi", 1);
  typed(&mut app, "/ban zed");
  assert_eq!(who(&sent(&mut rx)), [(Platform::Youtube, "@abc".into(), "@zed".into(), Verb::Ban)]);
}

#[test]
fn a_youtube_only_tab_that_is_not_ready_says_so_instead_of_naming_twitch() {
  let (mut app, mut rx) = yt_only();
  yt(&mut app, "@bob", "y1", "hi", 1);
  typed(&mut app, "/ban bob");
  assert!(!std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, Cmd::Nuke(..) | Cmd::Mod(_))));
  assert_eq!(lines(&app).iter().filter(|l| *l == NOT_READY).count(), 1);
  assert!(!lines(&app).iter().any(|l| l.contains("needs a twitch or kick")));
  // logged out says that, not this
  app.on_net(NetEvent::Me(None));
  typed(&mut app, "/ban bob");
  assert_eq!(app.tabs[app.active].view.last_msg().map(|m| &*m.body), Some("log in first (ctrl-l)"));
}

#[test]
fn delete_by_id_goes_to_youtube_when_the_line_is_a_youtube_one_and_anywhere_else_to_twitch_first() {
  let (mut app, mut rx) = boot();
  ready(&mut app);
  twitch(&mut app, "bob", "t1", "hi", 1);
  yt(&mut app, "@bob", "Chw_k-1", "buy followers", 2);
  let ts = app.tabs[app.active].view.msgs().find(|m| &*m.id == "Chw_k-1").map(|m| m.ts).unwrap();
  // youtube's id has `_` and `-` in it: it parses, finds its line in this tab, and rides the line menu's own request
  typed(&mut app, "/delete Chw_k-1");
  let got = sent(&mut rx);
  assert_eq!(got.len(), 1);
  let r = &got[0];
  assert_eq!(
    (r.platform, &*r.chan, r.verb, r.msg_id.as_deref()),
    (Platform::Youtube, "@abc", Verb::Delete, Some("Chw_k-1"))
  );
  assert_eq!((r.user_id.as_deref(), r.msg_ts, r.msg_text.as_deref()), (Some(UC), Some(ts), Some("buy followers")));
  assert_eq!(app.tabs[app.active].view.last_msg().map(|m| &*m.body), Some("delete message\u{2026}"));
  // a twitch id, and one nobody here holds: twitch first, as before
  for id in ["t1", "zz9"] {
    typed(&mut app, &format!("/delete {id}"));
    let got = sent(&mut rx);
    assert_eq!(got.len(), 1, "{id}");
    assert_eq!((got[0].platform, got[0].msg_id.as_deref()), (Platform::Twitch, Some(id)));
  }
  // a youtube line while youtube is not ready: nothing goes (and it says why), not a try on twitch
  let (mut app, mut rx) = boot();
  modded(&mut app);
  twitch(&mut app, "bob", "t1", "hi", 1);
  yt(&mut app, "@bob", "y1", "hi", 1);
  typed(&mut app, "/delete y1");
  assert!(sent(&mut rx).is_empty());
  assert!(has(&app, NOT_READY));
  // a youtube-only tab: an id it never saw says so
  let (mut app, mut rx) = yt_only();
  ready(&mut app);
  typed(&mut app, "/delete zz9");
  assert!(sent(&mut rx).is_empty());
  assert!(has(&app, "youtube skipped: zz9 not seen in this tab"));
}

#[test]
fn nuke_leaves_youtube_out_and_says_so_only_when_youtube_lines_matched() {
  let nuke = |app: &mut App, rx: &mut Rx| {
    typed(app, "/nuke buy");
    let head = app.pop().and_then(|p| p.menu.head().map(str::to_owned));
    if head.is_some() {
      app.pop_key(Key::Named(NamedKey::Enter));
    }
    let reqs: Vec<Req> = std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| match c {
        Cmd::Nuke(r, _) => Some(r),
        _ => None,
      })
      .flatten()
      .collect();
    (head, reqs)
  };
  let skipped = "youtube skipped: nuke is twitch and kick only (youtube's daily limit)";
  // a ready youtube too: its matching lines stay out, one line says so, twitch goes as before
  let (mut app, mut rx) = boot();
  ready(&mut app);
  twitch(&mut app, "bob", "t1", "BUY followers", 5);
  yt(&mut app, "@bob", "y1", "buy now", 4);
  yt(&mut app, "@bob", "y2", "buy again", 3);
  app.redraw();
  let (head, reqs) = nuke(&mut app, &mut rx);
  assert_eq!(head.as_deref(), Some("delete 1 message matching \"buy\" in #forsen?"));
  assert_eq!(
    reqs.iter().map(|r| (r.platform, r.msg_id.as_deref())).collect::<Vec<_>>(),
    [(Platform::Twitch, Some("t1"))]
  );
  assert_eq!(lines(&app).iter().filter(|l| *l == skipped).count(), 1, "said once, not per line");
  assert!(!has(&app, NOT_READY));
  // youtube not ready: the same, and nothing about "not live"
  let (mut app, mut rx) = boot();
  twitch(&mut app, "bob", "t1", "buy", 5);
  yt(&mut app, "@bob", "y1", "buy now", 4);
  app.redraw();
  let (_, reqs) = nuke(&mut app, &mut rx);
  assert_eq!(reqs.len(), 1);
  assert!(has(&app, skipped) && !has(&app, NOT_READY));
  // youtube lines that don't match the term (or are too old) earn no line
  let (mut app, mut rx) = boot();
  ready(&mut app);
  twitch(&mut app, "bob", "t1", "buy", 5);
  yt(&mut app, "@bob", "y1", "hello", 4);
  yt(&mut app, "@bob", "y2", "buy late", 100);
  app.redraw();
  let (_, reqs) = nuke(&mut app, &mut rx);
  assert_eq!(reqs.len(), 1);
  assert!(!has(&app, skipped));
  // a tab with no youtube says nothing of it
  let (mut app, mut rx) = boot_tab("forsen kick:xqc");
  twitch(&mut app, "bob", "t1", "buy", 5);
  app.redraw();
  let (_, reqs) = nuke(&mut app, &mut rx);
  assert_eq!(reqs.len(), 1);
  assert!(!lines(&app).iter().any(|l| l.contains("youtube")));
  // a youtube-only tab has no twitch or kick feed to nuke in, ready or not
  let (mut app, mut rx) = yt_only();
  ready(&mut app);
  yt(&mut app, "@bob", "y1", "buy now", 4);
  let (head, reqs) = nuke(&mut app, &mut rx);
  assert!(head.is_none() && reqs.is_empty());
  assert!(has(&app, "/nuke needs a twitch or kick channel"));
}

#[test]
fn a_ban_greys_a_youtube_persons_lines_and_the_servers_refusal_comes_through() {
  let (mut app, _rx) = boot();
  ready(&mut app);
  yt(&mut app, "@bob", "y1", "hi", 1);
  yt(&mut app, "@amy", "y2", "hi", 1);
  yt(&mut app, "@amy", "y3", "hi", 1);
  let cleared =
    |app: &App, id: &str| app.tabs[app.active].view.msgs().any(|m| &*m.id == id && m.flags & flag::CLEARED != 0);
  // a ban greys that person's youtube lines (their login there is `@handle`)
  let done = |verb, login: &str, msg: Option<&str>, res| Done {
    platform: Platform::Youtube,
    chan: "@abc".into(),
    verb,
    login: login.into(),
    msg_id: msg.map(Into::into),
    res,
  };
  app.on_mod_done(done(Verb::Ban, "@bob", None, Ok(())));
  assert!(cleared(&app, "y1") && !cleared(&app, "y2"));
  // an unban of a ban heatsync didn't make: the server's own words, not ours
  let why = "heatsync didn't make that ban \u{2014} unban it on youtube";
  app.on_mod_done(done(Verb::Unban, "@amy", None, Err(Fail::Other(why.into()))));
  assert_eq!(app.tabs[app.active].view.last_msg().map(|m| &*m.body), Some(format!("unban failed: {why}").as_str()));
  assert!(!cleared(&app, "y3"));
}
