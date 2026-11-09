// the lists you keep, as screens over the chat: who you muted, blocked or hid, your filter rules (hidden
// words, highlights and patterns are rules) and ban reasons. j/k move, x removes the picked one, a (or
// enter on the add row) types a new one, enter on a person opens their card, esc closes. every change is
// the slash command's own path (`filter_cmd`), so /mute, /rule and this screen can never disagree.
// the mention rules are the exception: they live on the server, so a change is asked of it (app/mrules.rs)
// and enter switches a rule on or off
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::filters::span_text;
use super::screen::{Page, Target};
use crate::model::filter;
use crate::model::filtercmd::{self, Cmd, RuleCmd};
use crate::model::line::Line;
use crate::model::modcmd::Verb;
use crate::model::mrules::{self, Rule};
use crate::ui::sheet::L;
use crate::util::time::now_ms;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
  People,
  Rules,
  Reasons,
  // the server's mention rules
  Alerts,
  // your own slash commands
  Commands,
  // the mod toolbar's buttons: each one on or off
  Buttons,
}

impl Kind {
  fn title(self) -> &'static str {
    match self {
      Kind::People => "muted & blocked people",
      Kind::Rules => "filter rules",
      Kind::Reasons => "ban reasons",
      Kind::Alerts => "mention rules",
      Kind::Commands => "your commands",
      Kind::Buttons => "mod toolbar buttons",
    }
  }

  fn addable(self) -> bool {
    !matches!(self, Kind::People | Kind::Buttons)
  }

  fn hint(self) -> &'static str {
    match self {
      Kind::People => "enter opens their card · x lets them back",
      Kind::Rules => "type hide or highlight, then a word: hide spoilers \u{b7} ? more",
      Kind::Reasons => "quick reasons offered when you ban",
      Kind::Buttons => {
        "enter turns a button on or off \u{b7} they show on the line under the pointer in rooms you moderate"
      }
      Kind::Commands => "type: /name what it says. {1} {2} words · {1+} the rest · {input} all · {channel} · {user}",
      Kind::Alerts => "the server watches every channel for a word or phrase and pings you here \u{b7} ? more",
    }
  }

  // the advanced syntax, behind ?
  fn more(self) -> Option<&'static str> {
    match self {
      Kind::Rules => Some(
        "more kinds: put user, badge, msgtype, regex or expr before the value, like highlight user bob or hide regex ^! \u{b7} a highlight can play your own wav: /rule sound <number> <path>",
      ),
      Kind::Alerts => Some("more: phrase big news \u{b7} regex ^hi \u{b7} end with #channel to keep it to one channel"),
      _ => None,
    }
  }

  fn adding(self) -> &'static str {
    match self {
      Kind::Rules => "hide spoilers",
      Kind::Reasons => "a reason",
      Kind::Alerts => "a word or phrase",
      Kind::Commands => "/gg good game {1}",
      _ => "a word or phrase",
    }
  }

  fn none(self) -> &'static str {
    match self {
      Kind::People => "nobody muted, blocked or hidden",
      _ => "nothing yet",
    }
  }
}

// what x does to a row
#[derive(Clone, PartialEq, Eq, Debug)]
enum Del {
  Unmute(String),
  Unblock(String),
  Unhide(String),
  Reason(String),
  // its number in /rule list
  Rule(usize),
  // a mention rule, by the server's id
  Alert(u64),
  // one of your commands, by name
  Command(String),
  // a mod toolbar button, by its place in the catalog
  Button(usize),
}

struct Item {
  text: String,
  detail: String,
  del: Del,
}

pub(super) struct Ls {
  pub kind: Kind,
  // who is hidden in the tab the screen was opened from
  hidden: Vec<Box<str>>,
  pub adding: bool,
  pub add: Line,
  // the rule being rewritten in the add box (its number), else None: enter saves it where it stands
  pub editing: Option<usize>,
  // what the last change said, under the list
  pub msg: String,
  // the mention rules as the app holds them, and what an empty list says (signed out, still reading, none)
  mr: Vec<Rule>,
  mr_note: &'static str,
  // your commands as they stand
  cmds: Vec<(String, String)>,
  // the mod toolbar's picked buttons
  mask: u16,
}

impl Ls {
  fn items(&self) -> Vec<Item> {
    let item = |text: &str, detail: &str, del| Item { text: text.to_owned(), detail: detail.to_owned(), del };
    filter::with(|f| match self.kind {
      Kind::People => {
        let now = now_ms();
        let mut v: Vec<Item> = f
          .mutes()
          .into_iter()
          .filter(|(_, until)| until.is_none_or(|u| u > now))
          .map(|(n, until)| {
            let left = until.map_or("forever".to_owned(), |u| format!("{} left", span_text((u - now) / 1000)));
            item(n, &format!("muted · {left}"), Del::Unmute(n.to_owned()))
          })
          .collect();
        v.extend(f.blocks_sorted().iter().map(|n| item(n, "blocked", Del::Unblock(n.clone()))));
        v.extend(self.hidden.iter().map(|n| item(n, "hidden in this tab", Del::Unhide(n.to_string()))));
        v
      }
      Kind::Buttons => crate::model::modbar::CATALOG
        .iter()
        .enumerate()
        .map(|(i, c)| {
          let what = match c.verb {
            Verb::Delete => "delete the line".to_owned(),
            Verb::Timeout(s) => format!("time out for {}", span_text(i64::from(s))),
            Verb::Purge => "clear their lines".to_owned(),
            Verb::Ban => "ban".to_owned(),
            Verb::Unban => "unban".to_owned(),
          };
          let on = if self.mask >> i & 1 == 1 { "on" } else { "off" };
          item(&format!("{} \u{b7} {what}", c.label), on, Del::Button(i))
        })
        .collect(),
      Kind::Commands => self.cmds.iter().map(|(n, e)| item(&format!("/{n}"), e, Del::Command(n.clone()))).collect(),
      Kind::Reasons => f.reasons().iter().map(|r| item(r, "", Del::Reason(r.to_string()))).collect(),
      Kind::Alerts => self
        .mr
        .iter()
        .map(|r| {
          let (pattern, detail) = mrules::describe(r);
          item(&pattern, &detail, Del::Alert(r.id))
        })
        .collect(),
      Kind::Rules => f
        .rules()
        .describe()
        .iter()
        .enumerate()
        .map(|(i, d)| item(d.split_once(' ').map_or(&**d, |x| x.1), &f.rules().extras(i), Del::Rule(i + 1)))
        .collect(),
    })
  }

  pub(super) fn lines(&self) -> (Vec<L>, Vec<Target>) {
    let items = self.items();
    let row = |a: String, b: String| L::Row { a, b, pill: None };
    let mut lines: Vec<L> = items.iter().map(|i| row(i.text.clone(), i.detail.clone())).collect();
    let mut marks: Vec<Target> = (0..items.len()).map(Target::Item).collect();
    if items.is_empty() {
      let none = if self.kind == Kind::Alerts { self.mr_note } else { self.kind.none() };
      lines.push(row(none.into(), String::new()));
      marks.push(Target::None);
    }
    if self.kind.addable() {
      lines.push(row("add…".into(), "a".into()));
      marks.push(Target::AddItem);
    }
    (lines, marks)
  }

  pub(super) fn title(&self) -> String {
    let n = self.items().len();
    if n == 0 { self.kind.title().to_owned() } else { format!("{} · {n}", self.kind.title()) }
  }

  pub(super) fn tip(&self) -> &str {
    if self.msg.is_empty() { self.kind.hint() } else { &self.msg }
  }

  pub(super) fn keys(&self) -> &'static str {
    match (self.adding, self.kind.addable()) {
      (true, _) if self.editing.is_some() => "enter saves · esc leaves it as it was",
      (true, _) => "enter adds · esc stops",
      (_, true) if self.kind == Kind::Alerts => "j k move · a add · enter on or off · x delete · ? more · esc closes",
      (_, true) if self.kind == Kind::Rules => {
        "j k move · a add · e edit · n notify · s sound · + - volume · x delete · ? more · esc closes"
      }
      (_, true) => "j k move · a add · x delete · esc closes",
      _ if self.kind == Kind::Buttons => "j k move · enter on or off · esc closes",
      _ => "j k move · enter opens their card · x lets them back · esc closes",
    }
  }

  pub(super) fn prompt(&self) -> String {
    format!("{} · ", self.kind.adding())
  }
}

impl App {
  pub(super) fn list_open(&mut self, kind: Kind) {
    let hidden = self.tabs.get(self.active).map(|t| t.view.hidden_users()).unwrap_or_default();
    let (mr, mr_note) = (self.mrules.clone(), self.mrules_note());
    self.open_screen(Page::List);
    if let Some(s) = &mut self.screen {
      let cmds = self.ucmds.iter().map(|(n, e)| (n.to_owned(), e.to_owned())).collect();
      let mask = self.set.mod_buttons;
      s.ls = Some(Box::new(Ls {
        kind,
        hidden,
        adding: false,
        add: Line::default(),
        editing: None,
        msg: String::new(),
        mr,
        mr_note,
        cmds,
        mask,
      }));
    }
    if kind == Kind::Alerts {
      // another device may have changed them since the last read
      self.mrules_look();
    }
  }

  // the mention rules changed (or were read): the open list shows the new ones
  pub(super) fn list_sync(&mut self) {
    let (mr, mr_note) = (self.mrules.clone(), self.mrules_note());
    if let Some(l) = self.ls() {
      (l.mr, l.mr_note) = (mr, mr_note);
    }
  }

  fn list_cmds_sync(&mut self) {
    let cmds = self.ucmds.iter().map(|(n, e)| (n.to_owned(), e.to_owned())).collect();
    if let Some(l) = self.ls() {
      l.cmds = cmds;
    }
  }

  // a word under the open list, or in the status line when no list is open
  pub(super) fn list_note(&mut self, text: String) {
    match self.ls() {
      Some(l) => l.msg = text,
      None => self.say(text),
    }
  }

  fn ls(&mut self) -> Option<&mut Ls> {
    self.screen.as_mut()?.ls.as_deref_mut()
  }

  // what the status line said last (the slash command's own words), kept under the list
  fn said(&self) -> String {
    self.tabs.get(self.active).and_then(|t| t.view.msgs().next_back()).map(|m| m.body.to_string()).unwrap_or_default()
  }

  fn list_run(&mut self, cmd: Cmd) {
    self.filter_cmd(cmd);
    let said = self.said();
    if let Some(l) = self.ls() {
      l.msg = said;
    }
  }

  // x on the picked row
  fn list_delete(&mut self, i: usize) {
    let Some(item) = self.ls().and_then(|l| l.items().into_iter().nth(i)) else { return };
    match item.del {
      Del::Unmute(n) => self.list_run(Cmd::Unmute(n)),
      Del::Unblock(n) => self.list_run(Cmd::Unblock(n)),
      Del::Unhide(n) => {
        self.list_run(Cmd::Hide(n.clone(), false));
        if let Some(l) = self.ls() {
          l.hidden.retain(|h| **h != *n);
        }
      }
      Del::Reason(text) => self.list_run(Cmd::Reason { add: false, text: Some(text) }),
      Del::Rule(n) => self.list_run(Cmd::Rule(RuleCmd::Del(n.to_string()))),
      Del::Alert(id) => self.mrules_remove(id),
      Del::Button(_) => {}
      Del::Command(n) => {
        let said = self.ucmd_remove(&n);
        self.list_cmds_sync();
        self.list_note(said);
      }
    }
  }

  // enter on the typed line
  fn list_add(&mut self) {
    let Some(l) = self.ls() else { return };
    let (kind, typed, editing) = (l.kind, l.add.text.trim().to_owned(), l.editing);
    if typed.is_empty() {
      return;
    }
    if kind == Kind::Alerts {
      // kept in the box when refused, so it can be fixed
      let said = self.mrules_add(&typed);
      let sent = said.is_none();
      self.list_note(said.unwrap_or_else(|| "adding…".to_owned()));
      if let Some(l) = self.ls().filter(|_| sent) {
        l.add.set_text(String::new());
        l.add.home();
      }
      return;
    }
    if kind == Kind::Commands {
      // kept in the box when refused, so it can be fixed
      match self.ucmd_add(&typed) {
        Err(e) => return self.list_note(e),
        Ok(said) => {
          self.list_cmds_sync();
          self.list_note(said);
        }
      }
      if let Some(l) = self.ls() {
        l.add.set_text(String::new());
        l.add.home();
      }
      return;
    }
    let cmd = match kind {
      Kind::Reasons => Cmd::Reason { add: true, text: Some(typed) },
      Kind::Rules => match filtercmd::command(&format!(
        "/rule {} {typed}",
        editing.map_or("add".to_owned(), |n| format!("edit {n}"))
      )) {
        Some(c) => c,
        None => return,
      },
      Kind::People | Kind::Alerts | Kind::Commands | Kind::Buttons => return,
    };
    // a rule that would not save stays in the box, with why, so it can be fixed
    let refused = editing.is_some() && matches!(cmd, Cmd::Usage(_));
    self.list_run(cmd);
    let said = self.said();
    let kept = editing.is_some() && !said.starts_with("rule changed");
    if refused || kept {
      return;
    }
    if let Some(l) = self.ls() {
      l.add.set_text(String::new());
      l.add.home();
      if l.editing.take().is_some() {
        l.adding = false;
        if let Some(s) = self.screen.as_mut() {
          s.typing = false;
        }
      }
    }
  }

  // e on a rule: its words come into the add box, enter saves them back into the same rule
  fn list_edit(&mut self, i: usize) {
    let Some(l) = self.ls().filter(|l| l.kind == Kind::Rules) else { return };
    let Some(v) = filter::with(|f| f.rules().view(i)) else { return };
    l.editing = Some(i + 1);
    l.add.set_text(format!("{} {} {}", v.action, v.ty, v.value));
    l.add.end();
    l.msg.clear();
    l.adding = true;
    if let Some(s) = self.screen.as_mut() {
      s.typing = true;
    }
    self.list_preview();
  }

  // n / s / + - on a highlight rule: notify, the next sound, the volume. every one is the slash command's own path
  fn list_rule_tweak(&mut self, i: usize, key: &str) {
    let Some(v) = filter::with(|f| f.rules().view(i)) else { return };
    let n = (i + 1).to_string();
    let cmd = match key {
      "n" => RuleCmd::Notify(n, !v.notify),
      "s" => {
        // off, then each preset in turn, then off again; a file of yours is set with /rule sound
        let order = ["ping", "blip", "knock", "chime"];
        let next = match v.sound.as_deref() {
          None => order[0],
          Some(cur) => order.iter().position(|o| *o == cur).and_then(|p| order.get(p + 1)).copied().unwrap_or("off"),
        };
        RuleCmd::Sound(n, next.to_owned())
      }
      _ => {
        let now = v.volume.unwrap_or_else(|| self.set.mention_volume.percent());
        let to = if key == "+" { now.saturating_add(10).min(100) } else { now.saturating_sub(10) };
        RuleCmd::Volume(n, Some(to))
      }
    };
    self.list_run(Cmd::Rule(cmd));
  }

  // the add box while it holds a rule: how many of the lines already here it would catch (ext "matches n of the
  // last 200 messages"), from the real matcher
  pub(super) fn list_preview(&mut self) {
    const ROWS: usize = 200;
    let Some(l) = self.ls().filter(|l| l.kind == Kind::Rules && l.adding) else { return };
    let (typed, case) = (
      l.add.text.trim().to_owned(),
      l.editing.and_then(|n| filter::with(|f| f.rules().view(n - 1))).is_some_and(|v| v.case),
    );
    let text = if typed.is_empty() {
      String::new()
    } else {
      let parts = match filtercmd::command(&format!("/rule add {typed}")) {
        Some(Cmd::Rule(RuleCmd::Add { action, ty, value })) => Some((action, ty, value)),
        _ => None,
      };
      let tried = parts.and_then(|(a, t, v)| {
        let rows = self.tabs.get(self.active)?.view.msgs().rev().take(ROWS).collect::<Vec<_>>();
        crate::model::rules::Rules::preview(&a, &t, &v, case, rows.into_iter(), "")
      });
      match tried {
        None => "that isn't a rule yet".to_owned(),
        Some((_, 0)) => "no lines here to try it on yet".to_owned(),
        Some((0, n)) => format!("matches none of the last {n} messages"),
        Some((h, n)) => format!("matches {h} of the last {n} messages"),
      }
    };
    if let Some(l) = self.ls() {
      l.msg = text;
    }
  }

  fn list_start_add(&mut self) {
    if let Some(s) = self.screen.as_mut()
      && let Some(l) = s.ls.as_deref_mut()
      && l.kind.addable()
    {
      (l.adding, s.typing) = (true, true);
      l.msg.clear();
    }
  }

  // enter / click on a row: a person opens their card, the add row starts typing
  pub(super) fn list_pick(&mut self, t: Target) {
    match t {
      Target::AddItem => self.list_start_add(),
      Target::Item(i) => {
        if let Some(Del::Alert(id)) =
          self.ls().filter(|l| l.kind == Kind::Alerts).and_then(|l| l.items().into_iter().nth(i)).map(|x| x.del)
        {
          return self.mrules_toggle(id);
        }
        if let Some(Del::Button(b)) =
          self.ls().filter(|l| l.kind == Kind::Buttons).and_then(|l| l.items().into_iter().nth(i)).map(|x| x.del)
        {
          super::modseed::mod_buttons_decided();
          self.set.mod_buttons ^= 1 << b;
          let mask = self.set.mod_buttons;
          self.set.save();
          if let Some(l) = self.ls() {
            l.mask = mask;
          }
          return self.wake();
        }
        let Some(l) = self.ls().filter(|l| l.kind == Kind::People) else { return };
        let Some(name) = l.items().into_iter().nth(i).map(|x| x.text) else { return };
        self.close_screen();
        self.user_cmd(&name);
      }
      _ => {}
    }
  }

  // the screen's keys while it shows a list. true = spent
  pub(super) fn list_key(&mut self, key: &Key<&str>, text: Option<&str>) -> bool {
    let (ctrl, alt) = (self.mods.control_key(), self.mods.alt_key());
    let accepts = self.ime.accepts_key_text();
    let Some(s) = self.screen.as_mut() else { return false };
    let Some(l) = s.ls.as_deref_mut() else { return false };
    if l.adding {
      match key {
        Key::Named(NamedKey::Escape) if l.editing.is_some() => {
          (l.adding, s.typing, l.editing) = (false, false, None);
          l.add.set_text(String::new());
          l.add.home();
          l.msg.clear();
        }
        Key::Named(NamedKey::Escape) if !l.add.text.is_empty() => {
          l.add.set_text(String::new());
          l.add.home();
        }
        Key::Named(NamedKey::Escape) => (l.adding, s.typing) = (false, false),
        Key::Named(NamedKey::Enter) => self.list_add(),
        _ if super::tabs::edit_line(&mut l.add, key, ctrl) => {}
        Key::Character(_) | Key::Named(NamedKey::Space) if !ctrl && !alt => {
          if let Some(t) = text.filter(|_| accepts) {
            l.add.insert(t);
          }
        }
        _ => {}
      }
      // a typed key moves the count; enter has just said what it did, and that stays
      if *key != Key::Named(NamedKey::Enter) {
        self.list_preview();
      }
      return true;
    }
    match key {
      Key::Character(k @ ("e" | "n" | "s" | "+" | "-" | "=")) if !ctrl && !alt && l.kind == Kind::Rules => {
        let k: &str = if *k == "=" { "+" } else { k };
        if let Some(Target::Item(i)) = s.target() {
          match k {
            "e" => self.list_edit(i),
            _ => self.list_rule_tweak(i, k),
          }
        }
      }
      Key::Character("x") | Key::Named(NamedKey::Delete) if !ctrl => {
        if let Some(Target::Item(i)) = s.target() {
          self.list_delete(i);
        }
      }
      Key::Character("a") if !ctrl => self.list_start_add(),
      Key::Character("?") if l.kind.more().is_some() => l.msg = l.kind.more().unwrap_or_default().to_owned(),
      _ => return false,
    }
    true
  }
}
