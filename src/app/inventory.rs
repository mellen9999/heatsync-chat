// your heatsync emote inventory, managed (the ext's picker "mine" menu, its removed and sets tabs, undo/redo):
// a list screen over the chat from ctrl-p or /emotes. three tabs ([ ] switch): mine (your slots), removed
// (what you took out, restorable), sets (saved sets). on mine: n renames, m moves to a slot, J K carry a
// slot down / up, x removes (y yes, n no), u undoes, ctrl-r redoes, s shares the set, S stops sharing, i
// imports a channel's emotes, / filters. every change goes to heatsync first and shows when it says yes; your
// own chat lines then read the new set through the usual sender refetch (net::inventory sends it)
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::screen::{Page, Target};
use crate::i18n::{f, t};
use crate::model::inventory::{self as inv, Gone, Saved, Slot};
use crate::model::line::Line;
use crate::net::Cmd;
use crate::net::inventory::{Got, Op};
use crate::ui::sheet::L;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Tab {
  Mine,
  Removed,
  Sets,
}

const TABS: [Tab; 3] = [Tab::Mine, Tab::Removed, Tab::Sets];

impl Tab {
  fn name(self) -> &'static str {
    match self {
      Tab::Mine => t("settings.inventory.tab.mine"),
      Tab::Removed => t("settings.inventory.tab.removed"),
      Tab::Sets => t("settings.inventory.tab.sets"),
    }
  }

  fn at(self) -> usize {
    TABS.iter().position(|t| *t == self).unwrap_or(0)
  }

  fn step(self, up: bool) -> Tab {
    TABS[(self.at() + if up { 1 } else { TABS.len() - 1 }) % TABS.len()]
  }
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum Load {
  Not,
  Busy,
  Ready,
  Failed(String),
}

// a typed answer being asked for: the slot's new name, the slot to go to, the channel to import
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Ask {
  Rename(u32),
  Move(u32),
  Import,
}

impl Ask {
  pub(super) fn mark(self) -> &'static str {
    match self {
      Ask::Rename(_) => t("settings.inventory.ask.rename"),
      Ask::Move(_) => t("settings.inventory.ask.move"),
      Ask::Import => t("settings.inventory.ask.import"),
    }
  }
}

// a change waiting on y / n
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sure {
  Remove(u32),
  Apply(u64),
  Unshare,
}

pub(super) struct Inv {
  tab: Tab,
  load: [Load; 3],
  slots: Vec<Slot>,
  gone: Vec<Gone>,
  sets: Vec<Saved>,
  pub ask: Option<Ask>,
  pub input: Line,
  sure: Option<Sure>,
  // a change is on its way: the next waits for its answer
  acting: bool,
  // the last thing done or gone wrong, under the list
  pub msg: String,
}

fn row(a: impl Into<String>, b: impl Into<String>) -> L {
  L::Row { a: a.into(), b: b.into(), pill: None }
}

fn detail(s: &Slot) -> String {
  let mut p = vec![format!("#{}", s.slot)];
  if !s.source.is_empty() {
    p.push(s.source.to_string());
  }
  if s.upload {
    p.push(t("settings.inventory.upload").into());
  }
  if s.zero_width {
    p.push(t("settings.inventory.zero_width").into());
  }
  if s.uses > 0 {
    p.push(f("settings.inventory.uses", &[("n", &s.uses.to_string())]));
  }
  p.join(" · ")
}

impl Inv {
  fn new() -> Self {
    Inv {
      tab: Tab::Mine,
      load: [Load::Not, Load::Not, Load::Not],
      slots: Vec::new(),
      gone: Vec::new(),
      sets: Vec::new(),
      ask: None,
      input: Line::default(),
      sure: None,
      acting: false,
      msg: String::new(),
    }
  }

  fn state(&self) -> &Load {
    &self.load[self.tab.at()]
  }

  fn count(&self) -> usize {
    match self.tab {
      Tab::Mine => self.slots.len(),
      Tab::Removed => self.gone.len(),
      Tab::Sets => self.sets.len(),
    }
  }

  // the names a filter keeps, by their place in the full list
  fn shown(&self, q: &str) -> Vec<usize> {
    let q = q.trim().to_lowercase();
    let names: Vec<&str> = match self.tab {
      Tab::Mine => self.slots.iter().map(|s| &*s.name).collect(),
      Tab::Removed => self.gone.iter().map(|g| &*g.name).collect(),
      Tab::Sets => self.sets.iter().map(|s| &*s.name).collect(),
    };
    names.iter().enumerate().filter(|(_, n)| q.is_empty() || n.to_lowercase().contains(&q)).map(|(i, _)| i).collect()
  }

  pub(super) fn lines(&self, q: &str) -> (Vec<L>, Vec<Target>) {
    let say = |a: &str| (vec![row(a, "")], vec![Target::None]);
    match self.state() {
      Load::Not | Load::Busy => return say(t("settings.inventory.loading")),
      Load::Failed(e) => {
        return (vec![row(e.as_str(), ""), row(t("settings.inventory.retry"), "")], vec![Target::None, Target::Retry]);
      }
      Load::Ready => {}
    }
    let shown = self.shown(q);
    if shown.is_empty() {
      return say(match (self.count(), self.tab) {
        (n, _) if n > 0 => t("settings.inventory.empty.filter"),
        (_, Tab::Mine) => t("settings.inventory.empty.mine"),
        (_, Tab::Removed) => t("settings.inventory.empty.removed"),
        (_, Tab::Sets) => t("settings.inventory.empty.sets"),
      });
    }
    let lines = shown
      .iter()
      .map(|&i| match self.tab {
        Tab::Mine => {
          let s = &self.slots[i];
          match self.sure {
            Some(Sure::Remove(slot)) if slot == s.slot => {
              row(f("settings.inventory.sure_remove", &[("name", &s.name)]), "")
            }
            _ => row(&*s.name, detail(s)),
          }
        }
        Tab::Removed => {
          row(&*self.gone[i].name, f("settings.inventory.was", &[("slot", &self.gone[i].slot.to_string())]))
        }
        Tab::Sets => {
          let s = &self.sets[i];
          match self.sure {
            Some(Sure::Apply(id)) if id == s.id => row(f("settings.inventory.sure_apply", &[("name", &s.name)]), ""),
            _ => row(
              format!("{}{}", if s.starred { "★ " } else { "" }, s.name),
              f("settings.inventory.emotes", &[("n", &s.count.to_string())]),
            ),
          }
        }
      })
      .collect();
    (lines, shown.into_iter().map(Target::Inv).collect())
  }

  pub(super) fn title(&self) -> String {
    let tabs: Vec<String> =
      TABS.iter().map(|k| if *k == self.tab { format!("[{}]", k.name()) } else { k.name().to_owned() }).collect();
    let tabs = tabs.join(" ");
    match self.state() {
      Load::Ready => f("settings.inventory.title_n", &[("tabs", &tabs), ("n", &self.count().to_string())]),
      _ => f("settings.inventory.title", &[("tabs", &tabs)]),
    }
  }

  // the dim lines under the list: what happened last, else what the screen is for
  pub(super) fn tip(&self) -> &str {
    if !self.msg.is_empty() {
      return &self.msg;
    }
    match (self.ask, self.sure, self.tab) {
      (Some(Ask::Rename(_)), ..) => t("settings.inventory.tip.rename"),
      (Some(Ask::Move(_)), ..) => t("settings.inventory.tip.move"),
      (Some(Ask::Import), ..) => t("settings.inventory.tip.import"),
      (_, Some(Sure::Unshare), _) => t("settings.inventory.tip.unshare"),
      (.., Tab::Mine) => t("settings.inventory.tip.mine"),
      (.., Tab::Removed) => t("settings.inventory.tip.removed"),
      (.., Tab::Sets) => t("settings.inventory.tip.sets"),
    }
  }

  pub(super) fn keys(&self, filtering: bool) -> &'static str {
    if self.ask.is_some() {
      return t("settings.inventory.keys.ask");
    }
    if filtering {
      return t("settings.inventory.keys.filter");
    }
    if self.sure.is_some() {
      return t("settings.inventory.keys.sure");
    }
    match self.tab {
      Tab::Mine => t("settings.inventory.keys.mine"),
      Tab::Removed => t("settings.inventory.keys.removed"),
      Tab::Sets => t("settings.inventory.keys.sets"),
    }
  }
}

impl App {
  // ctrl-p, /emotes
  pub(super) fn inv_open(&mut self) {
    if self.screen_is(Page::Inventory) {
      return self.close_screen();
    }
    if self.me.is_none() {
      return self.say(t("settings.inventory.login_first"));
    }
    self.open_screen(Page::Inventory);
    if let Some(s) = &mut self.screen {
      s.inv = Some(Box::new(Inv::new()));
    }
    self.inv_load(Tab::Mine);
  }

  fn inv(&mut self) -> Option<&mut Inv> {
    self.screen.as_mut()?.inv.as_deref_mut()
  }

  fn inv_load(&mut self, tab: Tab) {
    let Some(iv) = self.inv() else { return };
    if iv.load[tab.at()] == Load::Busy {
      return;
    }
    iv.load[tab.at()] = Load::Busy;
    let op = match tab {
      Tab::Mine => Op::List,
      Tab::Removed => Op::Removed,
      Tab::Sets => Op::Sets,
    };
    self.net.send(Cmd::Inventory(op));
  }

  // a change: one at a time, so every answer lands on the list it was asked of
  fn inv_act(&mut self, op: Op) {
    let Some(iv) = self.inv() else { return };
    if iv.acting {
      iv.msg = t("settings.inventory.one_moment").into();
      return;
    }
    (iv.acting, iv.sure) = (true, None);
    iv.msg.clear();
    self.net.send(Cmd::Inventory(op));
  }

  fn inv_tab(&mut self, tab: Tab) {
    let Some(s) = self.screen.as_mut() else { return };
    let Some(iv) = s.inv.as_deref_mut() else { return };
    (iv.tab, iv.sure, iv.msg) = (tab, None, String::new());
    s.st.at = 0;
    s.st.scroll = 0;
    if iv.state() == &Load::Not {
      self.inv_load(tab);
    }
  }

  fn inv_end_ask(&mut self) {
    if let Some(s) = self.screen.as_mut() {
      s.typing = false;
      if let Some(iv) = s.inv.as_deref_mut() {
        iv.ask = None;
        iv.input.set_text(String::new());
        iv.input.home();
      }
    }
  }

  fn inv_start_ask(&mut self, ask: Ask, text: &str) {
    if let Some(s) = self.screen.as_mut() {
      if let Some(iv) = s.inv.as_deref_mut() {
        iv.ask = Some(ask);
        iv.sure = None;
        iv.msg.clear();
        iv.input.set_text(text.to_owned());
        iv.input.end();
      }
      s.typing = true;
    }
  }

  // enter enough: the typed answer, checked here with the server's own rules before anything is sent
  fn inv_submit(&mut self) {
    let Some(iv) = self.inv() else { return };
    let Some(ask) = iv.ask else { return };
    let typed = iv.input.text.clone();
    let op = match ask {
      Ask::Rename(slot) => {
        let Some(old) = iv.slots.iter().find(|s| s.slot == slot).map(|s| s.name.clone()) else {
          return self.inv_end_ask();
        };
        match inv::rename_ok(&iv.slots, slot, &typed) {
          Ok(name) if name == old => return self.inv_end_ask(),
          Ok(name) => Op::Rename(slot, old, name),
          Err(e) => return iv.msg = e.into(),
        }
      }
      Ask::Move(from) => match inv::slot_number(&typed) {
        Ok(to) if to == from => return self.inv_end_ask(),
        Ok(to) => Op::Move(from, to),
        Err(e) => return iv.msg = e.into(),
      },
      Ask::Import => match inv::channel(&typed) {
        Ok(c) => Op::Import(c),
        Err(e) => return iv.msg = e.into(),
      },
    };
    self.inv_end_ask();
    self.inv_act(op);
  }

  // y on a change that asked first
  fn inv_confirm(&mut self) {
    let Some(iv) = self.inv() else { return };
    let op = match iv.sure.take() {
      Some(Sure::Remove(slot)) => iv.slots.iter().find(|s| s.slot == slot).map(|s| Op::Remove(slot, s.name.clone())),
      Some(Sure::Apply(id)) => Some(Op::Apply(id)),
      Some(Sure::Unshare) => Some(Op::Unshare),
      None => None,
    };
    iv.msg.clear();
    if let Some(op) = op {
      self.inv_act(op);
    }
  }

  // enter / click on a row (or the retry row)
  pub(super) fn inv_pick(&mut self, t: Target) {
    let Some(iv) = self.inv() else { return };
    match (t, iv.tab) {
      (Target::Retry, tab) => {
        iv.load[tab.at()] = Load::Not;
        self.inv_load(tab);
      }
      (Target::Inv(i), Tab::Mine) => {
        if let Some(s) = iv.slots.get(i).map(|s| (s.slot, s.name.clone())) {
          self.inv_start_ask(Ask::Rename(s.0), &s.1);
        }
      }
      (Target::Inv(i), Tab::Removed) => {
        if let Some(id) = iv.gone.get(i).map(|g| g.id) {
          self.inv_act(Op::Restore(id));
        }
      }
      (Target::Inv(i), Tab::Sets) => {
        if let Some(id) = iv.sets.get(i).map(|s| s.id) {
          iv.sure = Some(Sure::Apply(id));
          iv.msg.clear();
        }
      }
      _ => {}
    }
  }

  // the screen's keys while it shows your emotes. true = spent
  pub(super) fn inv_key(&mut self, key: &Key<&str>, text: Option<&str>) -> bool {
    let (ctrl, alt, shift) = (self.mods.control_key(), self.mods.alt_key(), self.mods.shift_key());
    let accepts = self.ime.accepts_key_text();
    let Some(s) = self.screen.as_mut() else { return false };
    let at = s.target();
    let filtering = !s.q.text.trim().is_empty();
    let Some(iv) = s.inv.as_deref_mut() else { return false };
    // a typed answer has the keys
    if iv.ask.is_some() {
      match key {
        Key::Named(NamedKey::Escape) => self.inv_end_ask(),
        Key::Named(NamedKey::Enter) => self.inv_submit(),
        _ if super::tabs::edit_line(&mut iv.input, key, ctrl) => {}
        Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
          if let Some(t) = text.filter(|_| accepts) {
            iv.input.insert(t);
          }
        }
        _ => {}
      }
      return true;
    }
    if s.typing {
      return false;
    }
    if iv.sure.is_some() {
      match key {
        Key::Character("y") | Key::Named(NamedKey::Enter) => self.inv_confirm(),
        Key::Character("n") | Key::Named(NamedKey::Escape) => iv.sure = None,
        _ => {}
      }
      return true;
    }
    let tab = iv.tab;
    let row = match at {
      Some(Target::Inv(i)) => Some(i),
      _ => None,
    };
    if ctrl {
      return match key {
        Key::Character("r") if tab == Tab::Mine => {
          self.inv_act(Op::Redo);
          true
        }
        _ => false,
      };
    }
    match key {
      Key::Character("]") => self.inv_tab(tab.step(true)),
      Key::Character("[") => self.inv_tab(tab.step(false)),
      Key::Named(NamedKey::Tab) => self.inv_tab(tab.step(!shift)),
      Key::Character("R") => {
        iv.load[tab.at()] = Load::Not;
        self.inv_load(tab);
      }
      _ if tab != Tab::Mine => return false,
      Key::Character("u") => self.inv_act(Op::Undo),
      Key::Character("s") => self.inv_act(Op::Share),
      Key::Character("S") => {
        iv.sure = Some(Sure::Unshare);
        iv.msg.clear();
      }
      Key::Character("i") => self.inv_start_ask(Ask::Import, ""),
      Key::Character(k @ ("J" | "K")) => {
        let Some(i) = row else { return true };
        if filtering {
          iv.msg = t("settings.inventory.clear_filter").into();
          return true;
        }
        if let Some(to) = inv::neighbour(&iv.slots, i, *k == "J") {
          let from = iv.slots[i].slot;
          self.inv_act(Op::Move(from, to));
        }
      }
      Key::Character("n" | "m" | "x") if row.is_some() => {
        let Some(slot) = row.and_then(|i| iv.slots.get(i)).map(|s| (s.slot, s.name.clone())) else { return true };
        match key {
          Key::Character("n") => self.inv_start_ask(Ask::Rename(slot.0), &slot.1),
          Key::Character("m") => self.inv_start_ask(Ask::Move(slot.0), ""),
          _ => {
            iv.sure = Some(Sure::Remove(slot.0));
            iv.msg.clear();
          }
        }
      }
      _ => return false,
    }
    true
  }

  // an ask answered (or not)
  pub(super) fn on_inv(&mut self, op: Op, res: Result<Got, String>) {
    let Some(s) = self.screen.as_mut() else { return };
    let Some(iv) = s.inv.as_deref_mut() else { return };
    let list = match op {
      Op::List => Some(Tab::Mine),
      Op::Removed => Some(Tab::Removed),
      Op::Sets => Some(Tab::Sets),
      _ => None,
    };
    if list.is_none() {
      iv.acting = false;
    }
    let mut reload = None;
    let mut copy = None;
    match res {
      Err(e) => match list {
        Some(t) => iv.load[t.at()] = Load::Failed(e),
        None => {
          iv.msg = e;
          // a remove or move that missed: the list is out of date
          if matches!(op, Op::Remove(..) | Op::Move(..)) {
            iv.load[Tab::Mine.at()] = Load::Not;
            reload = Some(Tab::Mine).filter(|t| *t == iv.tab);
          }
        }
      },
      Ok(Got::List(l)) => (iv.slots, iv.load[0]) = (l, Load::Ready),
      Ok(Got::Removed(g)) => (iv.gone, iv.load[1]) = (g, Load::Ready),
      Ok(Got::Sets(l)) => (iv.sets, iv.load[2]) = (l, Load::Ready),
      Ok(Got::Renamed(slot, name)) => {
        inv::renamed(&mut iv.slots, slot, &name);
        iv.msg = f("settings.inventory.renamed", &[("name", &name)]);
      }
      Ok(Got::Moved(from, to)) => {
        inv::moved(&mut iv.slots, from, to);
        iv.msg = f("settings.inventory.moved", &[("to", &to.to_string())]);
        // the pick follows the emote it carried
        if let Some(p) = iv.shown(&s.q.text).into_iter().position(|i| iv.slots[i].slot == to) {
          s.st.at = p;
        }
      }
      Ok(Got::Dropped(slot, name)) => {
        inv::dropped(&mut iv.slots, slot);
        iv.load[Tab::Removed.at()] = Load::Not;
        iv.msg = f("settings.inventory.removed_msg", &[("name", &name)]);
      }
      Ok(Got::Changed(say)) => {
        // only a fresh read shows what these did
        iv.msg = say;
        iv.load[Tab::Mine.at()] = Load::Not;
        iv.load[Tab::Removed.at()] = Load::Not;
        reload = Some(iv.tab).filter(|t| *t != Tab::Sets);
      }
      Ok(Got::Shared(url)) => {
        iv.msg = f("settings.inventory.shared", &[("url", &url)]);
        copy = Some(url);
      }
      Ok(Got::Unshared) => iv.msg = t("settings.inventory.unshared").into(),
    }
    if let Some(t) = reload {
      self.inv_load(t);
    }
    if let Some(text) = copy {
      self.net.send(Cmd::Copy { text, done: String::new() });
    }
    if let Some(s) = self.screen.as_mut() {
      let (lines, _) = s.lines(&self.set);
      s.st.settle(&lines);
    }
    self.wake();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::{Net, NetEvent};

  fn slot(n: u32, name: &str) -> Slot {
    Slot { slot: n, name: name.into(), source: "7tv".into(), uses: 0, upload: false, zero_width: false }
  }

  fn app() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    let (net, rx) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    app.dpi = 1.0;
    app.res.set_scale(app.scale());
    app.resized(720, 400);
    (app, rx)
  }

  fn login(app: &mut App) {
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
  }

  fn asked(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) -> Vec<Op> {
    std::iter::from_fn(|| rx.try_recv().ok())
      .filter_map(|c| if let Cmd::Inventory(op) = c { Some(op) } else { None })
      .collect()
  }

  fn rows(app: &App) -> Vec<String> {
    let s = app.screen.as_ref().unwrap();
    let (lines, _) = s.inv.as_ref().unwrap().lines(&s.q.text);
    lines.into_iter().filter_map(|l| if let L::Row { a, .. } = l { Some(a) } else { None }).collect()
  }

  fn press(app: &mut App, keys: &[&'static str]) {
    for k in keys {
      app.on_key(Key::Character(k), Some(k));
    }
  }

  fn answer(app: &mut App, op: Op, res: Result<Got, String>) {
    app.on_net(NetEvent::Inv { op, res });
  }

  fn open(app: &mut App, rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
    login(app);
    app.inv_open();
    assert_eq!(asked(rx), [Op::List]);
    assert_eq!(rows(app), ["loading…"]);
    let l = vec![slot(1, "Kappa"), slot(2, "pog"), slot(9, "cat")];
    answer(app, Op::List, Ok(Got::List(l)));
  }

  #[test]
  fn logged_out_it_does_not_open() {
    let (mut app, mut rx) = app();
    app.inv_open();
    assert!(app.screen.is_none() && asked(&mut rx).is_empty());
  }

  #[test]
  fn open_rename_carry_remove_and_undo_from_the_keyboard() {
    let (mut app, mut rx) = app();
    open(&mut app, &mut rx);
    assert_eq!(rows(&app), ["Kappa", "pog", "cat"]);
    app.redraw();
    // j n: rename the second row. a name with a space is refused and nothing is sent
    press(&mut app, &["j", "n"]);
    assert!(app.screen.as_ref().unwrap().typing());
    app.on_key(Key::Named(NamedKey::Backspace), None);
    app.on_key(Key::Named(NamedKey::Backspace), None);
    app.on_key(Key::Named(NamedKey::Backspace), None);
    press(&mut app, &["a", " ", "b"]);
    app.on_key(Key::Named(NamedKey::Enter), None);
    assert!(asked(&mut rx).is_empty());
    assert_eq!(app.screen.as_ref().unwrap().inv.as_ref().unwrap().msg, "one word, no spaces, not starting with / or .");
    // a name another slot has is refused too
    for _ in 0..3 {
      app.on_key(Key::Named(NamedKey::Backspace), None);
    }
    press(&mut app, &["c", "a", "t"]);
    app.on_key(Key::Named(NamedKey::Enter), None);
    assert!(asked(&mut rx).is_empty());
    for _ in 0..3 {
      app.on_key(Key::Named(NamedKey::Backspace), None);
    }
    press(&mut app, &["P", "o", "g", "U"]);
    app.on_key(Key::Named(NamedKey::Enter), None);
    let rename = Op::Rename(2, "pog".into(), "PogU".into());
    assert_eq!(asked(&mut rx), vec![rename.clone()]);
    // nothing changes until heatsync says yes
    assert_eq!(rows(&app), ["Kappa", "pog", "cat"]);
    answer(&mut app, rename, Ok(Got::Renamed(2, "PogU".into())));
    assert_eq!(rows(&app), ["Kappa", "PogU", "cat"]);
    assert!(!app.screen.as_ref().unwrap().typing());
    // K carries it up: a swap with the neighbour's slot, and the pick follows the emote
    press(&mut app, &["K"]);
    assert_eq!(asked(&mut rx), [Op::Move(2, 1)]);
    answer(&mut app, Op::Move(2, 1), Ok(Got::Moved(2, 1)));
    assert_eq!(rows(&app), ["PogU", "Kappa", "cat"]);
    assert_eq!(app.screen.as_ref().unwrap().st.at, 0);
    // K at the top asks nothing
    press(&mut app, &["K"]);
    assert!(asked(&mut rx).is_empty());
    // x asks first; n keeps it, x y removes
    press(&mut app, &["j", "x"]);
    assert_eq!(rows(&app)[1], "remove \"Kappa\"?  y yes · n no");
    press(&mut app, &["n"]);
    assert_eq!(rows(&app)[1], "Kappa");
    press(&mut app, &["x", "y"]);
    let rm = Op::Remove(2, "Kappa".into());
    assert_eq!(asked(&mut rx), vec![rm.clone()]);
    answer(&mut app, rm, Ok(Got::Dropped(2, "Kappa".into())));
    assert_eq!(rows(&app), ["PogU", "cat"]);
    // u undoes: the list is read again
    press(&mut app, &["u"]);
    assert_eq!(asked(&mut rx), [Op::Undo]);
    answer(&mut app, Op::Undo, Ok(Got::Changed("undone".into())));
    assert_eq!(asked(&mut rx), [Op::List]);
    assert_eq!(rows(&app), ["loading…"]);
    answer(&mut app, Op::List, Ok(Got::List(vec![slot(1, "PogU"), slot(2, "Kappa"), slot(9, "cat")])));
    assert_eq!(rows(&app), ["PogU", "Kappa", "cat"]);
    app.redraw();
    app.on_key(Key::Named(NamedKey::Escape), None);
    assert!(app.screen.is_none(), "esc closes");
  }

  #[test]
  fn a_move_to_a_typed_slot_is_checked_then_sent_and_one_change_goes_at_a_time() {
    let (mut app, mut rx) = app();
    open(&mut app, &mut rx);
    press(&mut app, &["m", "5", "0", "0", "0"]);
    app.on_key(Key::Named(NamedKey::Enter), None);
    assert!(asked(&mut rx).is_empty(), "past the cap");
    for _ in 0..4 {
      app.on_key(Key::Named(NamedKey::Backspace), None);
    }
    press(&mut app, &["4", "0"]);
    app.on_key(Key::Named(NamedKey::Enter), None);
    assert_eq!(asked(&mut rx), [Op::Move(1, 40)]);
    // a second change before the first answers waits
    press(&mut app, &["u"]);
    assert!(asked(&mut rx).is_empty());
    assert_eq!(app.screen.as_ref().unwrap().inv.as_ref().unwrap().msg, "one moment…");
    answer(&mut app, Op::Move(1, 40), Ok(Got::Moved(1, 40)));
    assert_eq!(rows(&app), ["pog", "cat", "Kappa"]);
    // a failed move says why and reads the list again
    press(&mut app, &["m", "3"]);
    app.on_key(Key::Named(NamedKey::Enter), None);
    assert_eq!(asked(&mut rx), [Op::Move(40, 3)]);
    answer(&mut app, Op::Move(40, 3), Err("No emote in source slot".into()));
    assert_eq!(asked(&mut rx), [Op::List]);
    assert_eq!(app.screen.as_ref().unwrap().inv.as_ref().unwrap().msg, "No emote in source slot");
  }

  #[test]
  fn removed_restores_and_sets_apply_only_after_a_yes() {
    let (mut app, mut rx) = app();
    open(&mut app, &mut rx);
    press(&mut app, &["]"]);
    assert_eq!(asked(&mut rx), [Op::Removed]);
    answer(&mut app, Op::Removed, Ok(Got::Removed(vec![Gone { id: 77, name: "old".into(), slot: 4 }])));
    assert_eq!(rows(&app), ["old"]);
    app.on_key(Key::Named(NamedKey::Enter), None);
    assert_eq!(asked(&mut rx), [Op::Restore(77)]);
    answer(&mut app, Op::Restore(77), Ok(Got::Changed("restored".into())));
    assert_eq!(asked(&mut rx), [Op::Removed], "the tab you are on is read again");
    press(&mut app, &["]"]);
    assert_eq!(asked(&mut rx), [Op::Sets]);
    let sets = vec![Saved { id: 5, name: "my pogs".into(), count: 12, starred: true }];
    answer(&mut app, Op::Sets, Ok(Got::Sets(sets)));
    assert_eq!(rows(&app), ["★ my pogs"]);
    app.on_key(Key::Named(NamedKey::Enter), None);
    assert_eq!(rows(&app), ["replace your emotes with \"my pogs\"?  y yes · n no"]);
    assert!(asked(&mut rx).is_empty());
    press(&mut app, &["n"]);
    assert!(asked(&mut rx).is_empty());
    app.on_key(Key::Named(NamedKey::Enter), None);
    press(&mut app, &["y"]);
    assert_eq!(asked(&mut rx), [Op::Apply(5)]);
    answer(&mut app, Op::Apply(5), Ok(Got::Changed("set applied".into())));
    // mine and removed are stale now: coming back reads them
    press(&mut app, &["["]);
    press(&mut app, &["["]);
    assert_eq!(asked(&mut rx), [Op::Removed, Op::List]);
  }

  #[test]
  fn share_copies_the_link_and_unshare_asks_first() {
    let (mut app, mut rx) = app();
    open(&mut app, &mut rx);
    press(&mut app, &["s"]);
    assert_eq!(asked(&mut rx), [Op::Share]);
    answer(&mut app, Op::Share, Ok(Got::Shared("https://heatsync.org/set/AbCd1234".into())));
    let copied = std::iter::from_fn(|| rx.try_recv().ok())
      .any(|c| matches!(c, Cmd::Copy { text, .. } if text == "https://heatsync.org/set/AbCd1234"));
    assert!(copied);
    press(&mut app, &["S"]);
    assert!(asked(&mut rx).is_empty());
    press(&mut app, &["y"]);
    assert_eq!(asked(&mut rx), [Op::Unshare]);
  }

  #[test]
  fn a_failed_read_offers_retry_and_a_filter_narrows_the_list() {
    let (mut app, mut rx) = app();
    login(&mut app);
    app.inv_open();
    assert_eq!(asked(&mut rx), [Op::List]);
    answer(&mut app, Op::List, Err("heatsync is having trouble, try again soon".into()));
    assert_eq!(rows(&app), ["heatsync is having trouble, try again soon", "retry"]);
    app.on_key(Key::Named(NamedKey::ArrowDown), None);
    app.on_key(Key::Named(NamedKey::Enter), None);
    assert_eq!(asked(&mut rx), [Op::List]);
    answer(&mut app, Op::List, Ok(Got::List(vec![slot(1, "Kappa"), slot(2, "pog")])));
    press(&mut app, &["/", "p", "o"]);
    assert_eq!(rows(&app), ["pog"]);
    // carrying a slot through a filter would swap with something unseen
    app.on_key(Key::Named(NamedKey::Enter), None);
    press(&mut app, &["K"]);
    assert!(asked(&mut rx).is_empty());
  }

  #[test]
  fn tabs_wrap_both_ways() {
    assert_eq!(Tab::Mine.step(false), Tab::Sets);
    assert_eq!(Tab::Sets.step(true), Tab::Mine);
    assert_eq!(Tab::Removed.step(true), Tab::Sets);
  }
}
