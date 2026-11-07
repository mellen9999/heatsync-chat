// a twitch room's blocked terms, for its moderators (ext blocked-terms.js): a list screen over the chat
// from /blocked or a line's menu. j/k move, x deletes the picked term (y yes, n no), a adds, / filters,
// esc closes. without the twitch permission it offers to ask for it
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::screen::{Page, Target};
use crate::model::blocked::{self, Term};
use crate::model::line::Line;
use crate::model::message::Platform;
use crate::net::Cmd;
use crate::net::blocked::{Fail, Got, Op};
use crate::ui::sheet::L;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum View {
  Loading,
  List,
  // the twitch login lacks the blocked terms permission
  Perm,
  NotMod,
  Auth,
  Error,
}

// what is being asked, so a failure lands on the right thing
enum Asking {
  List,
  Add,
  // (id, text)
  Remove(Box<str>, Box<str>),
}

pub(super) struct Bt {
  pub chan: Box<str>,
  room: Box<str>,
  pub view: View,
  pub terms: Vec<Term>,
  // twitch had more than the server pages through
  more: bool,
  // the term asked about deleting (y / n)
  pub confirm: Option<Box<str>>,
  pub add: Line,
  pub adding: bool,
  asking: Option<Asking>,
  // the last thing done or gone wrong, under the list
  pub msg: String,
}

impl Bt {
  pub(super) fn new(chan: Box<str>, room: Box<str>) -> Self {
    Bt {
      chan,
      room,
      view: View::Loading,
      terms: Vec::new(),
      more: false,
      confirm: None,
      add: Line::default(),
      adding: false,
      asking: None,
      msg: String::new(),
    }
  }

  // the screen's rows: the terms (a filter typed), or what stands in their way
  pub(super) fn lines(&self, q: &str) -> (Vec<L>, Vec<Target>) {
    let row = |a: &str| L::Row { a: a.into(), b: String::new(), pill: None };
    let say = |a: &str| (vec![row(a)], vec![Target::None]);
    match self.view {
      View::Loading => say("loading…"),
      View::NotMod => say("you are not a moderator of this channel"),
      View::Auth => (vec![row("sign in to heatsync again (ctrl-l)"), row("retry")], vec![Target::None, Target::Retry]),
      View::Error => (vec![row("couldn't reach twitch"), row("retry")], vec![Target::None, Target::Retry]),
      View::Perm => (
        vec![
          row("heatsync needs one more twitch permission to manage blocked terms"),
          row("allow blocked terms"),
          row("retry"),
        ],
        vec![Target::None, Target::Allow, Target::Retry],
      ),
      View::List => {
        let shown = blocked::visible(&self.terms, q);
        if shown.is_empty() {
          return say(if self.terms.is_empty() { "no blocked terms yet: a adds one" } else { "nothing matches" });
        }
        let lines = shown
          .iter()
          .map(|t| match &self.confirm {
            Some(id) if *id == t.id => row(&format!("delete \"{}\"?  y yes · n no", t.text)),
            _ => row(&t.text),
          })
          .collect();
        let at = |t: &Term| self.terms.iter().position(|x| x.id == t.id).unwrap_or(0);
        (lines, shown.iter().map(|t| Target::Term(at(t))).collect())
      }
    }
  }

  pub(super) fn title(&self, q: &str) -> String {
    let n = self.terms.len();
    match self.view {
      View::List if !q.trim().is_empty() => {
        format!("blocked terms · {} · {} of {n}", self.chan, blocked::visible(&self.terms, q).len())
      }
      View::List => format!("blocked terms · {} · {n}{}", self.chan, if self.more { "+" } else { "" }),
      _ => format!("blocked terms · {}", self.chan),
    }
  }
}

impl App {
  // /blocked: the active tab's twitch room
  pub(super) fn blocked_here(&mut self) {
    let chan =
      self.tabs.get(self.active).and_then(|t| t.sources.iter().find(|s| s.0 == Platform::Twitch)).map(|s| s.1.clone());
    match chan {
      None => self.say("open a twitch channel tab first"),
      Some(c) => self.blocked_open(c),
    }
  }

  pub(super) fn blocked_open(&mut self, chan: Box<str>) {
    if self.me.is_none() {
      return self.say("log in first (ctrl-l)");
    }
    if !self.can_mod(Platform::Twitch, &chan) {
      return self.say("/blocked needs mod on this twitch channel");
    }
    let Some(room) = self.rooms.iter().find(|r| r.0 == chan).map(|r| r.1.clone()) else {
      return self.say("still joining that channel: try again in a moment");
    };
    self.open_screen(Page::Blocked);
    if let Some(s) = &mut self.screen {
      s.bt = Some(Box::new(Bt::new(chan, room)));
    }
    self.blocked_load();
  }

  fn bt(&mut self) -> Option<&mut Bt> {
    self.screen.as_mut()?.bt.as_deref_mut()
  }

  fn blocked_ask(&mut self, asking: Asking, op: Op) {
    let Some(bt) = self.bt() else { return };
    if bt.asking.is_some() {
      return;
    }
    bt.asking = Some(asking);
    let room = bt.room.clone();
    self.net.send(Cmd::Blocked(room, op));
  }

  pub(super) fn blocked_load(&mut self) {
    if let Some(bt) = self.bt() {
      bt.view = View::Loading;
      bt.asking = None;
    }
    self.blocked_ask(Asking::List, Op::List);
  }

  fn blocked_add(&mut self) {
    let Some(bt) = self.bt() else { return };
    let Some(text) = blocked::addable(&bt.add.text).map(str::to_owned) else {
      bt.msg = format!("a term is {} to {} characters", blocked::MIN, blocked::MAX);
      return;
    };
    self.blocked_ask(Asking::Add, Op::Add(text));
  }

  fn blocked_remove(&mut self, i: usize) {
    let Some(bt) = self.bt() else { return };
    bt.confirm = None;
    let Some(t) = bt.terms.get(i).cloned() else { return };
    self.blocked_ask(Asking::Remove(t.id.clone(), t.text), Op::Remove(t.id));
  }

  // allow (the permission) and retry rows, and enter / click on a term (asks to delete it)
  pub(super) fn blocked_pick(&mut self, t: Target) {
    match t {
      Target::Allow => {
        self.say(super::feed::needs_twitch("blocked terms"));
        self.net.send(Cmd::Relink("blockedterms"));
      }
      Target::Retry => self.blocked_load(),
      Target::Term(i) => {
        if let Some(bt) = self.bt()
          && let Some(id) = bt.terms.get(i).map(|t| t.id.clone())
        {
          bt.confirm = Some(id);
        }
      }
      _ => {}
    }
  }

  pub(super) fn on_blocked(&mut self, room: Box<str>, res: Result<Got, Fail>) {
    let Some(bt) = self.bt().filter(|b| b.room == room) else { return };
    let asking = bt.asking.take();
    match (res, asking) {
      (Ok(Got::List(terms, more)), _) => {
        (bt.terms, bt.more, bt.view) = (terms, more, View::List);
      }
      (Ok(Got::Added(t)), _) => {
        bt.msg = format!("blocked \"{}\"", t.text);
        bt.terms.retain(|x| x.id != t.id);
        bt.terms.insert(0, t);
        bt.add.set_text(String::new());
        bt.add.home();
      }
      // already gone is what the mod wanted
      (Ok(Got::Removed(id)), Some(Asking::Remove(_, text))) | (Err(Fail::Gone), Some(Asking::Remove(id, text))) => {
        bt.terms.retain(|x| x.id != id);
        bt.msg = format!("unblocked \"{text}\"");
      }
      (Ok(Got::Removed(id)), _) => bt.terms.retain(|x| x.id != id),
      (Err(Fail::Relink), _) => bt.view = View::Perm,
      (Err(Fail::NotMod), _) => bt.view = View::NotMod,
      (Err(Fail::Auth), _) => bt.view = View::Auth,
      (Err(Fail::Limited), _) => bt.msg = "slow down: try again in a moment".into(),
      (Err(_), Some(Asking::List)) => bt.view = View::Error,
      (Err(_), _) => bt.msg = "couldn't reach twitch: try again".into(),
    }
    self.wake();
  }

  // the screen's keys while it shows blocked terms. true = spent
  pub(super) fn blocked_key(&mut self, key: &Key<&str>, text: Option<&str>) -> bool {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    let accepts = self.ime.accepts_key_text();
    let Some(s) = self.screen.as_mut() else { return false };
    let Some(bt) = s.bt.as_deref_mut() else { return false };
    // the add field has the keys
    if bt.adding {
      match key {
        Key::Named(NamedKey::Escape) if !bt.add.text.is_empty() => {
          bt.add.set_text(String::new());
          bt.add.home();
        }
        Key::Named(NamedKey::Escape) => (bt.adding, s.typing) = (false, false),
        Key::Named(NamedKey::Enter) => self.blocked_add(),
        _ if super::tabs::edit_line(&mut bt.add, key, ctrl) => {}
        Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
          if let Some(t) = text.filter(|_| accepts) {
            bt.add.insert(t);
          }
        }
        _ => {}
      }
      return true;
    }
    if s.typing || bt.view != View::List {
      return false;
    }
    if bt.confirm.is_some() {
      match key {
        Key::Character("y") | Key::Named(NamedKey::Enter) => {
          let i = s.target();
          if let Some(Target::Term(i)) = i {
            self.blocked_remove(i);
          }
        }
        Key::Character("n") | Key::Named(NamedKey::Escape) => bt.confirm = None,
        _ => {}
      }
      return true;
    }
    match key {
      Key::Character("x") if !ctrl => {}
      Key::Named(NamedKey::Delete) => {}
      Key::Character("a") if !ctrl => {
        (bt.adding, s.typing) = (true, true);
        return true;
      }
      _ => return false,
    }
    if let Some(t @ Target::Term(_)) = s.target() {
      self.blocked_pick(t);
    }
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::irc::Chat;
  use crate::net::{Net, NetEvent};

  fn t(id: &str, s: &str) -> Term {
    Term { id: id.into(), text: s.into() }
  }

  fn rows(app: &App) -> Vec<String> {
    let s = app.screen.as_ref().unwrap();
    let (lines, _) = s.bt.as_ref().unwrap().lines(&s.q.text);
    lines.into_iter().filter_map(|l| if let L::Row { a, .. } = l { Some(a) } else { None }).collect()
  }

  fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<String> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| match c {
        Cmd::Blocked(room, Op::List) => Some(format!("{room} list")),
        Cmd::Blocked(room, Op::Add(t)) => Some(format!("{room} add {t}")),
        Cmd::Blocked(room, Op::Remove(id)) => Some(format!("{room} remove {id}")),
        Cmd::Relink(p) => Some(format!("relink {p}")),
        _ => None,
      })
      .collect()
  }

  #[test]
  fn a_mod_lists_adds_and_deletes_terms_from_the_keyboard() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
    app.blocked_here();
    assert!(app.screen.is_none(), "logged out: nothing opens");
    app.me = Some(crate::net::auth::Me {
      username: "mellen".into(),
      platform: "twitch".into(),
      twitch: Some("mellen".into()),
      twitch_id: Some(1),
      names: vec!["mellen".into()],
      kick: None,
      youtube: None,
      packs: vec![],
    });
    app.on_chat("forsen", Chat::Room { id: "22484632".into(), modes: Vec::new() });
    app.blocked_here();
    assert!(app.screen.is_none(), "not a mod: nothing opens");
    app.on_chat("forsen", Chat::Mod(true));
    app.blocked_here();
    assert_eq!(asked(&mut rx), ["22484632 list"]);
    assert_eq!(rows(&app), ["loading…"]);
    let list = Got::List(vec![t("1", "spam link"), t("2", "other")], false);
    app.on_net(NetEvent::Blocked { room: "22484632".into(), res: Ok(list) });
    assert_eq!(rows(&app), ["spam link", "other"]);
    app.redraw();
    // a: type a term, enter adds it on top
    app.on_key(Key::Character("a"), Some("a"));
    for c in ["b", "a", "d"] {
      app.on_key(Key::Character(c), Some(c));
    }
    app.on_key(Key::Named(NamedKey::Enter), None);
    assert_eq!(asked(&mut rx), ["22484632 add bad"]);
    app.on_net(NetEvent::Blocked { room: "22484632".into(), res: Ok(Got::Added(t("3", "bad"))) });
    assert_eq!(rows(&app), ["bad", "spam link", "other"]);
    assert_eq!(app.screen.as_ref().unwrap().bt.as_ref().unwrap().msg, "blocked \"bad\"");
    app.on_key(Key::Named(NamedKey::Escape), None);
    // j, x asks first, n keeps it, x y deletes; d does nothing
    app.on_key(Key::Character("j"), Some("j"));
    app.on_key(Key::Character("d"), Some("d"));
    assert!(rows(&app).iter().all(|r| !r.starts_with("delete")), "d is not delete");
    app.on_key(Key::Character("x"), Some("x"));
    assert_eq!(rows(&app)[1], "delete \"spam link\"?  y yes · n no");
    app.on_key(Key::Character("n"), Some("n"));
    assert_eq!(rows(&app)[1], "spam link");
    app.on_key(Key::Character("x"), Some("x"));
    app.on_key(Key::Character("y"), Some("y"));
    assert_eq!(asked(&mut rx), ["22484632 remove 1"]);
    // twitch says it was already gone: that is the state wanted
    app.on_net(NetEvent::Blocked { room: "22484632".into(), res: Err(Fail::Gone) });
    assert_eq!(rows(&app), ["bad", "other"]);
    // / filters
    app.on_key(Key::Character("/"), Some("/"));
    for c in ["o", "t"] {
      app.on_key(Key::Character(c), Some(c));
    }
    assert_eq!(rows(&app), ["other"]);
    app.redraw();
    app.on_key(Key::Named(NamedKey::Escape), None);
    app.on_key(Key::Named(NamedKey::Escape), None);
    app.on_key(Key::Named(NamedKey::Escape), None);
    assert!(app.screen.is_none(), "esc closes");
  }

  #[test]
  fn without_the_permission_it_offers_to_ask_for_it() {
    let mut bt = Bt::new("forsen".into(), "1".into());
    bt.view = View::Perm;
    let (lines, targets) = bt.lines("");
    assert!(matches!(&lines[1], L::Row { a, .. } if a == "allow blocked terms"));
    assert!(matches!(targets[1], Target::Allow) && matches!(targets[2], Target::Retry));
    bt.view = View::List;
    assert!(matches!(&bt.lines("").0[0], L::Row { a, .. } if a == "no blocked terms yet: a adds one"));
  }
}
