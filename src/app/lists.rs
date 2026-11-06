// the lists you keep, as screens over the chat: who you muted, blocked or hid, and your highlight words,
// hidden words, automod patterns, filter rules and ban reasons. j/k move, x removes the picked one, a (or
// enter on the add row) types a new one, enter on a person opens their card, esc closes. every change is
// the slash command's own path (`filter_cmd`), so /mute, /unhighlight and this screen can never disagree.
// the mention rules are the exception: they live on the server, so a change is asked of it (app/mrules.rs)
// and enter switches a rule on or off
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::filters::span_text;
use super::screen::{Page, Target};
use crate::model::filter;
use crate::model::filtercmd::{self, Cmd, RuleCmd};
use crate::model::line::Line;
use crate::model::mrules::{self, Rule};
use crate::ui::sheet::L;
use crate::util::time::now_ms;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
  People,
  // true = the highlight words, false = the hidden ones
  Words(bool),
  Patterns,
  Rules,
  Reasons,
  // the server's mention rules
  Alerts,
}

impl Kind {
  fn title(self) -> &'static str {
    match self {
      Kind::People => "muted & blocked people",
      Kind::Words(true) => "highlight words",
      Kind::Words(false) => "hidden words",
      Kind::Patterns => "automod patterns",
      Kind::Rules => "filter rules",
      Kind::Reasons => "ban reasons",
      Kind::Alerts => "mention rules",
    }
  }

  fn addable(self) -> bool {
    self != Kind::People
  }

  fn hint(self) -> &'static str {
    match self {
      Kind::People => "enter opens their card · x lets them back",
      Kind::Words(true) => "messages with these words are tinted. whole word, any case",
      Kind::Words(false) => "messages with these words are hidden",
      Kind::Patterns => "messages matching a pattern are hidden (a regex, or plain text if it isn't one)",
      Kind::Rules => "type: hide or highlight, then keyword regex user badge msgtype or expr, then the value",
      Kind::Reasons => "quick reasons offered when you ban",
      Kind::Alerts => {
        "the server watches every channel for these and pings you here. a word, or: phrase big news · regex ^hi · end with #channel to limit it"
      }
    }
  }

  fn adding(self) -> &'static str {
    match self {
      Kind::Rules => "hide keyword spoilers",
      Kind::Patterns => "a pattern",
      Kind::Reasons => "a reason",
      Kind::Alerts => "a word, phrase or regex",
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
  Word(bool, String),
  Pattern(String),
  Reason(String),
  // its number in /rule list
  Rule(usize),
  // a mention rule, by the server's id
  Alert(u64),
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
  // what the last change said, under the list
  pub msg: String,
  // the mention rules as the app holds them, and what an empty list says (signed out, still reading, none)
  mr: Vec<Rule>,
  mr_note: &'static str,
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
      Kind::Words(m) => f.words(m).iter().map(|w| item(w, "", Del::Word(m, w.to_string()))).collect(),
      Kind::Patterns => f.patterns().iter().map(|p| item(p, "", Del::Pattern(p.to_string()))).collect(),
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
        .map(|(i, d)| item(d.split_once(' ').map_or(&**d, |x| x.1), "", Del::Rule(i + 1)))
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
      (true, _) => "enter adds · esc stops",
      (_, true) if self.kind == Kind::Alerts => "j k move · a add · enter on or off · x delete · esc closes",
      (_, true) => "j k move · a add · x delete · esc closes",
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
      s.ls = Some(Box::new(Ls { kind, hidden, adding: false, add: Line::default(), msg: String::new(), mr, mr_note }));
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
      Del::Word(marks, term) => self.list_run(Cmd::Word { marks, add: false, term: Some(term) }),
      Del::Pattern(pat) => self.list_run(Cmd::Pattern { add: false, pat: Some(pat) }),
      Del::Reason(text) => self.list_run(Cmd::Reason { add: false, text: Some(text) }),
      Del::Rule(n) => self.list_run(Cmd::Rule(RuleCmd::Del(n.to_string()))),
      Del::Alert(id) => self.mrules_remove(id),
    }
  }

  // enter on the typed line
  fn list_add(&mut self) {
    let Some(l) = self.ls() else { return };
    let (kind, typed) = (l.kind, l.add.text.trim().to_owned());
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
    let cmd = match kind {
      Kind::Words(marks) => Cmd::Word { marks, add: true, term: Some(typed) },
      Kind::Patterns => Cmd::Pattern { add: true, pat: Some(typed) },
      Kind::Reasons => Cmd::Reason { add: true, text: Some(typed) },
      Kind::Rules => match filtercmd::command(&format!("/rule add {typed}")) {
        Some(c) => c,
        None => return,
      },
      Kind::People | Kind::Alerts => return,
    };
    self.list_run(cmd);
    if let Some(l) = self.ls() {
      l.add.set_text(String::new());
      l.add.home();
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
      return true;
    }
    match key {
      Key::Character("x") | Key::Named(NamedKey::Delete) if !ctrl => {
        if let Some(Target::Item(i)) = s.target() {
          self.list_delete(i);
        }
      }
      Key::Character("a") if !ctrl => self.list_start_add(),
      _ => return false,
    }
    true
  }
}
