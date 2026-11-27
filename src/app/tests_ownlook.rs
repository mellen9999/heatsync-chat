// your own line on kick wears your twitch badges and colour (model::own_look), the whole app headless: mellen
// sends to twitch and kick in nl_kripp, kick's echo is the row that stands, and a real twitch history line of his
// (tags as the history service recorded them) says what his twitch self looks like there
use super::*;
use crate::model::message::Message;
use crate::net::auth::Me;
use crate::net::irc::{self, Chat};

const REAL: &str = "@first-msg=0;color=#FF8700;returning-chatter=0;id=356aa42b-34de-4a4f-a2f2-ebbf6b77ac16;badges=moderator/1,subscriber/108;subscriber=1;rm-received-ts=1795792290209;tmi-sent-ts=1795792290049;emotes;badge-info=subscriber/110;room-id=29795919;flags;turbo=0;user-id=73266147;display-name=mellen;mod=1;user-type=mod;historical=1 :mellen!mellen@mellen.tmi.twitch.tv PRIVMSG #nl_kripp :gm there";
const ORANGE: u32 = 0xff8700;
const KICK_GREEN: u32 = 0x75fd46;

fn app() -> App {
  let (net, _rx) = Net::tap();
  let mut a = App::new(net, vec!["nl_kripp kick:nl_kripp".into()], false);
  a.dpi = 1.0;
  a.res.set_scale(a.scale());
  a.resized(900, 400);
  a.on_net(NetEvent::Me(Some(Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: Some(73266147),
    names: vec!["mellen".into()],
    kick: Some("mellen".into()),
    youtube: None,
    packs: vec![],
  })));
  a
}

fn kick_line(a: &mut App) {
  let mut m = Message::system(now_ms(), Platform::Kick, "x");
  (m.kind, m.id, m.login, m.nick, m.body) =
    (crate::model::message::Kind::Chat, "k1".into(), "mellen".into(), "mellen".into(), "or leftover tkvg".into());
  m.color = KICK_GREEN;
  a.on_relay(Platform::Kick, "nl_kripp", vec![m], false);
}

fn pixels(a: &mut App, c: u32) -> usize {
  a.redraw();
  a.canvas.px.iter().filter(|p| (**p & 0xff_ffff) == c).count()
}

fn twitch_history() -> NetEvent {
  let (_, ev) = irc::to_chat(&irc::parse(REAL).unwrap()).unwrap();
  NetEvent::Backfill("nl_kripp".into(), vec![ev])
}

#[test]
fn history_of_your_twitch_self_dresses_your_kick_line_already_on_screen() {
  let mut a = app();
  kick_line(&mut a);
  assert!(pixels(&mut a, KICK_GREEN) > 0 && pixels(&mut a, ORANGE) == 0, "kick's own colour before");
  a.on_net(twitch_history());
  let (green, orange) = (pixels(&mut a, KICK_GREEN), pixels(&mut a, ORANGE));
  assert!(green == 0 && orange > 0, "the name reads as your twitch name: green {green}, orange {orange}");
  let look = a.res.own.get("nl_kripp").expect("your twitch look in the room");
  assert_eq!((look.color, look.badges.len()), (ORANGE, 2), "mod + sub");
}

#[test]
fn a_live_twitch_line_of_yours_does_the_same_even_when_it_folds_away() {
  let mut a = app();
  // the send to both feeds: kick's echo comes first and stands, so the twitch copy is the one that folds
  a.sentfold.track(now_ms(), "or leftover tkvg", false);
  kick_line(&mut a);
  let (_, ev) = irc::to_chat(&irc::parse(REAL).unwrap()).unwrap();
  let Chat::Msg(mut m) = ev else { panic!() };
  m.flags &= !crate::model::message::flag::HISTORY;
  m.body = "or leftover tkvg".into();
  m.ts = now_ms();
  a.on_chat("nl_kripp", Chat::Msg(m));
  assert!(a.tabs[0].view.msgs().all(|m| m.platform == Platform::Kick), "the twitch copy folded away");
  assert!(a.res.own.get("nl_kripp").is_some_and(|l| l.color == ORANGE && l.badges.len() == 2));
  assert!(pixels(&mut a, KICK_GREEN) == 0 && pixels(&mut a, ORANGE) > 0);
}
