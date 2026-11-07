// take back one of your own heatsync dms: `x` on its row in the whispers tab. it asks first, because the other
// person loses it too and there is no undo. heatsync's answer takes the row out of the list, and so does its
// `dm:message-deleted` push, which also reaches the other person's window and your other windows
use super::App;
use super::feed::LOGIN_FIRST;
use super::overlay::Overlay;
use super::pop::{Act, Pop};
use crate::model::tabs::Kind;
use crate::model::whisper::{self, Status, Via};
use crate::net::dmdel::Fail;
use crate::net::{Cmd, social};

// how much of their name the question shows
const NAME_SHOWN: usize = 20;

impl App {
  // `x` on a picked whispers row: ask, or say in plain words why that row can't go
  pub(super) fn dm_delete_ask(&mut self) {
    let Some(w) = self.selected_whisper() else { return };
    let refuse = if w.who.via != Via::Heatsync {
      "twitch whispers can't be deleted"
    } else if !w.out {
      "you can only delete your own dms"
    } else if matches!(w.status, Status::Failed(_)) {
      "that dm never sent, so there's nothing to delete"
    } else if w.status != Status::Sent || w.dm_id().is_none() || w.who.id.is_empty() {
      // heatsync's id for the dm lands just before its answer: until the answer, who it went to is not settled
      "that dm isn't on heatsync yet: try again in a moment"
    } else {
      self.whisper.pending_delete = Some(w.key);
      let name: String = w.who.name.chars().take(NAME_SHOWN).collect();
      let chat = self.chat_area();
      let rows = vec![(Act::DeleteDm, "yes, delete for both".to_owned()), (Act::No, "cancel".to_owned())];
      let at = (chat.x + 24, chat.y + chat.h / 4);
      self.overlay = Some(Overlay::Pop(Pop::ask(format!("delete dm to {name}?"), at, rows)));
      self.pop_changed();
      return;
    };
    self.say(refuse);
  }

  // `m` / right-click on a whispers row: "delete dm" for your own heatsync dm whose id is known. any other row
  // has nothing to offer, so no menu opens
  pub(super) fn dm_line_menu(&mut self, at: (i32, i32)) {
    if self.selected_whisper().is_none_or(|w| w.dm_id().is_none()) {
      return;
    }
    let rows = vec![(Act::DmAsk, "delete dm".to_owned())];
    self.overlay = Some(Overlay::Pop(Pop::ask("dm".to_owned(), at, rows)));
    self.pop_changed();
  }

  // "yes, delete for both"
  pub(super) fn dm_delete_go(&mut self) {
    let Some(key) = self.whisper.pending_delete.take() else { return };
    // the row can be gone by now (the other person took it back while the question was open)
    let Some(w) = self.whisper.timeline.get(key) else { return };
    let Some(id) = w.dm_id() else { return };
    self.net.send(Cmd::Social(social::Cmd::DmDelete { with: w.who.id.clone(), id: id.into() }));
    self.say("deleting dm…");
  }

  // heatsync's answer to our delete: gone, or why not
  pub(super) fn on_dm_deleted(&mut self, id: &str, res: Result<(), Fail>) {
    let words = match res {
      Ok(()) => {
        self.dm_drop(&whisper::dm_key(id));
        "dm deleted"
      }
      Err(Fail::Gone) => {
        self.dm_drop(&whisper::dm_key(id));
        "that dm was already gone"
      }
      Err(Fail::Auth) => LOGIN_FIRST,
      Err(Fail::Refused) => "heatsync won't let you delete that dm",
      Err(Fail::NoThread) => "couldn't find that conversation on heatsync",
      Err(Fail::Limited) => "slow down: try again in a moment",
      Err(Fail::Other) => "couldn't delete the dm: try again",
    };
    self.say(words);
  }

  // a dm taken down by one of its two people, on any client (`dm:message-deleted`): no word about it, the row
  // just leaves
  pub(super) fn on_dm_gone(&mut self, key: &str) {
    self.dm_drop(key);
  }

  // heatsync stored a send of ours: its row learns the id, so it can be taken back
  pub(super) fn on_dm_stored(&mut self, key: u64, id: &str) {
    self.whisper.timeline.stamp(key, whisper::dm_key(id));
  }

  // the row for `key` leaves the list and the tab. the tab is rebuilt from the list (scroll and the pick stay,
  // the stripes stay alternating)
  fn dm_drop(&mut self, key: &str) {
    if !self.whisper.timeline.drop_ext(key) {
      return;
    }
    let hide = self.hides_whispers();
    let rows: Vec<_> = self.whisper.timeline.iter().map(|w| whisper::message(w, hide)).collect();
    if let Some(i) = self.tab_of(Kind::Whispers) {
      self.tabs[i].chat().reload(rows);
    }
    self.input_dirty = true;
    self.wake();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::whisper::{Fail as SendFail, Whisper, Who};
  use crate::net::{Net, NetEvent};
  use winit::keyboard::Key;

  type Asked = tokio::sync::mpsc::UnboundedReceiver<Cmd>;

  fn amy(via: Via) -> Who {
    Who { via, id: "77".into(), login: "amy".into(), name: "amy".into(), color: 0xffffff }
  }

  // signed in, the whispers tab up
  fn boot() -> (App, Asked) {
    let (net, asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.me = Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: None,
      twitch_id: None,
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.canvas.resize(720, 360);
    for t in &mut app.tabs {
      t.view.resize(720);
    }
    let i = app.tab_of(Kind::Whispers).expect("a whispers tab");
    app.select(i);
    (app, asked)
  }

  // a row the server sent (history), in time order of the calls
  fn put(app: &mut App, ext: &str, out: bool, via: Via, text: &str) -> u64 {
    let ts = 1_000 + app.whisper.timeline.len() as i64;
    let w = Whisper { key: 0, ext: ext.into(), ts, out, who: amy(via), text: text.into(), status: Status::Sent };
    app.on_whispers(vec![w], false);
    app.whisper.timeline.iter().find(|w| &*w.text == text).expect("added").key
  }

  fn pick(app: &mut App, key: u64) {
    let h = app.chat_area().h;
    let i = app.active;
    assert!(app.tabs[i].view.select_id(&whisper::row_id(key), &mut app.res, h), "the row is on screen to pick");
  }

  fn press_x(app: &mut App) {
    assert!(app.pick_key(Key::Character("x")), "x is answered on a picked whispers row");
  }

  fn said(app: &App) -> String {
    app.tabs[app.active].view.last_msg().map(|m| m.body.to_string()).unwrap_or_default()
  }

  fn deletes(asked: &mut Asked) -> Vec<(String, String)> {
    std::iter::from_fn(|| asked.try_recv().ok())
      .filter_map(|c| match c {
        Cmd::Social(social::Cmd::DmDelete { with, id }) => Some((with.to_string(), id.to_string())),
        _ => None,
      })
      .collect()
  }

  fn asking(app: &App) -> Option<(String, Vec<String>)> {
    match &app.overlay {
      Some(Overlay::Pop(p)) => {
        Some((p.menu.head().unwrap_or_default().to_owned(), p.menu.items.iter().map(|r| r.1.clone()).collect()))
      }
      _ => None,
    }
  }

  fn shown(app: &App) -> Vec<String> {
    app.tabs[app.active].view.bodies()
  }

  #[test]
  fn x_on_a_row_that_cannot_go_says_why_and_asks_nothing() {
    let (mut app, mut asked) = boot();
    let whisper = put(&mut app, "a1", false, Via::Twitch, "a whisper");
    let theirs = put(&mut app, "dm1", false, Via::Heatsync, "their dm");
    // a send still on its way, and one that failed
    let pending = app.whisper.timeline.push_out(amy(Via::Heatsync), "on its way").key;
    let failed = app.whisper.timeline.push_out(amy(Via::Heatsync), "never sent").key;
    app.whisper.timeline.settle(failed, Status::Failed(SendFail::Server), None);
    // its id has come but the answer has not: still waiting on who it went to
    let stamped = app.whisper.timeline.push_out(amy(Via::Heatsync), "id first").key;
    app.whisper.timeline.stamp(stamped, whisper::dm_key("5"));
    for k in [pending, failed, stamped] {
      let w = app.whisper.timeline.get(k).unwrap().clone();
      app.put_row(&w);
    }
    for (key, why) in [
      (whisper, "twitch whispers can't be deleted"),
      (theirs, "you can only delete your own dms"),
      (pending, "isn't on heatsync yet"),
      (stamped, "isn't on heatsync yet"),
      (failed, "never sent"),
    ] {
      pick(&mut app, key);
      press_x(&mut app);
      assert!(said(&app).contains(why), "{why}: {}", said(&app));
      assert!(asking(&app).is_none(), "{why}: no question");
    }
    assert!(deletes(&mut asked).is_empty());
    assert_eq!(app.whisper.pending_delete, None);
  }

  #[test]
  fn the_line_menu_offers_delete_dm_on_your_own_dm_only_and_it_opens_the_same_confirm() {
    let (mut app, mut asked) = boot();
    let mine = put(&mut app, "dm2", true, Via::Heatsync, "mine");
    let theirs = put(&mut app, "dm1", false, Via::Heatsync, "their dm");
    let whisper = put(&mut app, "a1", true, Via::Twitch, "a whisper");
    for key in [theirs, whisper] {
      pick(&mut app, key);
      app.open_line_menu((10, 10));
      assert!(asking(&app).is_none(), "no menu, so no delete row");
    }
    pick(&mut app, mine);
    app.open_line_menu((10, 10));
    assert_eq!(asking(&app).map(|a| a.1), Some(vec!["delete dm".to_owned()]));
    app.pop_key(Key::Named(winit::keyboard::NamedKey::Enter));
    let (head, rows) = asking(&app).expect("the confirm");
    assert_eq!(head, "delete dm to amy?");
    assert_eq!(rows, ["yes, delete for both", "cancel"]);
    assert!(deletes(&mut asked).is_empty(), "nothing goes before the yes");
    assert_eq!(app.whisper.pending_delete, Some(mine));
  }

  #[test]
  fn your_own_dm_asks_first_and_only_yes_sends() {
    let (mut app, mut asked) = boot();
    let mine = put(&mut app, "dm2", true, Via::Heatsync, "mine");
    pick(&mut app, mine);
    press_x(&mut app);
    let (head, rows) = asking(&app).expect("a question");
    assert_eq!(head, "delete dm to amy?");
    assert_eq!(rows, ["yes, delete for both", "cancel"]);
    assert!(deletes(&mut asked).is_empty(), "nothing goes before the yes");
    app.act(Act::DeleteDm, None, None);
    assert_eq!(deletes(&mut asked), [("77".to_owned(), "2".to_owned())], "their id and the dm's");
    assert!(said(&app).contains("deleting dm"));
    assert_eq!(app.whisper.pending_delete, None);
    // the row stays until heatsync says so
    assert!(shown(&app).iter().any(|b| b == "mine"));
    // a second yes with nothing asked sends nothing
    app.act(Act::DeleteDm, None, None);
    assert!(deletes(&mut asked).is_empty());
  }

  #[test]
  fn heatsyncs_yes_takes_the_row_out_and_the_pick_stays_on_its_neighbour() {
    let (mut app, _asked) = boot();
    let keep = put(&mut app, "dm1", false, Via::Heatsync, "their dm");
    let mine = put(&mut app, "dm2", true, Via::Heatsync, "mine");
    pick(&mut app, keep);
    app.on_net(NetEvent::DmDeleted { id: "2".into(), res: Ok(()) });
    assert!(said(&app).contains("dm deleted"));
    assert!(app.whisper.timeline.get(mine).is_none());
    assert!(!shown(&app).iter().any(|b| b == "mine"), "gone from the tab too");
    assert!(shown(&app).iter().any(|b| b == "their dm"));
    let still = app.tabs[app.active].view.selected().map(|m| m.id.clone());
    assert_eq!(still.as_deref(), Some(&*whisper::row_id(keep)), "the pick did not move");
    // the push that follows it (heatsync tells every member) finds nothing left to do
    app.on_net(NetEvent::DmGone(whisper::dm_key("2")));
    assert_eq!(app.whisper.timeline.len(), 1);
  }

  #[test]
  fn a_dm_the_other_person_took_back_leaves_without_a_word() {
    let (mut app, _asked) = boot();
    put(&mut app, "dm1", false, Via::Heatsync, "their dm");
    put(&mut app, "dm2", true, Via::Heatsync, "mine");
    let before = shown(&app);
    app.on_net(NetEvent::DmGone(whisper::dm_key("1")));
    assert_eq!(app.whisper.timeline.len(), 1);
    assert_eq!(shown(&app), before.into_iter().filter(|b| b != "their dm").collect::<Vec<_>>());
    // a stranger's id, or a twitch whisper's, changes nothing
    put(&mut app, "dm7", false, Via::Twitch, "a whisper");
    app.on_net(NetEvent::DmGone(whisper::dm_key("999")));
    app.on_net(NetEvent::DmGone(whisper::dm_key("7")));
    assert_eq!(app.whisper.timeline.len(), 2);
  }

  #[test]
  fn a_refused_delete_keeps_the_row_and_says_why_in_plain_words() {
    let (mut app, _asked) = boot();
    put(&mut app, "dm2", true, Via::Heatsync, "mine");
    for (fail, words) in [
      (Fail::Auth, "log in"),
      (Fail::Refused, "won't let you delete that dm"),
      (Fail::NoThread, "couldn't find that conversation"),
      (Fail::Limited, "slow down"),
      (Fail::Other, "couldn't delete the dm"),
    ] {
      app.on_net(NetEvent::DmDeleted { id: "2".into(), res: Err(fail) });
      assert!(said(&app).contains(words), "{fail:?}: {}", said(&app));
      assert_eq!(app.whisper.timeline.len(), 1, "{fail:?}: the row stays");
    }
    // already gone on the server: it goes here too
    app.on_net(NetEvent::DmDeleted { id: "2".into(), res: Err(Fail::Gone) });
    assert!(said(&app).contains("already gone"));
    assert_eq!(app.whisper.timeline.len(), 0);
  }

  #[test]
  fn a_send_gets_its_id_so_it_can_go_and_its_history_copy_is_no_second_row() {
    let (mut app, mut asked) = boot();
    app.line.set_text("/dm amy hello".into());
    app.send();
    let key = app.whisper.timeline.iter().next().expect("the echo").key;
    // sent, but heatsync's id for it has not come: it can't go yet
    app.on_net(NetEvent::WhisperSent { key, res: Ok(amy(Via::Heatsync)) });
    pick(&mut app, key);
    press_x(&mut app);
    assert!(said(&app).contains("isn't on heatsync yet"));
    app.on_net(NetEvent::DmStored { key, id: "9".into() });
    press_x(&mut app);
    assert!(asking(&app).is_some());
    app.act(Act::DeleteDm, None, None);
    assert_eq!(deletes(&mut asked), [("77".to_owned(), "9".to_owned())]);
    // the server's own copy of the send, in a later history read, is the same row
    let copy = Whisper {
      key: 0,
      ext: "dm9".into(),
      ts: 1,
      out: true,
      who: amy(Via::Heatsync),
      text: "hello".into(),
      status: Status::Sent,
    };
    let epoch = Some(app.whisper.epoch);
    app.on_net(NetEvent::Whispers { items: vec![copy], live: false, epoch });
    assert_eq!(app.whisper.timeline.len(), 1);
  }
}
