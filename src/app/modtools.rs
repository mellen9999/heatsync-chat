// the mod tools screen (ext /modtools, mod-suite.js) for a twitch room you moderate: shield mode (enter
// flips it), automod levels (h l, 0-4), pending unban requests (y approve, x deny, then y / n) and who
// is in chat (/ filters). /modtools, /unbanrequests, or the status screen's row. asks for the twitch
// permission when it's missing
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::screen::{Page, Target};
use crate::model::message::Platform;
use crate::model::modtools::{LEVELS, Levels, MAX_LEVEL, Unban};
use crate::model::stats;
use crate::net::Cmd;
use crate::net::blocked::Fail;
use crate::net::modtools::{Got, Op};
use crate::ui::sheet::L;
use crate::util::time::now_ms;

// the chatters listed before a filter is needed
const SHOWN_CHATTERS: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum View {
  Loading,
  Ready,
  Perm,
  NotMod,
  Auth,
  Error,
}

pub(super) struct Mt {
  pub chan: Box<str>,
  pub view: View,
  shield: Option<bool>,
  levels: Option<Levels>,
  unbans: Option<Vec<Unban>>,
  chatters: Option<(u64, Vec<Box<str>>)>,
  // the unban request asked about: (id, approve)
  pub confirm: Option<(Box<str>, bool)>,
  busy: bool,
  pub msg: String,
}

fn row(a: impl Into<String>, b: impl Into<String>) -> L {
  L::Row { a: a.into(), b: b.into(), pill: None }
}

impl Mt {
  fn new(chan: Box<str>) -> Self {
    Mt {
      chan,
      view: View::Loading,
      shield: None,
      levels: None,
      unbans: None,
      chatters: None,
      confirm: None,
      busy: false,
      msg: String::new(),
    }
  }

  pub(super) fn lines(&self, q: &str) -> (Vec<L>, Vec<Target>) {
    let say = |a: &str| (vec![row(a, "")], vec![Target::None]);
    match self.view {
      View::Loading => return say("loading…"),
      View::NotMod => return say("you are not a moderator of this channel"),
      View::Auth => {
        return (
          vec![row("sign in to heatsync again (ctrl-l)", ""), row("retry", "")],
          vec![Target::None, Target::Retry],
        );
      }
      View::Error => {
        return (vec![row("couldn't reach twitch", ""), row("retry", "")], vec![Target::None, Target::Retry]);
      }
      View::Perm => {
        return (
          vec![
            row("heatsync needs one more twitch permission for mod tools", ""),
            row("allow mod tools", ""),
            row("retry", ""),
          ],
          vec![Target::None, Target::Allow, Target::Retry],
        );
      }
      View::Ready => {}
    }
    let q = q.trim().to_ascii_lowercase();
    let (mut lines, mut marks) = (Vec::new(), Vec::new());
    let mut put = |l: L, t: Target| {
      lines.push(l);
      marks.push(t);
    };
    let fail = "couldn't load: r reloads";
    put(L::Head("shield mode".into()), Target::None);
    match self.shield {
      Some(on) => put(L::Row { a: "shield mode".into(), b: String::new(), pill: Some(on) }, Target::Shield),
      None => put(row(fail, ""), Target::None),
    }
    put(L::Head("automod · h l set 0-4".into()), Target::None);
    match &self.levels {
      Some(lv) => {
        for (i, ((_, label), n)) in LEVELS.iter().zip(lv).enumerate() {
          put(row(*label, n.map_or_else(|| "-".to_owned(), |n| n.to_string())), Target::Level(i as u8));
        }
      }
      None => put(row(fail, ""), Target::None),
    }
    match &self.unbans {
      Some(u) => {
        put(L::Head(format!("unban requests · {} pending", u.len())), Target::None);
        let now = now_ms();
        let shown: Vec<(usize, &Unban)> = u
          .iter()
          .enumerate()
          .filter(|(_, r)| q.is_empty() || r.login.contains(&q) || r.text.to_lowercase().contains(&q))
          .collect();
        if shown.is_empty() {
          put(row(if u.is_empty() { "none waiting" } else { "nothing matches" }, ""), Target::None);
        }
        for (i, r) in shown {
          let a = match &self.confirm {
            Some((id, yes)) if *id == r.id => {
              format!("{} {}?  y yes · n no", if *yes { "approve" } else { "deny" }, r.name)
            }
            _ if r.text.is_empty() => r.name.to_string(),
            _ => format!("{}: {}", r.name, r.text),
          };
          let ago = if r.at > 0 { format!("{} ago", stats::span(now - r.at)) } else { String::new() };
          put(row(a, ago), Target::Unban(i));
        }
      }
      None => {
        put(L::Head("unban requests".into()), Target::None);
        put(row(fail, ""), Target::None);
      }
    }
    match &self.chatters {
      Some((total, c)) => {
        put(L::Head(format!("chatters · {}", stats::group(*total))), Target::None);
        let hits: Vec<&Box<str>> = c.iter().filter(|l| q.is_empty() || l.contains(&q)).collect();
        for l in hits.iter().take(SHOWN_CHATTERS) {
          put(row(&***l, ""), Target::None);
        }
        if hits.len() > SHOWN_CHATTERS {
          put(row(format!("{} more: / filters", stats::group((hits.len() - SHOWN_CHATTERS) as u64)), ""), Target::None);
        }
      }
      None => {
        put(L::Head("chatters".into()), Target::None);
        put(row(fail, ""), Target::None);
      }
    }
    (lines, marks)
  }
}

impl App {
  // /modtools, /unbanrequests
  pub(super) fn tools_here(&mut self) {
    match self.twitch_here() {
      Some(chan) => self.tools_open(chan),
      None => self.say("/modtools needs a twitch channel tab"),
    }
  }

  pub(super) fn tools_open(&mut self, chan: Box<str>) {
    if self.me.is_none() {
      return self.say("log in first (ctrl-l)");
    }
    if !self.can_mod(Platform::Twitch, &chan) {
      return self.say(format!("/modtools needs mod on {chan}"));
    }
    self.open_screen(Page::ModTools);
    if let Some(s) = &mut self.screen {
      s.mt = Some(Box::new(Mt::new(chan)));
    }
    self.tools_load();
  }

  fn mt(&mut self) -> Option<&mut Mt> {
    self.screen.as_mut()?.mt.as_deref_mut()
  }

  fn tools_ask(&mut self, op: Op) {
    let Some(mt) = self.mt() else { return };
    if mt.busy {
      return;
    }
    mt.busy = true;
    let chan = mt.chan.clone();
    self.net.send(Cmd::Tools(chan, op));
  }

  fn tools_load(&mut self) {
    if let Some(mt) = self.mt() {
      (mt.view, mt.confirm, mt.busy) = (View::Loading, None, false);
    }
    self.tools_ask(Op::Load);
  }

  // enter / click, and h / l (Some(up))
  pub(super) fn tools_pick(&mut self, t: Target, adjust: Option<bool>) {
    let Some(mt) = self.mt() else { return };
    let op = match (t, adjust) {
      (Target::Allow, None) => {
        self.say("finish allowing mod tools in your browser, then retry");
        return self.net.send(Cmd::Relink("modsuite"));
      }
      (Target::Retry, None) => return self.tools_load(),
      (Target::Shield, _) => {
        let Some(on) = mt.shield else { return };
        let want = adjust.unwrap_or(!on);
        (want != on).then_some(Op::Shield(want))
      }
      (Target::Level(i), _) => {
        let Some(cur) = mt.levels.as_ref().map(|l| l[i as usize].unwrap_or(0)) else { return };
        let next = match adjust {
          Some(true) => (cur + 1).min(MAX_LEVEL),
          Some(false) => cur.saturating_sub(1),
          None => (cur + 1) % (MAX_LEVEL + 1),
        };
        (next != cur).then_some(Op::Level(i as usize, next))
      }
      (Target::Unban(i), None) => {
        mt.confirm = mt.unbans.as_ref().and_then(|u| u.get(i)).map(|r| (r.id.clone(), true));
        None
      }
      _ => None,
    };
    if let Some(op) = op {
      self.tools_ask(op);
    }
  }

  pub(super) fn on_tools(&mut self, chan: Box<str>, res: Result<Got, Fail>) {
    let Some(mt) = self.mt().filter(|m| m.chan == chan) else { return };
    mt.busy = false;
    match res {
      Ok(Got::Loaded(l)) => {
        (mt.shield, mt.levels, mt.unbans, mt.chatters) = (l.shield, l.levels, l.unbans, l.chatters);
        mt.view = View::Ready;
      }
      Ok(Got::Shield(on)) => {
        mt.shield = Some(on);
        mt.msg = format!("shield mode {}", if on { "on" } else { "off" });
      }
      Ok(Got::Levels(l)) => {
        mt.levels = Some(l);
        mt.msg = "automod updated".into();
      }
      Ok(Got::Resolved(id, approve)) => {
        let who = mt.unbans.as_ref().and_then(|u| u.iter().find(|r| r.id == id)).map(|r| r.name.to_string());
        if let Some(u) = &mut mt.unbans {
          u.retain(|r| r.id != id);
        }
        mt.confirm = None;
        mt.msg = format!("{} {}", if approve { "unbanned" } else { "denied" }, who.unwrap_or_default());
      }
      Err(Fail::Relink) => mt.view = View::Perm,
      Err(Fail::NotMod) => mt.view = View::NotMod,
      Err(Fail::Auth) => mt.view = View::Auth,
      Err(Fail::Limited) => mt.msg = "slow down: try again in a moment".into(),
      Err(_) if mt.view == View::Loading => mt.view = View::Error,
      // a request someone else settled already
      Err(Fail::Gone) => {
        if let Some((id, _)) = mt.confirm.take()
          && let Some(u) = &mut mt.unbans
        {
          u.retain(|r| r.id != id);
        }
        mt.msg = "already settled".into();
      }
      Err(Fail::Other) => mt.msg = "couldn't reach twitch: try again".into(),
    }
    self.wake();
  }

  // the screen's own keys: r reloads; on a request y asks to approve, x to deny, then y / n. true = spent
  pub(super) fn tools_key(&mut self, key: &Key<&str>) -> bool {
    let ctrl = self.mods.control_key();
    let Some(s) = self.screen.as_mut() else { return false };
    if s.typing || ctrl {
      return false;
    }
    let at = s.target();
    let Some(mt) = s.mt.as_deref_mut() else { return false };
    let op = if let Some((id, yes)) = mt.confirm.clone() {
      match key {
        Key::Character("y") | Key::Named(NamedKey::Enter) => Some(Op::Resolve(id, yes)),
        Key::Character("n") | Key::Named(NamedKey::Escape) => {
          mt.confirm = None;
          None
        }
        _ => None,
      }
    } else {
      match (key, at) {
        (Key::Character("r"), _) => {
          self.tools_load();
          return true;
        }
        (Key::Character(k @ ("y" | "x")), Some(Target::Unban(i))) => {
          mt.confirm = mt.unbans.as_ref().and_then(|u| u.get(i)).map(|r| (r.id.clone(), *k == "y"));
          None
        }
        _ => return false,
      }
    };
    if let Some(op) = op {
      self.tools_ask(op);
    }
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::irc::Chat;
  use crate::net::modtools::Loaded;
  use crate::net::{Net, NetEvent};

  fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<String> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| match c {
        Cmd::Tools(c, Op::Load) => Some(format!("{c} load")),
        Cmd::Tools(c, Op::Shield(on)) => Some(format!("{c} shield {on}")),
        Cmd::Tools(c, Op::Resolve(id, yes)) => Some(format!("{c} resolve {id} {yes}")),
        Cmd::Tools(c, Op::Level(i, n)) => Some(format!("{c} level {i} {n}")),
        Cmd::Relink(p) => Some(format!("relink {p}")),
        _ => None,
      })
      .collect()
  }

  fn rows(app: &App) -> Vec<String> {
    let s = app.screen.as_ref().unwrap();
    let (lines, _) = s.mt.as_ref().unwrap().lines(&s.q.text);
    lines
      .into_iter()
      .map(|l| match l {
        L::Head(h) => format!("# {h}"),
        L::Tint { a, .. } => a,
        L::Row { a, pill: Some(on), .. } => format!("{a} [{on}]"),
        L::Row { a, b, .. } if b.is_empty() => a,
        L::Row { a, b, .. } => format!("{a} | {b}"),
      })
      .collect()
  }

  #[test]
  fn a_mod_flips_shield_sets_automod_and_settles_unban_requests() {
    let (net, mut rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    let tab = app.tabs.iter().position(|t| &*t.name == "forsen").unwrap();
    app.select(tab);
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
    app.tools_here();
    assert!(app.screen.is_none(), "not a mod");
    app.on_chat("forsen", Chat::Mod(true));
    app.tools_here();
    assert_eq!(asked(&mut rx), ["forsen load"]);
    // the permission is missing: offer it
    app.on_net(NetEvent::Tools { chan: "forsen".into(), res: Err(Fail::Relink) });
    app.tools_pick(Target::Allow, None);
    assert_eq!(asked(&mut rx), ["relink modsuite"]);
    app.tools_pick(Target::Retry, None);
    let mut levels = [Some(1); 9];
    levels[0] = Some(2);
    let unban =
      |id: &str, who: &str| Unban { id: id.into(), login: who.into(), name: who.into(), text: "sorry".into(), at: 0 };
    let loaded = Loaded {
      shield: Some(false),
      levels: Some(levels),
      unbans: Some(vec![unban("u1", "bob"), unban("u2", "amy")]),
      chatters: Some((3, vec!["amy".into(), "bob".into(), "zed".into()])),
    };
    app.on_net(NetEvent::Tools { chan: "forsen".into(), res: Ok(Got::Loaded(Box::new(loaded))) });
    let r = rows(&app);
    assert!(r.contains(&"shield mode [false]".to_owned()));
    assert!(r.contains(&"overall | 2".to_owned()));
    assert!(r.contains(&"# unban requests · 2 pending".to_owned()));
    assert!(r.contains(&"bob: sorry".to_owned()));
    assert!(r.contains(&"# chatters · 3".to_owned()));
    app.redraw();
    assert_eq!(asked(&mut rx), ["forsen load"]);
    app.tools_pick(Target::Shield, None);
    app.tools_pick(Target::Level(0), Some(true));
    assert_eq!(asked(&mut rx), ["forsen shield true"], "one ask at a time");
    app.on_net(NetEvent::Tools { chan: "forsen".into(), res: Ok(Got::Shield(true)) });
    app.tools_pick(Target::Level(0), Some(true));
    assert_eq!(asked(&mut rx), ["forsen level 0 3"]);
    app.on_net(NetEvent::Tools { chan: "forsen".into(), res: Ok(Got::Levels([Some(3); 9])) });
    // deny amy: x asks, y confirms
    let at =
      app.screen.as_ref().unwrap().mt.as_ref().unwrap().lines("").1.iter().position(|t| matches!(t, Target::Unban(1)));
    app.screen.as_mut().unwrap().st.at = at.unwrap();
    assert!(app.tools_key(&Key::Character("x")));
    assert!(rows(&app).contains(&"deny amy?  y yes · n no".to_owned()));
    assert!(app.tools_key(&Key::Character("y")));
    assert_eq!(asked(&mut rx), ["forsen resolve u2 false"]);
    app.on_net(NetEvent::Tools { chan: "forsen".into(), res: Ok(Got::Resolved("u2".into(), false)) });
    assert!(rows(&app).contains(&"# unban requests · 1 pending".to_owned()));
    assert_eq!(app.screen.as_ref().unwrap().mt.as_ref().unwrap().msg, "denied amy");
    // a filter narrows requests and chatters
    app.screen.as_mut().unwrap().q.set_text("zed".into());
    let r = rows(&app);
    assert!(r.contains(&"zed".to_owned()) && !r.contains(&"bob: sorry".to_owned()));
  }
}
