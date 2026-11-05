// the notifs tab (ext background.js `notification:new`): heatsync's own notifications, a mention of you,
// a reply to your post, a fixed report. the ext shows only a count on its toolbar badge; here the count
// rides the tab and each one that arrives is a row. looking at the tab reads them all (here, on the
// account, and for your other clients through seen:update); enter or a click on a row goes where it
// points: a post opens in the feed tab, a page opens in the browser
use super::App;
use crate::model::inbox::{self, Item};
use crate::model::tabs::Kind;
use crate::model::unseen::Surface;
use crate::net::{Cmd, ORIGIN, social};
use crate::util::time::now_ms;

impl App {
  // what the empty tab says: it depends on who you are
  pub(super) fn notifs_empty(&mut self) {
    let text = match self.me {
      None => "log in to see your heatsync notifications (ctrl-l)",
      Some(_) => "nothing yet. mentions of you, replies to your posts and fixed reports land here",
    };
    if let Some(i) = self.tab_of(Kind::Notifs) {
      self.tabs[i].chat().set_empty(text);
    }
  }

  // a different login (or none): the old person's notifications must not stay on screen
  pub(super) fn notifs_reset(&mut self) {
    self.notifs.clear();
    if let Some(i) = self.tab_of(Kind::Notifs) {
      self.tabs[i].chat().clear();
      self.tabs[i].unread = false;
    }
    self.notifs_empty();
    self.bar_dirty = true;
    self.wake();
  }

  // the socket signed in: the account's count is the truth, whatever this window missed while it was away
  pub(super) fn notifs_ask(&mut self) {
    if self.me.is_some() {
      self.net.send(Cmd::Social(social::Cmd::NotifsGet));
    }
  }

  pub(super) fn on_notif(&mut self, it: Item) {
    // a frame for nobody: only a signed-in account is sent these
    if self.me.is_none() {
      return;
    }
    let at = it.at;
    let it = self.notifs.push(it).clone();
    if let Some(i) = self.tab_of(Kind::Notifs) {
      let m = inbox::message(&it);
      self.tabs[i].chat().push(m);
    }
    self.unseen_note_at(Surface::Inbox, at);
  }

  pub(super) fn on_notif_count(&mut self, n: u32) {
    if self.me.is_none() {
      return;
    }
    let now = now_ms();
    self.notifs.hydrate(n, now);
    if n == 0 {
      // nothing unread anywhere: whatever lit it was read on another client
      self.on_seen(Surface::Inbox, now);
    } else {
      self.unseen_note(Surface::Inbox);
    }
    self.bar_dirty = true;
    self.wake();
  }

  // the number after the tab's name
  pub(super) fn notifs_count(&self) -> String {
    match self.notifs.unread() {
      0 => String::new(),
      n => n.to_string(),
    }
  }

  // enter / click on a row: a post opens in the feed, a page opens in the browser
  pub(super) fn notifs_open(&mut self) {
    let Some(t) = self.tabs.get(self.active).filter(|t| t.kind == Kind::Notifs) else { return };
    let Some(row) = t.view.selected().map(|m| m.id.clone()) else { return };
    let Some(it) = self.notifs.find(&row).cloned() else { return };
    if let Some(id) = it.post {
      // the feed tab is where posts open; bring it back if it was hidden
      if !self.set.shown(Kind::Feed) {
        self.set_shown(Kind::Feed, true);
      }
      let Some(i) = self.tab_of(Kind::Feed) else { return };
      self.select(i);
      self.feed_open(&id);
    } else if let Some(path) = it.path {
      self.open_link(&format!("{ORIGIN}{path}"));
    } else {
      self.say("that one has nothing to open");
    }
    self.wake();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::{Net, NetEvent};
  use serde_json::json;

  fn me() -> crate::net::auth::Me {
    crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: None,
      twitch_id: None,
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    }
  }

  // a frame that came in `ago` ms back
  fn frame_ago(v: serde_json::Value, ago: i64) -> NetEvent {
    NetEvent::Notif(inbox::parse(&v, now_ms() - ago).expect("a notification"))
  }

  fn frame(v: serde_json::Value) -> NetEvent {
    frame_ago(v, 0)
  }

  fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<&'static str> {
    let mut out = Vec::new();
    while let Ok(c) = rx.try_recv() {
      out.push(match c {
        Cmd::Social(social::Cmd::NotifsRead) => "read",
        Cmd::Social(social::Cmd::NotifsGet) => "get",
        Cmd::Social(social::Cmd::SeenBump { surface: Surface::Inbox, .. }) => "inbox bump",
        _ => "other",
      });
    }
    out
  }

  // a frame in lights the tab and counts it; the account's count adds the older ones; a look on another
  // client puts it out (only if it came after), and so does opening the tab, which tells the account
  #[test]
  fn a_notification_lights_the_tab_and_looking_reads_it() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.me = Some(me());
    let tab = app.tab_of(Kind::Notifs).unwrap();
    assert!(!app.tabs[tab].unread && app.notifs_count().is_empty());

    let first = json!({"type": "notification:new", "kind": "mention", "message_id": "ab12", "content": "hi"});
    app.on_net(frame_ago(first, 10_000));
    assert!(app.tabs[tab].unread);
    assert_eq!(app.notifs_count(), "1");
    assert_eq!(app.tabs[tab].view.len(), 1);
    assert_eq!(&*app.tabs[tab].view.last_msg().unwrap().body, "hi");

    asked(&mut rx);
    // one more, then open the tab: read here, the account told once, the site's inbox time left alone
    app.on_net(frame(json!({"kind": "reply", "message_id": "cd34", "parent_is_op": true})));
    assert_eq!(app.notifs_count(), "2");
    app.select(tab);
    assert!(!app.tabs[tab].unread && app.notifs_count().is_empty());
    assert_eq!(asked(&mut rx), ["read"]);
    // looking again with nothing new asks for nothing
    app.unseen_clear(Surface::Inbox);
    assert!(asked(&mut rx).is_empty());
  }

  // the account says none are unread (read on the site): a lit tab goes out
  #[test]
  fn the_accounts_count_of_none_clears_it() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.me = Some(me());
    let tab = app.tab_of(Kind::Notifs).unwrap();
    app.on_net(NetEvent::NotifCount(3));
    assert!(app.tabs[tab].unread);
    assert_eq!(app.notifs_count(), "3");
    // one comes in on top of the account's three, and the account's next count takes it in
    app.on_net(frame(json!({"kind": "system", "content": "hi"})));
    assert_eq!(app.notifs_count(), "4");
    app.on_net(NetEvent::NotifCount(4));
    assert_eq!(app.notifs_count(), "4");
    // read on the site: the account says none
    app.on_net(NetEvent::NotifCount(0));
    assert!(!app.tabs[tab].unread && app.notifs_count().is_empty());
  }

  // logged out nothing is kept, and a different login starts clean
  #[test]
  fn nobody_signed_in_keeps_nothing() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.on_net(frame(json!({"kind": "system", "content": "hello"})));
    app.on_net(NetEvent::NotifCount(5));
    let tab = app.tab_of(Kind::Notifs).unwrap();
    assert!(!app.tabs[tab].unread && app.tabs[tab].view.len() == 0);
    app.me = Some(me());
    app.on_net(frame(json!({"kind": "system", "content": "hello"})));
    assert_eq!(app.tabs[tab].view.len(), 1);
    app.notifs_reset();
    assert!(app.tabs[tab].view.len() == 0 && app.notifs_count().is_empty());
  }

  // enter on a mention opens its post in the feed tab, as a thread
  #[test]
  fn enter_on_a_post_notification_opens_it_in_the_feed() {
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.canvas.resize(720, 360);
    app.me = Some(me());
    app.on_net(frame(json!({"kind": "mention", "message_id": "ab12", "content": "look"})));
    let tab = app.tab_of(Kind::Notifs).unwrap();
    app.select(tab);
    let (id, h) = (inbox::row_id(1), app.chat_area().h);
    let res = &mut app.res;
    assert!(app.tabs[tab].view.select_id(&id, res, h));
    app.notifs_open();
    let feed = app.tab_of(Kind::Feed).unwrap();
    assert_eq!(app.active, feed);
    assert_eq!(app.feed.thread.as_ref().map(|t| &*t.id), Some("ab12"));
  }
}
