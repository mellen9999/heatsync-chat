// mention rules, the app's half (ext multichat/mention-rules.js + main.js `mention_rule_match`): the account's
// rules are read at login and again when the server says another client changed them; the list screen
// (app/lists.rs) adds, switches and removes them through the server; and a `mention:rule-match` hit lands
// in the mentions tab as a row that pings like any @. a line that already pinged you through your name is
// one row, not two
use super::App;
use crate::i18n::{f, t};
use crate::model::filter;
use crate::model::livegate::Ask;
use crate::model::message::{Extra, Kind as MsgKind, Message, Platform, Seg, flag};
use crate::model::mrules::{self, Hit};
use crate::model::tabs::Kind;
use crate::model::unseen::Surface;
use crate::net::mrules::{Fail, Got, Op, words};
use crate::net::{Cmd, social};
use crate::ui::theme;
use crate::util::notify;
use crate::util::time::now_ms;

// a rule's row and the line it matched were one ping if they came this close together
const SAME_LINE_MS: i64 = 15_000;
// rows of the mentions tab looked at for the same line
const LOOK_BACK: usize = 24;

// two texts of one line: the server's snippet may be cut short, or have its emote tokens stripped
fn same_words(a: &str, b: &str) -> bool {
  let (a, b) = (a.trim(), b.trim());
  !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

impl App {
  fn mrules_send(&self, op: Op) {
    self.net.send(Cmd::Social(social::Cmd::MentionRules(op)));
  }

  // signed in: read the account's rules
  pub(super) fn mrules_ask(&mut self) {
    if self.me.is_some() {
      self.mrules_send(Op::List);
    }
  }

  // logged out, or another account over this one: its rules are not the next one's
  pub(super) fn mrules_forget(&mut self) {
    self.mrules.clear();
    self.mrules_read = false;
    self.list_sync();
  }

  // the list screen opened: they may have changed on another device, so look again (one ask per burst)
  pub(super) fn mrules_look(&mut self) {
    self.on_refetch(Ask::MentionRules);
  }

  // what the list says when it has no rows
  pub(super) fn mrules_note(&self) -> &'static str {
    match (self.me.is_some(), self.mrules_read) {
      (false, _) => t("mod.mrules.note_login"),
      (true, false) => t("mod.mrules.note_reading"),
      (true, true) => t("mod.mrules.note_empty"),
    }
  }

  pub(super) fn on_mrules(&mut self, res: Result<Got, Fail>) {
    // an answer that outlived its login seeds nothing
    if self.me.is_none() {
      return;
    }
    match res {
      Ok(Got::List(v)) => {
        self.mrules = v;
        self.mrules_read = true;
        self.list_sync();
      }
      // added, switched or removed: read the list as the server now has it
      Ok(Got::Changed) => self.mrules_look(),
      Err(f) => {
        self.list_note(words(&f));
        // a switch or removal shown before the server agreed is put right by what it holds
        if f != Fail::Auth {
          self.mrules_send(Op::List);
        }
      }
    }
    self.wake();
  }

  // a typed rule from the list. None = it went to the server; Some = why not (the box keeps the text)
  pub(super) fn mrules_add(&mut self, text: &str) -> Option<String> {
    if self.me.is_none() {
      return Some(words(&Fail::Auth));
    }
    match mrules::parse_add(text) {
      Ok(body) => {
        self.mrules_send(Op::Add(body));
        None
      }
      Err(why) => Some(why.to_owned()),
    }
  }

  // enter on a rule: on <-> off. shown at once; the server's answer settles it
  pub(super) fn mrules_toggle(&mut self, id: u64) {
    let Some(r) = self.mrules.iter_mut().find(|r| r.id == id) else { return };
    r.on = !r.on;
    let on = r.on;
    self.mrules_send(Op::Toggle { id, on });
    self.list_sync();
    self.wake();
  }

  pub(super) fn mrules_remove(&mut self, id: u64) {
    self.mrules.retain(|r| r.id != id);
    self.mrules_send(Op::Remove(id));
    self.list_sync();
    self.wake();
  }

  // the mentions tab already holds this line: the ping through your name, or an earlier hit of a rule
  pub(super) fn mention_dup(&self, p: Platform, room: &str, login: &str, body: &str) -> bool {
    let Some(i) = self.tab_of(Kind::Mentions) else { return false };
    let now = now_ms();
    self.tabs[i].view.msgs().rev().take(LOOK_BACK).any(|m| {
      m.platform == p
        && now.saturating_sub(m.ts) < SAME_LINE_MS
        && m.login.eq_ignore_ascii_case(login)
        && m.x.as_ref().is_some_and(|x| x.chan.eq_ignore_ascii_case(room))
        && same_words(&m.body, body)
    })
  }

  // the server saw a line match one of your rules: a row of the mentions tab, red like a ping, that pings
  // when the window is away. a blocked person never pings you, rules or not (ext main.js)
  pub(super) fn on_rule_hit(&mut self, h: Hit) {
    if self.me.is_none() || filter::with(|f| f.blocked(&filter::key(&h.user))) {
      return;
    }
    let Some(i) = self.tab_of(Kind::Mentions) else { return };
    if self.mention_dup(h.platform, &h.channel, &h.user, &h.snippet) {
      return;
    }
    let now = now_ms();
    let mut m = Message::system(now, h.platform, "");
    m.kind = MsgKind::Chat;
    m.id = format!("rule-{now}-{}", self.tabs[i].view.len()).into();
    m.flags = flag::ALERT;
    m.login = h.user.to_lowercase().into();
    m.nick = h.user.clone();
    m.color = theme::WHITE;
    m.body = h.snippet.clone();
    m.x = Some(Box::new(Extra {
      chan: h.channel.clone(),
      head: Box::new([Seg { text: t("mod.mrules.tag").into(), ink: theme::CYAN, bold: false }]),
      ..Extra::default()
    }));
    self.tabs[i].chat().push(m);
    self.unseen_note(Surface::Mentions);
    self.mentions_changed();
    let title = f("mod.mrules.alert", &[("user", &notify::name(&h.user)), ("chan", &notify::name(&h.channel))]);
    self.mention_alert(&title, &h.user, &h.snippet);
    self.wake();
  }
}
