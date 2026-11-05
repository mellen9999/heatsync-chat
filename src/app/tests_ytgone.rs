// youtube:delete, headless: a deleted line (or all of a banned author's) greys in place, like any delete

use super::*;
use crate::model::message::{Kind as MsgKind, Message, flag};
use crate::net::ytdel::Gone;

fn yt(id: &str, author: &str) -> Message {
  let mut m = Message::system(now_ms(), Platform::Youtube, "hi");
  m.kind = MsgKind::Chat;
  m.login = "al".into();
  m.nick = "al".into();
  m.id = id.into();
  m.user_id = author.into();
  m
}

#[test]
fn a_youtube_delete_greys_the_named_lines_and_a_banned_authors_backlog() {
  let (net, _rx) = Net::tap();
  let mut app = App::new(net, vec!["yt:@abc".into()], false);
  app.dpi = 1.0;
  app.res.set_scale(app.scale());
  app.resized(520, 300);
  let t = app.tabs_with(Platform::Youtube, "@abc")[0];
  let msgs = vec![yt("m1", "UCa"), yt("m2", "UCb"), yt("m3", "UCc"), yt("m4", "UCb")];
  app.on_net(NetEvent::Relay { platform: Platform::Youtube, key: "@abc".into(), msgs, history: false });
  let gone = Gone { ids: vec!["m1".into()], authors: vec!["UCb".into()] };
  app.on_net(NetEvent::YtGone { key: "@abc".into(), gone });
  let cleared: Vec<(String, bool)> =
    app.tabs[t].chat().msgs().map(|m| (m.id.to_string(), m.flags & flag::CLEARED != 0)).collect();
  assert_eq!(
    cleared,
    [("m1".to_owned(), true), ("m2".to_owned(), true), ("m3".to_owned(), false), ("m4".to_owned(), true)]
  );
  // another room's key touches nothing
  app.on_net(NetEvent::YtGone { key: "@zzz".into(), gone: Gone { ids: vec!["m3".into()], authors: vec![] } });
  assert!(app.tabs[t].chat().msgs().any(|m| &*m.id == "m3" && m.flags & flag::CLEARED == 0));
}
