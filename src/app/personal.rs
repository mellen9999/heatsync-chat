// 7tv personal emotes in the app: what the net hears lands in the store (emotes/personal.rs), the lines of
// whoever's set changed are laid out again, and the net is told which twitch rooms to listen in. the switch
// is `personal_emotes`; off, the store holds nothing and the net listens to nothing
use super::App;
use crate::emotes::personal::Ev;
use crate::net::Cmd;

// owners past this many in one event: lay everything out again instead of hunting each one's lines
const MANY: usize = 16;

impl App {
  // the twitch rooms of the open channels, as 7tv knows them (their numeric ids). None = off
  fn rooms_heard(&self) -> Option<Vec<u64>> {
    self.set.personal_emotes.then(|| self.rooms.iter().filter_map(|r| r.1.parse().ok()).collect())
  }

  // the net listens where we are: a channel opened or closed. nothing is sent while the switch is off
  pub(super) fn sync_personal(&self) {
    if self.set.personal_emotes {
      self.net.send(Cmd::SevenTvPersonal(self.rooms_heard()));
    }
  }

  // the switch flipped (or the account's settings said so): what is held goes, or starts to come
  pub(super) fn personal_switched(&mut self) {
    self.res.personal.set_on(self.set.personal_emotes);
    self.net.send(Cmd::SevenTvPersonal(self.rooms_heard()));
    self.relayout_where(None);
    self.input_dirty = true;
  }

  pub(super) fn on_personal(&mut self, ev: Ev) {
    let touched = self.res.personal.apply(ev);
    if touched.len() > MANY {
      return self.relayout_where(None);
    }
    for id in touched {
      self.touch_sender(id);
    }
    // the box and tab-complete read the store for your own set
    self.input_dirty = true;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::emotes::Emote;
  use crate::net::{Net, NetEvent};

  fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    let (net, rx) = Net::tap();
    let chans = ["forsen", "xqc", "kekw", "another"].map(Into::into).to_vec();
    (App::new(net, chans, false), rx)
  }

  fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Cmd> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
  }

  fn listening(cmds: &[Cmd]) -> Vec<Option<Vec<u64>>> {
    cmds.iter().filter_map(|c| if let Cmd::SevenTvPersonal(r) = c { Some(r.clone()) } else { None }).collect()
  }

  fn open(app: &mut App, ch: &str, id: &str) {
    app.on_net(NetEvent::Chat(ch.into(), crate::net::irc::Chat::Room { id: id.into(), modes: Default::default() }));
  }

  fn set_of(url: &str) -> crate::emotes::Set {
    [("DonkMonk".into(), Emote { url: url.into(), w: 28, h: 28, zero_width: false })].into()
  }

  fn own(app: &mut App, users: &[u64], set: &str, url: &str) {
    app.on_net(NetEvent::Personal(Ev::Grant { users: users.to_vec(), set: set.into() }));
    app.on_net(NetEvent::Personal(Ev::Set(set.into(), set_of(url))));
  }

  fn word(app: &mut App, who: &str, w: &str) -> bool {
    use crate::model::message::{Kind, Message, Platform};
    let mut m = Message::system(1, Platform::Twitch, "x");
    (m.kind, m.user_id, m.body) = (Kind::Chat, who.into(), w.into());
    crate::emotes::resolve::find_word(&mut app.res.sources(), "forsen", &m, 0, w.len(), w).is_some()
  }

  // the net hears the rooms that are open and no others, a closed one is let go, and with the switch off
  // it hears nothing and is told nothing more
  #[test]
  fn the_net_is_told_which_rooms_to_listen_in() {
    let (mut app, mut rx) = boot();
    sent(&mut rx);
    open(&mut app, "forsen", "22484632");
    open(&mut app, "xqc", "71092938");
    open(&mut app, "kekw", "not a number");
    assert_eq!(listening(&sent(&mut rx)).last(), Some(&Some(vec![22484632, 71092938])), "ids only, in open order");
    app.set.personal_emotes = false;
    app.personal_switched();
    assert_eq!(listening(&sent(&mut rx)), [None], "off: the net lets go of every room");
    open(&mut app, "another", "5");
    assert!(listening(&sent(&mut rx)).is_empty(), "off: a new room tells it nothing");
  }

  // PRIVACY: switching personal emotes on or off never moves our own twitch id to 7tv: that is the paints
  // switch's alone (the lookup is `SevenTvMe`, asked only with paints on)
  #[test]
  fn personal_emotes_never_widen_the_gate_on_our_own_id() {
    let (mut app, mut rx) = boot();
    app.on_net(NetEvent::Me(Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(42),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    })));
    app.set.paints = false;
    app.set.personal_emotes = false;
    app.applied(crate::model::schema::Fx::Paints);
    sent(&mut rx);
    open(&mut app, "forsen", "22484632");
    for on in [true, false, true] {
      app.set.personal_emotes = on;
      app.applied(crate::model::schema::Fx::Personal);
    }
    let cmds = sent(&mut rx);
    assert!(!cmds.iter().any(|c| matches!(c, Cmd::SevenTvMe(Some(_)))), "no id of ours is asked about");
    // and what the net is told carries rooms, never a user id: 42 is not a room
    assert!(listening(&cmds).iter().flatten().flatten().all(|&r| r == 22484632));
    // the paints switch still decides it alone
    app.set.paints = true;
    app.applied(crate::model::schema::Fx::Paints);
    assert!(sent(&mut rx).iter().any(|c| matches!(c, Cmd::SevenTvMe(Some(42)))));
  }

  // PRIVACY INVARIANT, end to end: the net says user 42 owns a set. their lines draw it; anyone else's do not
  #[test]
  fn a_personal_emote_draws_for_its_owner_and_for_no_one_else() {
    let (mut app, _rx) = boot();
    own(&mut app, &[42], "SET", "https://cdn.7tv.app/emote/DONK/1x.webp");
    assert!(word(&mut app, "42", "DonkMonk"));
    assert!(!word(&mut app, "43", "DonkMonk"), "another sender typing the same word");
    assert!(!word(&mut app, "42", "Other"));
    // the switch off: it is gone at once, and what comes in while off is not kept
    app.set.personal_emotes = false;
    app.applied(crate::model::schema::Fx::Personal);
    assert!(!word(&mut app, "42", "DonkMonk"));
    own(&mut app, &[42], "SET", "https://cdn.7tv.app/emote/DONK/1x.webp");
    app.set.personal_emotes = true;
    app.applied(crate::model::schema::Fx::Personal);
    assert!(!word(&mut app, "42", "DonkMonk"), "off then on starts empty: the net tells us again");
  }

  // a set heard about is read once a line of its owner is drawn, and not asked for again while it is out
  #[test]
  fn a_set_is_read_when_its_owner_speaks() {
    let (mut app, mut rx) = boot();
    app.on_net(NetEvent::Personal(Ev::Grant { users: vec![42], set: "SET".into() }));
    assert!(!word(&mut app, "42", "DonkMonk"), "nothing to draw until it is read");
    assert!(!word(&mut app, "42", "DonkMonk"));
    let asks: Vec<_> = app.res.personal.take_wanted();
    assert_eq!(asks, [Box::<str>::from("SET")]);
    assert!(sent(&mut rx).iter().all(|c| !matches!(c, Cmd::PersonalSets(_))), "asked from the frame, not the event");
  }

  // your own set: in tab-complete only once 7tv has named you its owner, and only yours
  #[test]
  fn your_own_set_is_offered_only_when_7tv_says_it_is_yours() {
    let (mut app, _rx) = boot();
    app.on_net(NetEvent::Me(Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(42),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    })));
    // someone else's set is nowhere in your completion, your box or your picker
    own(&mut app, &[7], "THEIRS", "https://cdn.7tv.app/emote/THEIR/1x.webp");
    assert!(app.candidates("DonkM", false).is_empty());
    assert!(app.box_url("DonkMonk").is_none());
    assert!(!yours(&mut app).contains(&"DonkMonk".to_owned()));
    own(&mut app, &[42], "MINE", "https://cdn.7tv.app/emote/MINE/1x.webp");
    let found = app.candidates("DonkM", false);
    assert_eq!(found.iter().map(|p| &*p.name).collect::<Vec<_>>(), ["DonkMonk"]);
    assert_eq!(app.box_url("DonkMonk").as_deref(), Some("https://cdn.7tv.app/emote/MINE/1x.webp"));
    assert_eq!(yours(&mut app), ["DonkMonk"]);
    // 7tv takes it back
    app.on_net(NetEvent::Personal(Ev::Revoke { users: vec![42], set: "MINE".into() }));
    assert!(app.candidates("DonkM", false).is_empty());
    assert!(yours(&mut app).is_empty());
  }

  // the picker's "mine" section
  fn yours(app: &mut App) -> Vec<String> {
    app.toggle_picker();
    let p = app.picker.as_ref().unwrap();
    let names = (0..p.len()).filter_map(|i| p.item(i)).filter(|it| it.sec == crate::model::picker::Sec::Mine);
    let names = names.map(|it| it.name.to_string()).collect();
    app.close_picker();
    names
  }
}
