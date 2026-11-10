// the built-in tabs (feed, whispers, mentions, mod log): created once at boot in the ext's order, shown or
// hidden with /show and /hide, and the helpers that keep every tab-stepping key off hidden ones
use super::{App, SCROLLBACK, Tab};
use crate::i18n::t;
use crate::model::tabs::{BUILTIN, Kind};
use crate::ui::chat_view::ChatView;

// the ext keeps 500 mentions (MAX_BUFFER) and 150 feed posts
const MENTIONS: usize = 500;
const POSTS: usize = 150;
const WHISPERS: usize = 500;
// the ext keeps 300 mod actions
const MOD_LOG: usize = 300;

pub(super) fn make_all() -> Vec<Tab> {
  BUILTIN.into_iter().map(make).collect()
}

// what a built-in tab's empty box says
fn empty_of(kind: Kind) -> &'static str {
  match kind {
    Kind::Mentions => t("app.builtin.empty_mentions"),
    Kind::ModLog => t("app.builtin.empty_modlog"),
    Kind::Whispers => t("app.builtin.empty_whispers"),
    Kind::Feed => t("app.builtin.empty_feed"),
    Kind::Chat => "",
  }
}

impl App {
  // the language changed: the built-in tabs' empty boxes say it in the new one
  pub(super) fn refresh_empties(&mut self) {
    for tab in self.tabs.iter_mut().filter(|t| t.kind != Kind::Chat) {
      tab.view.set_empty(empty_of(tab.kind));
    }
    if let Some(i) = self.tab_of(Kind::Feed) {
      let text = self.feed.empty_text();
      self.tabs[i].chat().set_empty(text);
    }
    self.whisper_empty();
  }
}

fn make(kind: Kind) -> Tab {
  let name = kind.name();
  let cap = match kind {
    Kind::Feed => POSTS,
    Kind::Whispers => WHISPERS,
    Kind::Mentions => MENTIONS,
    Kind::ModLog => MOD_LOG,
    Kind::Chat => SCROLLBACK,
  };
  let mut view = ChatView::new(cap, name.into());
  // a copied or synthesized line is no chatter: no first-line glow, no relative-time clash
  view.glow = false;
  view.top = kind == Kind::Feed;
  view.timestamps = kind != Kind::Feed;
  view.tags = kind == Kind::Mentions;
  view.set_empty(empty_of(kind));
  Tab {
    name: name.into(),
    kind,
    view,
    unread: false,
    event: false,
    sources: Vec::new(),
    target: 0,
    send: Default::default(),
    under: None,
    count: Default::default(),
    stats: Default::default(),
    temp: false,
  }
}

impl App {
  pub(super) fn visible(&self, i: usize) -> bool {
    self.tabs.get(i).is_some_and(|t| self.set.shown(t.kind))
  }

  pub(super) fn tab_of(&self, k: Kind) -> Option<usize> {
    self.tabs.iter().position(|t| t.kind == k)
  }

  // the next shown tab from `from` (wraps); `from` itself when it is the only one
  pub(super) fn step_tab(&self, from: usize, back: bool) -> usize {
    let n = self.tabs.len();
    (1..=n)
      .map(|d| if back { (from + n - d % n) % n } else { (from + d) % n })
      .find(|&i| self.visible(i))
      .unwrap_or(from)
  }

  // the n-th shown tab (alt-1 .. alt-9)
  pub(super) fn nth_visible(&self, n: usize) -> Option<usize> {
    (0..self.tabs.len()).filter(|&i| self.visible(i)).nth(n)
  }

  // a tab came into view: load what it shows the first time (or when stale)
  pub(super) fn entered(&mut self) {
    match self.kind() {
      Kind::Whispers => self.whispers_load(),
      Kind::Feed => self.feed_entered(),
      _ => {}
    }
  }

  // after tabs come and go: never rest on a hidden one
  pub(super) fn fix_active(&mut self) {
    self.feed_room_sync();
    if !self.visible(self.active) {
      let i = self.step_tab(self.active, false);
      self.active = i;
      if let Some(t) = self.tabs.get_mut(i) {
        t.unread = false;
        t.view.invalidate();
      }
      self.input_dirty = true;
    }
    self.bar_dirty = true;
    self.wake();
  }

  pub(super) fn set_shown(&mut self, k: Kind, on: bool) {
    self.set.show(k, on);
    self.set.save();
    if on && let Some(i) = self.tab_of(k) {
      self.select(i);
    }
    self.fix_active();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_builtin_empty_box_and_permission_name_has_words() {
    for kind in BUILTIN {
      assert!(!empty_of(kind).is_empty(), "{kind:?}");
    }
    for what in ["clipping", "blocked terms", "automod holds", "moderating", "mod tools", "chat modes", "whispers"] {
      let said = super::super::feed::needs_twitch(what);
      assert!(said.starts_with(what) && said.ends_with(super::super::feed::ALLOW_IT), "{said}");
    }
  }

  #[test]
  fn builtins_are_in_the_ext_order_and_sized_like_it() {
    let tabs = make_all();
    let names: Vec<&str> = tabs.iter().map(|t| &*t.name).collect();
    assert_eq!(names, ["feed", "whispers", "mentions", "modlog"]);
    assert!(tabs[0].view.top && !tabs[1].view.top && !tabs[2].view.top);
    assert!(tabs.iter().all(|t| t.sources.is_empty() && !t.view.glow));
  }

  #[test]
  fn bans_timeouts_and_deletes_land_in_the_mod_log() {
    use crate::net::Net;
    use crate::net::irc::Chat;
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    assert!(!app.set.shown(Kind::ModLog), "hidden until shown, like the ext");
    app.on_chat("forsen", Chat::ClearUser { login: "bob".into(), secs: Some(600) });
    app.on_chat("forsen", Chat::ClearMsg("m1".into(), "amy".into()));
    let i = app.tab_of(Kind::ModLog).unwrap();
    let v = &app.tabs[i].view;
    assert_eq!(v.len(), 2);
    assert_eq!(&*v.last_msg().unwrap().body, "forsen · deleted amy's message");
    assert!(app.tabs[i].unread);
  }

  // your own line sent through heatsync.org wears [H], whether its echo lands before or after the site's
  // word; someone else saying the same words does not
  #[test]
  fn a_line_sent_through_heatsync_is_tagged() {
    use crate::model::message::{Kind as MsgKind, Message, Platform};
    use crate::net::irc::Chat;
    use crate::net::{Net, NetEvent};
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.mentions.set(["mellen"]);
    let line = |who: &str, words: &str| {
      let mut m = Message::system(crate::util::time::now_ms(), Platform::Twitch, words);
      (m.kind, m.login, m.nick) = (MsgKind::Chat, who.into(), who.into());
      Chat::Msg(m)
    };
    let i = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    let tagged = |app: &App| app.tabs[i].view.last_msg().unwrap().x.as_ref().is_some_and(|x| x.origin);
    // the echo first, then the site's word
    app.on_chat("forsen", line("mellen", "first"));
    assert!(!tagged(&app));
    app.on_net(NetEvent::Origin("first".into()));
    assert!(tagged(&app));
    // the word first, then the echo
    app.on_net(NetEvent::Origin("gg wp".into()));
    app.on_chat("forsen", line("bob", "gg wp"));
    assert!(!tagged(&app), "not your line");
    app.on_chat("forsen", line("mellen", "gg wp"));
    assert!(tagged(&app));
    app.on_chat("forsen", line("mellen", "something else"));
    assert!(!tagged(&app));
    // the site sent to twitch and kick: the kick copy of the same send folds into the twitch line
    app.tabs[i].sources.push((Platform::Kick, "forsen".into()));
    app.on_chat("forsen", line("mellen", "both"));
    app.on_net(NetEvent::Origin("both".into()));
    let n = app.tabs[i].view.len();
    let mut k = Message::system(crate::util::time::now_ms(), Platform::Kick, "both");
    (k.kind, k.login, k.nick) = (MsgKind::Chat, "mellen".into(), "mellen".into());
    app.on_net(NetEvent::Relay { platform: Platform::Kick, key: "forsen".into(), msgs: vec![k], history: false });
    assert_eq!(app.tabs[i].view.len(), n, "one send, one line");
  }

  // a redeem line lands only in its room's tab, headed by the redeemer, and is nobody's chat line
  #[test]
  fn a_redeem_line_stays_in_its_room() {
    use crate::model::message::{Kind as MsgKind, Platform};
    use crate::model::stream_events::Gate;
    use crate::net::Net;
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into(), "xqc".into()], false);
    let pos = |app: &App, n: &str| app.tabs.iter().position(|t| &*t.name == n).unwrap();
    let before = app.tabs[pos(&app, "xqc")].view.len();
    let v = serde_json::json!({"user": "Bob", "title": "hydrate", "cost": 500});
    let l = Gate::default().redeem(0, Platform::Twitch, "forsen".into(), &v).unwrap();
    app.on_stream(l);
    let m = app.tabs[pos(&app, "forsen")].view.last_msg().unwrap();
    assert_eq!((m.kind, &*m.login, &*m.nick), (MsgKind::Event, "bob", "Bob"));
    assert!(!m.kind.said());
    assert_eq!(app.tabs[pos(&app, "xqc")].view.len(), before, "not in other rooms");
    app.set.ev_redeem = false;
    let v = serde_json::json!({"user": "Amy", "title": "hydrate"});
    app.on_stream(Gate::default().redeem(0, Platform::Twitch, "forsen".into(), &v).unwrap());
    assert_eq!(&*app.tabs[pos(&app, "forsen")].view.last_msg().unwrap().login, "bob", "off: not shown");
  }

  // the room shows what a mod did in the ext's words and colors, and the line taken stays, dimmed
  #[test]
  fn a_deleted_line_is_dimmed_and_told() {
    use crate::model::message::{Message, Platform, flag};
    use crate::net::irc::Chat;
    use crate::net::{Net, NetEvent};
    use crate::ui::theme;
    let (net, _rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    // each removal in its own words: the fold has its own tests (tests_runs)
    app.set.collapse_timeouts = false;
    let mut m = Message::system(1, Platform::Twitch, "bad words");
    (m.kind, m.id, m.login, m.nick) = (crate::model::message::Kind::Chat, "m1".into(), "amy".into(), "amy".into());
    app.on_chat("forsen", Chat::Msg(m));
    app.on_chat("forsen", Chat::ClearMsg("m1".into(), "amy".into()));
    app.on_chat("forsen", Chat::ClearUser { login: "bob".into(), secs: Some(600) });
    app.on_chat("forsen", Chat::ClearUser { login: "cat".into(), secs: None });
    app.on_chat("forsen", Chat::ClearAll);
    let i = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    let rows: Vec<(String, u32)> = app.tabs[i].view.msgs().map(|m| (m.body.to_string(), m.color)).collect();
    assert_eq!(
      rows[1..],
      [
        ("message from amy deleted".into(), theme::RED),
        ("bob timed out for 600s".into(), theme::GREEN),
        ("cat was permanently banned".into(), theme::RED),
        ("chat cleared".into(), theme::RED),
      ]
    );
    assert!(app.tabs[i].view.msgs().next().unwrap().has(flag::CLEARED));
    // kick's removals are told the same way
    app.tabs[i].sources.push((Platform::Kick, "xqc".into()));
    app.on_net(NetEvent::RelayClear {
      platform: Platform::Kick,
      key: "xqc".into(),
      login: "bob".into(),
      secs: Some(60),
    });
    assert_eq!(&*app.tabs[i].view.last_msg().unwrap().body, "bob timed out for 60s");
  }
}
