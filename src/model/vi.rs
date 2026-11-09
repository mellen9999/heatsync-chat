// vim normal mode for the message box. pure: keys in, edits to `Line` out. spec: the ext's vi-mode.js
// internally works on chars (positions are char indices); `Line` keeps the byte cursor in sync
use super::line::{Line, MAX_CHARS};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
  #[default]
  Insert,
  // vim normal mode on the message box; a picked line and the space leader reach the chat from here
  Normal,
}

#[derive(PartialEq, Eq, Debug)]
pub enum Action {
  None,
  // switch to insert mode (the edit, if any, is already done)
  Insert,
  Send,
  HistoryOlder,
  HistoryNewer,
  // esc with nothing pending: let go of the reply or the search
  Leave,
}

const UNDO_CAP: usize = 100;

// text + cursor as units, for the duration of one key (and kept whole for undo). a unit is one char, or a
// handle char standing for an emoji (a grapheme of several chars) or an emote stack, which vi steps over,
// deletes and yanks whole (the ext's atoms)
#[derive(Clone)]
struct Buf {
  c: Vec<char>,
  pos: usize,
  // what each handle stands for, and whether it's an emote
  units: Vec<(Box<str>, bool)>,
}

// handles live in a private-use plane; a typed char from it gets a handle of its own, so none is ambiguous
const HANDLE: u32 = 0xF0000;

fn handle_of(c: char) -> Option<usize> {
  (c as u32).checked_sub(HANDLE).map(|i| i as usize)
}

impl Buf {
  fn load(line: &Line) -> Buf {
    let mut b = Buf { c: Vec::new(), pos: 0, units: Vec::new() };
    for (r, emote) in line.units() {
      if r.end <= line.cursor {
        b.pos += 1;
      }
      b.push(&line.text[r], emote);
    }
    b
  }

  fn push(&mut self, s: &str, emote: bool) {
    let mut cs = s.chars();
    match (cs.next(), cs.next()) {
      (Some(c), None) if !emote && handle_of(c).is_none() => self.c.push(c),
      _ => {
        self.c.push(char::from_u32(HANDLE + self.units.len() as u32).unwrap_or(' '));
        self.units.push((s.into(), emote));
      }
    }
  }

  fn unit(&self, c: char) -> Option<&(Box<str>, bool)> {
    handle_of(c).and_then(|i| self.units.get(i))
  }

  fn expand(&self, cs: &[char]) -> String {
    let mut out = String::new();
    for &c in cs {
      match self.unit(c) {
        Some((s, _)) => out.push_str(s),
        None => out.push(c),
      }
    }
    out
  }

  // the units as text, to compare two loads
  fn strs(&self) -> Vec<String> {
    self.c.iter().map(|&c| self.expand(&[c])).collect()
  }

  fn text(&self) -> String {
    self.expand(&self.c)
  }

  fn store(&self, line: &mut Line) {
    let text = self.text();
    let cursor = self.expand(&self.c[..self.pos.min(self.c.len())]).len();
    if text != line.text {
      line.set_text(text);
    }
    line.cursor = cursor;
  }

  fn len(&self) -> usize {
    self.c.len()
  }

  // last valid normal-mode cursor spot
  fn last(&self) -> usize {
    self.c.len().saturating_sub(1)
  }

  fn clamp(&mut self) {
    self.pos = self.pos.min(self.last());
  }

  fn slice(&self, a: usize, b: usize) -> String {
    self.expand(&self.c[a..b])
  }

  // word class of each unit: blank, word, punctuation, emote (`big`: blank or not)
  fn classes(&self, big: bool) -> Vec<u8> {
    self
      .c
      .iter()
      .map(|&c| match self.unit(c) {
        Some((_, true)) => {
          if big {
            1
          } else {
            3
          }
        }
        Some((s, false)) => s.chars().next().map_or(0, |c| cls(c, big)),
        None => cls(c, big),
      })
      .collect()
  }

  fn chars(&self, a: usize, b: usize) -> usize {
    self.slice(a, b).chars().count()
  }

  // replace units a..e; returns how many went in (control bytes become spaces, line cap holds)
  fn splice(&mut self, a: usize, e: usize, with: &str) -> usize {
    let mut room = MAX_CHARS.saturating_sub(self.chars(0, a) + self.chars(e, self.len()));
    let mut ins = Buf { c: Vec::new(), pos: 0, units: std::mem::take(&mut self.units) };
    let clean: String = crate::util::blank(with).collect();
    for r in crate::model::line::breaks(&clean).collect::<Vec<_>>().windows(2) {
      let g = &clean[r[0]..r[1]];
      let n = g.chars().count();
      if n > room {
        break;
      }
      room -= n;
      ins.push(g, false);
    }
    self.units = ins.units;
    let n = ins.c.len();
    self.c.splice(a..e, ins.c);
    n
  }
}

// last repeatable edit: replay the keys, then splice what insert mode added
#[derive(Clone)]
struct Edit {
  keys: Vec<String>,
  del: usize,
  ins: String,
}

// an insert session in progress
struct Rec {
  before: Vec<String>,
  keys: Vec<String>,
}

#[derive(Default)]
pub struct Vi {
  count: usize,
  op: Option<char>,
  op_n: usize,
  // f F t T r g waiting for their char
  pending: Option<char>,
  last_find: Option<(char, char)>,
  reg: String,
  // the register was just written by a yank: the app mirrors it to the system clipboard
  fresh: bool,
  undo: Vec<Buf>,
  redo: Vec<Buf>,
  last: Option<Edit>,
  rec: Option<Rec>,
  // keys of the command being typed
  cur: Vec<String>,
  replaying: bool,
}

fn cls(ch: char, big: bool) -> u8 {
  if ch.is_whitespace() {
    0
  } else if big || ch.is_alphanumeric() || ch == '_' {
    1
  } else {
    2
  }
}

fn word_fwd(k: &[u8], mut p: usize, n: usize) -> usize {
  for _ in 0..n {
    if p >= k.len() {
      break;
    }
    let cl = k[p];
    while p < k.len() && k[p] == cl {
      p += 1;
    }
    while p < k.len() && k[p] == 0 {
      p += 1;
    }
  }
  p
}

fn word_back(k: &[u8], mut p: usize, n: usize) -> usize {
  for _ in 0..n {
    if p == 0 {
      break;
    }
    p -= 1;
    while p > 0 && k[p] == 0 {
      p -= 1;
    }
    let cl = k[p];
    while p > 0 && k[p - 1] == cl {
      p -= 1;
    }
  }
  p
}

fn word_end(k: &[u8], mut p: usize, n: usize) -> usize {
  for _ in 0..n {
    if p + 1 >= k.len() {
      break;
    }
    p += 1;
    while p + 1 < k.len() && k[p] == 0 {
      p += 1;
    }
    let cl = k[p];
    while p + 1 < k.len() && k[p + 1] == cl {
      p += 1;
    }
  }
  p
}

// n-th `ch` from `p`; `before` stops one short (t/T). None = not there
fn find(c: &[char], p: usize, ch: char, fwd: bool, before: bool, n: usize) -> Option<usize> {
  let mut i = p;
  for _ in 0..n {
    i = if fwd { (i + 1..c.len()).find(|&j| c[j] == ch)? } else { (0..i).rev().find(|&j| c[j] == ch)? };
  }
  Some(match (before, fwd) {
    (false, _) => i,
    (true, true) => i - 1,
    (true, false) => i + 1,
  })
}

fn target(b: &Buf, key: char, n: usize, ch: Option<char>) -> Option<usize> {
  let (c, p) = (&b.c[..], b.pos);
  Some(match key {
    'h' => p.saturating_sub(n),
    'l' => (p + n).min(b.last()),
    'w' | 'W' => word_fwd(&b.classes(key == 'W'), p, n),
    'b' | 'B' => word_back(&b.classes(key == 'B'), p, n),
    'e' | 'E' => word_end(&b.classes(key == 'E'), p, n),
    '0' => 0,
    '$' => b.last(),
    '^' => c.iter().position(|x| !x.is_whitespace()).unwrap_or(c.len()),
    'f' | 'F' | 't' | 'T' => return find(c, p, ch?, matches!(key, 'f' | 't'), matches!(key, 't' | 'T'), n),
    _ => p,
  })
}

impl Vi {
  // the text of a yank since the last call, once
  pub fn take_fresh(&mut self) -> Option<String> {
    std::mem::take(&mut self.fresh).then(|| self.reg.clone())
  }

  // nothing half-typed (count, operator, f/t/r/g waiting)
  pub fn idle(&self) -> bool {
    self.count == 0 && self.op.is_none() && self.pending.is_none()
  }

  // a half-typed `g` (the app takes `gt` `gT` for tabs)
  pub fn g_pending(&self) -> bool {
    self.pending == Some('g') && self.op.is_none()
  }

  // drop the half-typed command
  pub fn abort(&mut self) {
    self.cancel();
  }

  // vim's showcmd: the half-typed command
  pub fn showcmd(&self) -> String {
    crate::ui::mode_label::showcmd(self.count, self.op.map(|o| (self.op_n, o)), self.pending)
  }

  // the line changed under us (history, send): drop undo state
  pub fn reset(&mut self) {
    self.cancel();
    self.undo.clear();
    self.redo.clear();
    self.rec = None;
  }

  // pull the cursor back onto a char (after the line was swapped for a history entry)
  pub fn settle(line: &mut Line) {
    let mut b = Buf::load(line);
    b.clamp();
    b.store(line);
  }

  // entering insert from outside (chat mode, click): one undo step per session
  pub fn begin_insert(&mut self, line: &Line) {
    let b = Buf::load(line);
    self.snap(&b);
    self.cur = vec!["i".into()];
    self.enter_insert(&b);
  }

  // esc in insert: close the session, cursor steps back onto a char (vim)
  pub fn end_insert(&mut self, line: &mut Line) {
    self.cancel();
    let mut b = Buf::load(line);
    if let Some(r) = self.rec.take() {
      let changed = match self.undo.last() {
        Some(s) => s.text() != b.text(),
        None => r.before != b.strs(),
      };
      if !changed {
        self.undo.pop();
      } else if !self.replaying {
        let (p, s) = affix(&r.before, &b.strs());
        self.last = Some(Edit { keys: r.keys, del: r.before.len() - p - s, ins: b.slice(p, b.len() - s) });
      }
    }
    b.pos = b.pos.saturating_sub(1);
    b.clamp();
    b.store(line);
  }

  pub fn key(&mut self, line: &mut Line, k: &str) -> Action {
    if !self.replaying {
      if self.idle() {
        self.cur.clear();
      }
      self.cur.push(k.to_string());
      if k == "." && self.idle() {
        self.repeat(line);
        return Action::None;
      }
    }
    let mut b = Buf::load(line);
    let a = self.step(&mut b, k);
    if a != Action::Insert {
      b.clamp();
    }
    b.store(line);
    a
  }

  fn cancel(&mut self) {
    self.count = 0;
    self.op = None;
    self.op_n = 1;
    self.pending = None;
  }

  fn take_count(&mut self) -> usize {
    std::mem::take(&mut self.count).max(1)
  }

  fn snap(&mut self, b: &Buf) {
    if self.undo.len() == UNDO_CAP {
      self.undo.remove(0);
    }
    self.undo.push(b.clone());
    self.redo.clear();
  }

  fn enter_insert(&mut self, b: &Buf) {
    let keys = if self.replaying { Vec::new() } else { std::mem::take(&mut self.cur) };
    self.rec = Some(Rec { before: b.strs(), keys });
  }

  fn commit(&mut self) {
    if !self.replaying {
      self.last = Some(Edit { keys: self.cur.clone(), del: 0, ins: String::new() });
    }
  }

  fn undo(&mut self, b: &mut Buf) {
    if let Some(s) = self.undo.pop() {
      self.redo.push(std::mem::replace(b, s));
    }
  }

  fn redo(&mut self, b: &mut Buf) {
    if let Some(s) = self.redo.pop() {
      self.undo.push(std::mem::replace(b, s));
    }
  }

  // `.`: run the last edit's keys again, then splice its typed text
  fn repeat(&mut self, line: &mut Line) {
    let Some(e) = self.last.clone() else { return };
    self.replaying = true;
    let mut inserting = false;
    for k in &e.keys {
      inserting |= self.key(line, k) == Action::Insert;
    }
    if inserting {
      let mut b = Buf::load(line);
      let end = (b.pos + e.del).min(b.len());
      let n = b.splice(b.pos, end, &e.ins);
      b.pos += n;
      b.store(line);
      self.end_insert(line);
    }
    self.replaying = false;
  }

  fn step(&mut self, b: &mut Buf, k: &str) -> Action {
    let k = match k {
      "Left" | "Backspace" => "h",
      "Right" | " " => "l",
      "Home" => "^",
      "End" => "$",
      "Up" => "k",
      "Down" => "j",
      "Delete" => "x",
      _ => k,
    };
    match k {
      "Esc" => {
        let idle = self.idle();
        self.cancel();
        return if idle { Action::Leave } else { Action::None };
      }
      "Enter" => {
        self.cancel();
        return Action::Send;
      }
      "<c-r>" => {
        for _ in 0..self.take_count() {
          self.redo(b);
        }
        self.cancel();
        return Action::None;
      }
      _ => {}
    }
    let mut cs = k.chars();
    let (Some(c), None) = (cs.next(), cs.next()) else {
      self.cancel();
      return Action::None;
    };
    if let Some(cmd) = self.pending.take() {
      return self.pend(b, cmd, c);
    }
    if c.is_ascii_digit() && (c != '0' || self.count > 0) {
      self.count = (self.count * 10 + (c as usize - '0' as usize)).min(9999);
      return Action::None;
    }
    let n = self.take_count();
    match self.op {
      Some(op) => self.with_op(b, op, c, n),
      None => self.cmd(b, c, n),
    }
  }

  // wait for a char: keep the count for it
  fn wait(&mut self, cmd: char, n: usize) -> Action {
    self.pending = Some(cmd);
    self.count = if n > 1 { n } else { 0 };
    Action::None
  }

  // f F t T r g got their char
  fn pend(&mut self, b: &mut Buf, cmd: char, c: char) -> Action {
    let n0 = self.take_count();
    let n = if self.op.is_some() { n0 * self.op_n.max(1) } else { n0 };
    match cmd {
      'g' if c == 'g' => self.motion(b, '0', n, None),
      'r' => {
        if b.pos + n <= b.len() {
          self.snap(b);
          b.splice(b.pos, b.pos + n, &c.to_string().repeat(n));
          b.pos += n - 1;
          self.commit();
        }
        Action::None
      }
      'f' | 'F' | 't' | 'T' => {
        self.last_find = Some((cmd, c));
        self.motion(b, cmd, n, Some(c))
      }
      _ => {
        self.cancel();
        Action::None
      }
    }
  }

  fn with_op(&mut self, b: &mut Buf, op: char, c: char, n0: usize) -> Action {
    let n = n0 * self.op_n.max(1);
    if c == op {
      self.cancel();
      return self.whole_line(b, op);
    }
    match c {
      'h' | 'l' | 'w' | 'b' | 'e' | 'W' | 'B' | 'E' | '0' | '$' | '^' => self.motion(b, c, n, None),
      'G' => self.motion(b, '$', n, None),
      'f' | 'F' | 't' | 'T' | 'g' => self.wait(c, n0),
      _ => {
        self.cancel();
        Action::None
      }
    }
  }

  // dd cc yy
  fn whole_line(&mut self, b: &mut Buf, op: char) -> Action {
    self.reg = b.slice(0, b.len());
    self.fresh = op == 'y';
    if op == 'y' {
      return Action::None;
    }
    self.snap(b);
    b.c.clear();
    b.pos = 0;
    if op == 'c' {
      self.enter_insert(b);
      return Action::Insert;
    }
    self.commit();
    Action::None
  }

  // a motion: moves the cursor, or is the range of a pending operator
  fn motion(&mut self, b: &mut Buf, key: char, n: usize, ch: Option<char>) -> Action {
    let Some(op) = self.op else {
      if let Some(t) = target(b, key, n, ch) {
        b.pos = t;
      }
      return Action::None;
    };
    self.cancel();
    let (p, len) = (b.pos, b.len());
    let t = target(b, key, n, ch);
    let range = match key {
      'l' => Some((p, (p + n).min(len))),
      'h' => Some((p.saturating_sub(n), p)),
      // cw on a word acts like ce (vim)
      'w' | 'W' if op == 'c' && p < len && b.classes(key == 'W')[p] != 0 => {
        let k = b.classes(key == 'W');
        let mut q = p;
        while q + 1 < len && k[q + 1] == k[p] {
          q += 1;
        }
        Some((p, word_end(&k, q, n - 1) + 1))
      }
      'e' | 'E' | '$' | 'f' | 't' => t.map(|t| (p, (t + 1).min(len))),
      _ => t.map(|t| if t >= p { (p, t) } else { (t, p) }),
    };
    let Some((s, e)) = range.filter(|r| r.0 < r.1) else { return Action::None };
    self.reg = b.slice(s, e);
    self.fresh = op == 'y';
    match op {
      'y' => b.pos = s,
      _ => {
        self.snap(b);
        b.splice(s, e, "");
        b.pos = s;
        if op == 'c' {
          self.enter_insert(b);
          return Action::Insert;
        }
        self.commit();
      }
    }
    Action::None
  }

  fn cmd(&mut self, b: &mut Buf, c: char, n: usize) -> Action {
    let (p, len) = (b.pos, b.len());
    match c {
      'i' | 'a' | 'I' | 'A' => {
        self.snap(b);
        b.pos = match c {
          'a' => (p + 1).min(len),
          'I' => 0,
          'A' => len,
          _ => p,
        };
        self.enter_insert(b);
        return Action::Insert;
      }
      's' | 'S' | 'C' => {
        self.snap(b);
        let (a, e) = match c {
          's' => (p, (p + n).min(len)),
          'S' => (0, len),
          _ => (p, len),
        };
        if a < e {
          self.reg = b.slice(a, e);
          b.splice(a, e, "");
        }
        b.pos = a.min(b.len());
        self.enter_insert(b);
        return Action::Insert;
      }
      'h' | 'l' | 'w' | 'b' | 'e' | 'W' | 'B' | 'E' | '0' | '$' | '^' => return self.motion(b, c, n, None),
      'G' => b.pos = b.last(),
      'f' | 'F' | 't' | 'T' | 'g' | 'r' => return self.wait(c, n),
      ';' | ',' => self.repeat_find(b, c == ',', n),
      'd' | 'c' | 'y' => {
        self.op = Some(c);
        self.op_n = n;
      }
      'x' | 'X' | 'D' => {
        let (a, e) = match c {
          'x' => (p, (p + n).min(len)),
          'X' => (p.saturating_sub(n), p),
          _ => (p, len),
        };
        if a < e {
          self.snap(b);
          self.reg = b.slice(a, e);
          b.splice(a, e, "");
          b.pos = a;
          self.commit();
        }
      }
      '~' if p < len => {
        self.snap(b);
        let e = (p + n).min(len);
        for i in p..e {
          b.c[i] = toggle(b.c[i]);
        }
        b.pos = e;
        self.commit();
      }
      'p' | 'P' if !self.reg.is_empty() => {
        self.snap(b);
        let at = if c == 'p' { (p + 1).min(len) } else { p };
        let put = self.reg.repeat(n);
        let m = b.splice(at, at, &put);
        b.pos = (at + m).saturating_sub(1);
        self.commit();
      }
      'u' => {
        for _ in 0..n {
          self.undo(b);
        }
      }
      'j' => return Action::HistoryNewer,
      'k' => return Action::HistoryOlder,
      _ => {}
    }
    Action::None
  }

  // ; and , (reversed). a repeated t/T skips the char it already sits next to
  fn repeat_find(&mut self, b: &mut Buf, rev: bool, n: usize) {
    let Some((cmd, ch)) = self.last_find else { return };
    let cmd = match (cmd, rev) {
      (c, false) => c,
      ('f', true) => 'F',
      ('F', true) => 'f',
      ('t', true) => 'T',
      (_, true) => 't',
    };
    let from = match cmd {
      't' if b.pos + 1 < b.len() => b.pos + 1,
      'T' if b.pos > 0 => b.pos - 1,
      't' | 'T' => return,
      _ => b.pos,
    };
    if let Some(t) = find(&b.c, from, ch, matches!(cmd, 'f' | 't'), matches!(cmd, 't' | 'T'), n) {
      b.pos = t;
    }
  }
}

// common prefix and suffix lengths of two texts (the part insert mode changed is between)
fn affix(a: &[String], b: &[String]) -> (usize, usize) {
  let p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
  let s = a[p..].iter().rev().zip(b[p..].iter().rev()).take_while(|(x, y)| x == y).count();
  (p, s)
}

// swap case, keeping the char count (ß and friends stay put)
fn toggle(c: char) -> char {
  if c.is_lowercase() {
    let mut u = c.to_uppercase();
    match (u.next(), u.next()) {
      (Some(x), None) => x,
      _ => c,
    }
  } else if c.is_uppercase() {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
      (Some(x), None) => x,
      _ => c,
    }
  } else {
    c
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  struct Sim {
    l: Line,
    v: Vi,
    ins: bool,
    acts: Vec<Action>,
  }

  // keys as chars; <esc> <cr> <c-r> as tokens
  fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
      let tok = ["<esc>", "<cr>", "<c-r>"].into_iter().find(|t| rest.starts_with(t));
      let n = tok.map_or(c.len_utf8(), str::len);
      out.push(rest[..n].to_string());
      rest = &rest[n..];
    }
    out
  }

  impl Sim {
    // normal mode, cursor on char index `at`
    fn new(text: &str, at: usize) -> Sim {
      let mut l = Line::default();
      l.insert(text);
      l.cursor = text.char_indices().nth(at).map_or(text.len(), |(i, _)| i);
      Sim { l, v: Vi::default(), ins: false, acts: Vec::new() }
    }

    fn keys(&mut self, s: &str) -> &mut Sim {
      for k in tokens(s) {
        if self.ins {
          if k == "<esc>" {
            self.v.end_insert(&mut self.l);
            self.ins = false;
          } else {
            self.l.insert(&k);
          }
          continue;
        }
        let name = match k.as_str() {
          "<esc>" => "Esc",
          "<cr>" => "Enter",
          k => k,
        };
        let a = self.v.key(&mut self.l, name);
        self.ins = a == Action::Insert;
        if a != Action::None {
          self.acts.push(a);
        }
      }
      self
    }

    fn text(&self) -> &str {
      &self.l.text
    }

    // cursor as a char index
    fn at(&self) -> usize {
      self.l.text[..self.l.cursor].chars().count()
    }
  }

  #[test]
  fn motions() {
    let mut s = Sim::new("one two  three", 0);
    s.keys("w");
    assert_eq!(s.at(), 4);
    s.keys("w");
    assert_eq!(s.at(), 9);
    s.keys("w");
    assert_eq!(s.at(), 13, "w at the last word stops on the last char");
    s.keys("b");
    assert_eq!(s.at(), 9);
    s.keys("2b");
    assert_eq!(s.at(), 0);
    s.keys("e");
    assert_eq!(s.at(), 2);
    s.keys("$");
    assert_eq!(s.at(), 13);
    s.keys("0");
    assert_eq!(s.at(), 0);
    s.keys("3l");
    assert_eq!(s.at(), 3);
    s.keys("9h");
    assert_eq!(s.at(), 0);
    s.keys("G");
    assert_eq!(s.at(), 13);
    s.keys("gg");
    assert_eq!(s.at(), 0);
  }

  #[test]
  fn big_words_and_caret() {
    let mut s = Sim::new("  foo.bar baz", 0);
    s.keys("^");
    assert_eq!(s.at(), 2);
    s.keys("w");
    assert_eq!(s.at(), 5, "w stops at punctuation");
    s.keys("0W");
    assert_eq!(s.at(), 2);
    s.keys("W");
    assert_eq!(s.at(), 10);
    s.keys("B");
    assert_eq!(s.at(), 2);
    s.keys("E");
    assert_eq!(s.at(), 8);
  }

  #[test]
  fn find_and_repeat() {
    let mut s = Sim::new("a,b,c,d", 0);
    s.keys("f,");
    assert_eq!(s.at(), 1);
    s.keys(";");
    assert_eq!(s.at(), 3);
    s.keys(",");
    assert_eq!(s.at(), 1);
    s.keys("$F,");
    assert_eq!(s.at(), 5);
    s.keys("0t,");
    assert_eq!(s.at(), 0, "t next to the char stays put");
    s.keys(";");
    assert_eq!(s.at(), 2, "; skips the adjacent match");
    s.keys("0fz");
    assert_eq!(s.at(), 0, "missing char: no move");
    s.keys("2f,");
    assert_eq!(s.at(), 3);
  }

  #[test]
  fn operators_with_counts() {
    let mut s = Sim::new("one two three four", 0);
    s.keys("d2w");
    assert_eq!(s.text(), "three four");
    s.keys("2dw");
    assert_eq!(s.text(), "");
    let mut s = Sim::new("abcdef", 0);
    s.keys("3x");
    assert_eq!(s.text(), "def");
    assert_eq!(s.v.reg, "abc");
    s.keys("$X");
    assert_eq!(s.text(), "df");
    let mut s = Sim::new("ab, cd", 0);
    s.keys("ct,X<esc>");
    assert_eq!(s.text(), "X, cd");
    let mut s = Sim::new("foo bar baz", 4);
    s.keys("cwX<esc>");
    assert_eq!(s.text(), "foo X baz");
    let mut s = Sim::new("foo bar", 6);
    s.keys("cwX<esc>");
    assert_eq!(s.text(), "foo baX", "cw on the last char changes just it");
    let mut s = Sim::new("foo bar baz", 0);
    s.keys("c2wX<esc>");
    assert_eq!(s.text(), "X baz");
    let mut s = Sim::new("foo bar", 4);
    s.keys("db");
    assert_eq!(s.text(), "bar");
    let mut s = Sim::new("foo bar", 4);
    s.keys("d0");
    assert_eq!(s.text(), "bar");
    let mut s = Sim::new("foo bar", 4);
    s.keys("d$");
    assert_eq!(s.text(), "foo ");
    assert_eq!(s.at(), 3);
    let mut s = Sim::new("foo bar", 0);
    s.keys("dl");
    assert_eq!(s.text(), "oo bar");
    let mut s = Sim::new("a,b,c", 0);
    s.keys("df,");
    assert_eq!(s.text(), "b,c");
    let mut s = Sim::new("foo bar", 6);
    s.keys("dgg");
    assert_eq!(s.text(), "r");
  }

  #[test]
  fn line_ops() {
    let mut s = Sim::new("hello world", 3);
    s.keys("dd");
    assert_eq!(s.text(), "");
    s.keys("p");
    assert_eq!(s.text(), "hello world");
    let mut s = Sim::new("hello world", 3);
    s.keys("ccbye<esc>");
    assert_eq!(s.text(), "bye");
    let mut s = Sim::new("hello world", 6);
    s.keys("D");
    assert_eq!(s.text(), "hello ");
    s.keys("Cx<esc>");
    assert_eq!(s.text(), "hellox", "D leaves the cursor on the space, C eats it");
    s.keys("Sfresh<esc>");
    assert_eq!(s.text(), "fresh");
    s.keys("0sZ<esc>");
    assert_eq!(s.text(), "Zresh");
  }

  #[test]
  fn only_yanks_are_fresh() {
    let mut s = Sim::new("foo bar", 0);
    s.keys("yw");
    assert_eq!(s.v.take_fresh().as_deref(), Some("foo "));
    assert_eq!(s.v.take_fresh(), None, "once");
    s.keys("dw");
    assert_eq!(s.v.take_fresh(), None, "deletes stay off the system clipboard");
    s.keys("x");
    assert_eq!(s.v.take_fresh(), None);
  }

  #[test]
  fn yank_and_put() {
    let mut s = Sim::new("foo bar", 0);
    s.keys("yw$p");
    assert_eq!(s.text(), "foo barfoo ");
    let mut s = Sim::new("ab", 0);
    s.keys("yl2P");
    assert_eq!(s.text(), "aaab");
    assert_eq!(s.at(), 1);
    let mut s = Sim::new("ab", 1);
    s.keys("yyP");
    assert_eq!(s.text(), "aabb");
    let mut s = Sim::new("ab cd", 3);
    s.keys("yb");
    assert_eq!(s.at(), 0, "yank backward lands at the start");
  }

  #[test]
  fn replace_and_case() {
    let mut s = Sim::new("hello", 0);
    s.keys("rj");
    assert_eq!(s.text(), "jello");
    s.keys("2rx");
    assert_eq!(s.text(), "xxllo");
    assert_eq!(s.at(), 1);
    s.keys("9ry");
    assert_eq!(s.text(), "xxllo", "not enough chars: no-op");
    s.keys("0~");
    assert_eq!(s.text(), "Xxllo");
    assert_eq!(s.at(), 1);
    s.keys("3~");
    assert_eq!(s.text(), "XXLLo");
  }

  #[test]
  fn insert_entries() {
    let mut s = Sim::new("abc", 1);
    s.keys("iX<esc>");
    assert_eq!(s.text(), "aXbc");
    assert_eq!(s.at(), 1, "esc steps back onto the typed char");
    let mut s = Sim::new("abc", 1);
    s.keys("aX<esc>");
    assert_eq!(s.text(), "abXc");
    let mut s = Sim::new("abc", 1);
    s.keys("AX<esc>");
    assert_eq!(s.text(), "abcX");
    assert_eq!(s.at(), 3);
    s.keys("IY<esc>");
    assert_eq!(s.text(), "YabcX");
    assert_eq!(s.at(), 0);
  }

  #[test]
  fn dot_repeat() {
    let mut s = Sim::new("a b c d", 0);
    s.keys("dw.");
    assert_eq!(s.text(), "c d");
    s.keys(".");
    assert_eq!(s.text(), "d");
    let mut s = Sim::new("foo bar", 0);
    s.keys("cwX<esc>w.");
    assert_eq!(s.text(), "X X");
    let mut s = Sim::new("ab", 0);
    s.keys("A!<esc>.");
    assert_eq!(s.text(), "ab!!");
    let mut s = Sim::new("abcdef", 0);
    s.keys("2x.");
    assert_eq!(s.text(), "ef");
    let mut s = Sim::new("a,b,c,d", 0);
    s.keys("dt,.");
    assert_eq!(s.text(), ",c,d");
    let mut s = Sim::new("hello", 0);
    s.keys("rxl.");
    assert_eq!(s.text(), "xxllo");
    // nothing to repeat is a no-op
    let mut s = Sim::new("abc", 0);
    s.keys(".");
    assert_eq!(s.text(), "abc");
    // . undoes as one step
    let mut s = Sim::new("foo bar", 0);
    s.keys("cwX<esc>w.u");
    assert_eq!(s.text(), "X bar");
  }

  #[test]
  fn undo_redo() {
    let mut s = Sim::new("abc", 0);
    s.keys("xx");
    assert_eq!(s.text(), "c");
    s.keys("u");
    assert_eq!(s.text(), "bc");
    s.keys("u");
    assert_eq!(s.text(), "abc");
    s.keys("u");
    assert_eq!(s.text(), "abc");
    s.keys("<c-r>");
    assert_eq!(s.text(), "bc");
    s.keys("<c-r>");
    assert_eq!(s.text(), "c");
    s.keys("2u");
    assert_eq!(s.text(), "abc");
    // a new edit drops redo
    s.keys("x<c-r>");
    assert_eq!(s.text(), "bc");
    // a whole insert session is one undo step
    let mut s = Sim::new("abc", 0);
    s.keys("ihello <esc>");
    assert_eq!(s.text(), "hello abc");
    s.keys("u");
    assert_eq!(s.text(), "abc");
    // an empty insert session leaves no undo step
    s.keys("i<esc>x");
    assert_eq!(s.text(), "bc");
    s.keys("u");
    assert_eq!(s.text(), "abc");
    // cw + typing undoes whole
    let mut s = Sim::new("foo bar", 0);
    s.keys("cwxyz<esc>");
    assert_eq!(s.text(), "xyz bar");
    s.keys("u");
    assert_eq!(s.text(), "foo bar");
    assert_eq!(s.at(), 0);
  }

  #[test]
  fn undo_is_capped() {
    let mut s = Sim::new(&"a".repeat(300), 0);
    for _ in 0..250 {
      s.keys("x");
    }
    assert_eq!(s.v.undo.len(), UNDO_CAP);
  }

  #[test]
  fn cursor_stays_on_a_char() {
    let mut s = Sim::new("abc", 0);
    s.keys("$");
    assert_eq!(s.at(), 2);
    s.keys("x");
    assert_eq!((s.text(), s.at()), ("ab", 1));
    s.keys("l");
    assert_eq!(s.at(), 1);
    let mut s = Sim::new("", 0);
    s.keys("xXDhl0$w");
    assert_eq!((s.text(), s.at()), ("", 0));
    s.keys("pP");
    assert_eq!(s.text(), "");
    // esc out of insert steps back; at 0 stays
    let mut s = Sim::new("", 0);
    s.keys("ia<esc>");
    assert_eq!((s.text(), s.at()), ("a", 0));
  }

  #[test]
  fn utf8_is_safe() {
    let mut s = Sim::new("日本語 😀 éa", 0);
    s.keys("l");
    assert_eq!(s.at(), 1);
    s.keys("x");
    assert_eq!(s.text(), "日語 😀 éa");
    s.keys("w");
    assert_eq!(s.at(), 3);
    s.keys("x");
    assert_eq!(s.text(), "日語  éa");
    s.keys("$X");
    assert_eq!(s.text(), "日語  a");
    s.keys("0rあ");
    assert_eq!(s.text(), "あ語  a");
    s.keys("f語");
    assert_eq!(s.at(), 1);
    s.keys("d$");
    assert_eq!(s.text(), "あ");
    s.keys("A😀é<esc>");
    assert_eq!(s.text(), "あ😀é");
    assert_eq!(s.at(), 2);
    s.keys("yh$p");
    assert_eq!(s.text(), "あ😀é😀");
    s.keys("~");
    assert_eq!(s.text(), "あ😀é😀");
    s.keys("0lcwx<esc>");
    assert_eq!(s.text(), "あxé😀", "emoji is punctuation to cw");
    s.keys("u");
    assert_eq!(s.text(), "あ😀é😀");
    let mut s = Sim::new("éß", 0);
    s.keys("2~");
    assert_eq!(s.text(), "Éß", "ß has no one-char upper: left alone");
  }

  #[test]
  fn emoji_is_one_step() {
    // skin tone, flag, zwj family: each is several chars but one emoji
    let mut s = Sim::new("a👍🏽🇯🇵👨‍👩‍👧🌇b", 0);
    s.keys("l");
    assert_eq!(s.l.cursor, 1);
    s.keys("l");
    assert_eq!(s.l.cursor, "a👍🏽".len());
    s.keys("x");
    assert_eq!(s.text(), "a👍🏽👨‍👩‍👧🌇b");
    s.keys("2x");
    assert_eq!(s.text(), "a👍🏽b");
    s.keys("hyl$p");
    assert_eq!(s.text(), "a👍🏽b👍🏽");
    assert_eq!(s.l.cursor, "a👍🏽b".len(), "the block sits on the pasted emoji");
    s.keys("X");
    assert_eq!(s.text(), "a👍🏽👍🏽");
    s.keys("u");
    assert_eq!(s.text(), "a👍🏽b👍🏽");
  }

  // the line "pepezen frozen0 hi" with frozen0 an overlay on pepezen: one stack
  fn stacked(at: usize) -> Sim {
    let mut s = Sim::new("x pepezen frozen0 hi", at);
    s.l.chips.extend(["pepezen".into(), "frozen0".into()]);
    s.l.overlays.insert("frozen0".into());
    s
  }

  #[test]
  fn emote_stack_is_one_unit() {
    // from the h of "hi", hh lands on the whole stack, x takes it all
    let mut s = stacked(18);
    s.keys("hh");
    assert_eq!(s.l.cursor, 2, "the cursor rests on the stack's start");
    s.keys("x");
    assert_eq!(s.text(), "x  hi");
    s.keys("u");
    assert_eq!(s.text(), "x pepezen frozen0 hi");
    // l steps over it, w treats it as its own word, dw/yank/paste carry it whole
    let mut s = stacked(2);
    s.keys("l");
    assert_eq!(s.l.cursor, 17);
    s.keys("0w");
    assert_eq!(s.l.cursor, 2);
    s.keys("w");
    assert_eq!(s.l.cursor, 18);
    s.keys("bdw");
    assert_eq!(s.text(), "x hi");
    s.keys("P");
    assert_eq!(s.text(), "x pepezen frozen0 hi");
    // a cursor that landed inside the stack (a click, history) snaps to its start
    let mut s = stacked(12);
    s.keys("x");
    assert_eq!(s.text(), "x  hi");
    // not an overlay: two emotes, two steps
    let mut s = stacked(18);
    s.l.overlays.clear();
    s.keys("hhx");
    assert_eq!(s.text(), "x pepezen  hi");
  }

  #[test]
  fn caps_and_sanitizes_pastes() {
    let mut s = Sim::new(&"x".repeat(MAX_CHARS - 2), 0);
    s.keys("yy$p");
    assert_eq!(s.text().chars().count(), MAX_CHARS);
    let mut s = Sim::new("ab", 0);
    s.v.reg = "\n\u{7}".into();
    s.keys("p");
    assert_eq!(s.text(), "a  b");
  }

  #[test]
  fn actions() {
    let mut s = Sim::new("abc", 0);
    s.keys("k");
    s.keys("j");
    s.keys("<cr>");
    s.keys("<esc>");
    assert_eq!(s.acts, [Action::HistoryOlder, Action::HistoryNewer, Action::Send, Action::Leave]);
    // esc only leaves when nothing is half-typed
    let mut s = Sim::new("abc", 0);
    s.keys("2d<esc>");
    assert!(s.acts.is_empty());
    assert!(s.v.idle());
    s.keys("<esc>");
    assert_eq!(s.acts, [Action::Leave]);
    s.keys("f<esc>");
    assert!(s.v.idle());
    // a cancelled operator does not eat the next command
    s.keys("dzx");
    assert_eq!(s.text(), "bc");
  }

  #[test]
  fn history_swap_settles_the_cursor() {
    let mut l = Line::default();
    l.insert("héllo");
    assert_eq!(l.cursor, "héllo".len());
    Vi::settle(&mut l);
    assert_eq!(&l.text[l.cursor..], "o");
  }
}
