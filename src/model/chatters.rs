// who is a "person" inside message text (ext main.js highlightMentionsInHtml): `@name` is always a
// mention (its color comes from the chatter, or from a lookup while it is white); a bare `name` only
// when that someone has actually spoken, and the word is not an everyday one ("bruh", "what").
// one table for the whole window, per platform, so a twitch and a kick chatter sharing a name never
// trade colors or paints. bounded; evicted names are simply learned again from their next line
use std::collections::{HashMap, HashSet};
use std::ops::Range;

use super::message::{Kind, Message, Platform};
use crate::cosmetics::id::PaintId;
use crate::ui::theme;

// names tracked per platform before the table sheds some
const CAP: usize = 5000;
// unknown @names waiting for a lookup, and names already asked (so a miss is not asked again)
const WANT_MAX: usize = 32;
const ASKED_MAX: usize = 4096;
// the ext's name regex: 3-25 of [A-Za-z0-9_]
const NAME_MIN: usize = 3;
const NAME_MAX: usize = 25;

// everyday words that are never a bare mention, even when someone with that login speaks (ext
// BARE_MENTION_STOPWORDS). `@word` is unaffected. contraction stems are here because an apostrophe
// ends a name ("don't" offers "don")
const STOP: &str = "the and you are for not but all any can has had her him his how its new now old see two way who did \
let put say she too use man own why yes yet off got run set try war win act add age air bad big end few \
that was with they this have from one were when what where whats which who whom while your yours yourself said there \
their theirs them these those some such here huh hmm oof will other about out many then would make like into more \
could than been being both people may down get come made over only just know take well very want because give most \
also back after work first even need much right think thing things does don isn wasn didn doesn couldn wouldn \
shouldn won ain dont cant wont thats hes shes youre theyre youve ive lets gonna wanna gotta kinda sorta before between \
during through under until against having each \
lol lmao lmfao omg wtf wth ngl tbh imo imho fyi idk ikr brb gtg yeah yea yep yup nah nope sure same true real fake \
bruh bro dude guys chat mods mod stream streamer game good nice best worst cool damn holy actually literally basically \
honestly seriously bot bots based cringe insane crazy sick wild huge small pog poggers kek lul ggs wow hey hello \
thanks thank please sorry congrats welcome night morning today tomorrow yesterday time year years day days week hour \
min sec guy girl kid dog cat food life love hate live dead lose lost play played playing watch watching look looks \
looking seen saw tell told talk ask asked keep stop start help wait coming going went gone still again never always \
ever maybe probably definitely absolutely";

fn stop(lower: &str) -> bool {
  STOP.split_ascii_whitespace().any(|w| w == lower)
}

// ends a name (the ext's lookahead set); whitespace never reaches here, words are split on spaces
fn edge(b: u8) -> bool {
  matches!(b, b'.' | b',' | b'!' | b'?' | b';' | b':' | b'(' | b')' | b'[' | b']' | b'"' | b'\'')
}

fn name_byte(b: u8) -> bool {
  b.is_ascii_alphanumeric() || b == b'_'
}

// one name inside a word: the whole of it (with its `@`), whether it had one, and the name alone
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
  pub full: Range<usize>,
  pub at: bool,
  pub name: Range<usize>,
}

// the names in one space-free word. punctuation around a name is left out of it ("(mellen)",
// "mellen,", "hey,@mellen"); a run with any other character in it ("mel-len", "a@b") is no name
pub fn names(word: &str) -> Vec<Found> {
  let b = word.as_bytes();
  let mut out = Vec::new();
  let mut i = 0;
  while i < b.len() {
    if edge(b[i]) {
      i += 1;
      continue;
    }
    let start = i;
    while i < b.len() && !edge(b[i]) {
      i += 1;
    }
    let at = b[start] == b'@';
    let name = start + at as usize..i;
    if (NAME_MIN..=NAME_MAX).contains(&name.len()) && b[name.clone()].iter().all(|&c| name_byte(c)) {
      out.push(Found { full: start..i, at, name });
    }
  }
  out
}

// what a name in text looks like: its color and, when we know whose it is, the paint id
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
  pub color: u32,
  pub paint: Option<PaintId>,
}

#[derive(Clone)]
struct Chatter {
  color: u32,
  paint: Option<PaintId>,
  // heard speaking (a bare name may only match these)
  spoke: bool,
}

#[derive(Default)]
pub struct Chatters {
  by: [HashMap<Box<str>, Chatter>; 3],
  want: Vec<(Platform, Box<str>)>,
  asked: [HashSet<Box<str>>; 3],
}

fn slot(p: Platform) -> usize {
  match p {
    Platform::Twitch => 0,
    Platform::Kick => 1,
    Platform::Youtube => 2,
  }
}

// ascii-lowercased into a stack buffer: a name is 3-25 name bytes, so no heap on the hot path
fn lower<'a>(name: &str, buf: &'a mut [u8; NAME_MAX]) -> Option<&'a str> {
  let n = name.len();
  if n == 0 || n > NAME_MAX {
    return None;
  }
  buf[..n].copy_from_slice(name.as_bytes());
  buf[..n].make_ascii_lowercase();
  std::str::from_utf8(&buf[..n]).ok()
}

impl Chatters {
  // a chat line was heard: its sender is a known person now, in their own color
  pub fn heard(&mut self, m: &Message) {
    if m.kind == Kind::System || m.login.is_empty() || m.login.len() > NAME_MAX {
      return;
    }
    let paint = PaintId::of_message(m);
    let map = &mut self.by[slot(m.platform)];
    let key: &str = &m.login;
    if let Some(c) = map.get_mut(key) {
      c.color = m.color;
      c.spoke = true;
      if paint.is_some() {
        c.paint = paint;
      }
      return;
    }
    if map.len() >= CAP {
      shed(map);
    }
    let key =
      if key.bytes().any(|b| b.is_ascii_uppercase()) { key.to_ascii_lowercase().into() } else { m.login.clone() };
    map.insert(key, Chatter { color: m.color, paint, spoke: true });
  }

  // a lookup answered: the color heatsync has for them, and the twitch id their paints hang off.
  // someone already heard keeps the color their own lines carry (the ext never lets a pick or a hash
  // overwrite a real twitch color)
  pub fn learned(&mut self, p: Platform, login: &str, color: u32, paint: Option<PaintId>) {
    let mut buf = [0u8; NAME_MAX];
    let Some(l) = lower(login, &mut buf) else { return };
    let map = &mut self.by[slot(p)];
    if let Some(c) = map.get_mut(l) {
      if !c.spoke {
        c.color = color;
      }
      if c.paint.is_none() {
        c.paint = paint;
      }
      return;
    }
    if map.len() >= CAP {
      shed(map);
    }
    map.insert(l.into(), Chatter { color, paint, spoke: false });
  }

  // the person a name in text means, or None when it is plain text. an unknown `@name` is white
  // until its lookup lands (and asks for one)
  pub fn person(&mut self, p: Platform, name: &str, at: bool) -> Option<Person> {
    let mut buf = [0u8; NAME_MAX];
    let l = lower(name, &mut buf)?;
    let k = slot(p);
    match self.by[k].get(l) {
      Some(c) if at || (c.spoke && !stop(l)) => Some(Person { color: c.color, paint: c.paint.clone() }),
      Some(_) => None,
      None if at => {
        if self.want.len() < WANT_MAX && !self.asked[k].contains(l) && !self.want.iter().any(|w| w.0 == p && *w.1 == *l)
        {
          self.want.push((p, l.into()));
        }
        Some(Person { color: theme::WHITE, paint: None })
      }
      None => None,
    }
  }

  // names to look up now (each is asked once until the table forgets)
  pub fn take_wanted(&mut self) -> Vec<(Platform, Box<str>)> {
    let w = std::mem::take(&mut self.want);
    for (p, l) in &w {
      let a = &mut self.asked[slot(*p)];
      if a.len() >= ASKED_MAX {
        a.clear();
      }
      a.insert(l.clone());
    }
    w
  }
}

// drop a quarter of a full table (arbitrary ones): they come back with their next line
fn shed(map: &mut HashMap<Box<str>, Chatter>) {
  let mut n = 0usize;
  map.retain(|_, _| {
    n += 1;
    !n.is_multiple_of(4)
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  fn msg(p: Platform, login: &str, color: u32, uid: &str) -> Message {
    let mut m = Message::system(0, p, String::new());
    m.kind = Kind::Chat;
    m.login = login.into();
    m.user_id = uid.into();
    m.color = color;
    m
  }

  fn spans(w: &str) -> Vec<(&str, bool)> {
    names(w).into_iter().map(|f| (&w[f.full], f.at)).collect()
  }

  #[test]
  fn names_are_found_with_their_punctuation_left_out() {
    assert_eq!(spans("mellen"), [("mellen", false)]);
    assert_eq!(spans("@mellen"), [("@mellen", true)]);
    assert_eq!(spans("(mellen)"), [("mellen", false)]);
    assert_eq!(spans("mellen,"), [("mellen", false)]);
    assert_eq!(spans("hey,@mellen!"), [("hey", false), ("@mellen", true)]);
    assert_eq!(spans("mellen's"), [("mellen", false)]);
    assert_eq!(spans("a.b.cde"), [("cde", false)]);
  }

  #[test]
  fn what_is_not_a_name_is_skipped() {
    assert!(names("hi").is_empty(), "too short");
    assert!(names("@hi").is_empty());
    assert!(names(&"a".repeat(26)).is_empty(), "too long");
    assert_eq!(names(&"a".repeat(25)).len(), 1);
    assert!(names("mel-len").is_empty(), "a dash is no name byte");
    assert!(names("a@bcd").is_empty());
    assert!(names("@@mellen").is_empty());
    assert!(names("mellén").is_empty(), "ascii names only");
    assert!(names("").is_empty() && names("...").is_empty());
  }

  #[test]
  fn the_name_range_skips_the_at() {
    let f = &names("xyz,@Bob_1")[1];
    assert_eq!((f.full.clone(), f.name.clone()), (4..10, 5..10));
  }

  #[test]
  fn at_always_counts_bare_needs_a_speaker() {
    let mut c = Chatters::default();
    c.heard(&msg(Platform::Twitch, "alice", 0x00ff00, "11"));
    // heard: both forms, any case, in their color and with their paint id
    for (n, at) in [("alice", false), ("Alice", false), ("ALICE", true)] {
      let p = c.person(Platform::Twitch, n, at).unwrap();
      assert_eq!((p.color, p.paint), (0x00ff00, Some(PaintId::Twitch(11))));
    }
    // not a chatter: a bare word is text, `@word` is white and asks
    assert_eq!(c.person(Platform::Twitch, "carol", false), None);
    assert_eq!(c.person(Platform::Twitch, "carol", true).unwrap().color, theme::WHITE);
    assert_eq!(c.take_wanted(), [(Platform::Twitch, "carol".into())]);
    // asked once, not again
    c.person(Platform::Twitch, "carol", true);
    assert!(c.take_wanted().is_empty());
  }

  #[test]
  fn everyday_words_stay_text_even_when_someone_has_that_name() {
    let mut c = Chatters::default();
    c.heard(&msg(Platform::Twitch, "bruh", 0xff0000, "5"));
    assert_eq!(c.person(Platform::Twitch, "bruh", false), None);
    assert!(c.person(Platform::Twitch, "bruh", true).is_some(), "@bruh still names them");
  }

  #[test]
  fn platforms_do_not_share_people() {
    let mut c = Chatters::default();
    c.heard(&msg(Platform::Kick, "dave", 0x00ffff, "9"));
    assert_eq!(c.person(Platform::Twitch, "dave", false), None);
    assert_eq!(c.person(Platform::Kick, "dave", false).unwrap().paint, Some(PaintId::Kick(9)));
  }

  #[test]
  fn a_lookup_fills_strangers_but_never_recolors_a_speaker() {
    let mut c = Chatters::default();
    c.learned(Platform::Twitch, "erin", 0xff00ff, Some(PaintId::Twitch(7)));
    // learned, not heard: `@` yes, bare no
    assert_eq!(c.person(Platform::Twitch, "erin", false), None);
    assert_eq!(c.person(Platform::Twitch, "erin", true).unwrap().color, 0xff00ff);
    c.heard(&msg(Platform::Twitch, "erin", 0x00ff00, "7"));
    assert_eq!(c.person(Platform::Twitch, "erin", false).unwrap().color, 0x00ff00);
    c.learned(Platform::Twitch, "erin", 0xff00ff, None);
    assert_eq!(c.person(Platform::Twitch, "erin", true).unwrap().color, 0x00ff00);
  }

  #[test]
  fn the_table_and_the_queue_stay_bounded() {
    let mut c = Chatters::default();
    for i in 0..CAP + 100 {
      c.heard(&msg(Platform::Twitch, &format!("user{i}"), 0xffffff, ""));
    }
    assert!(c.by[0].len() <= CAP);
    for i in 0..WANT_MAX + 20 {
      c.person(Platform::Twitch, &format!("zzz{i}"), true);
    }
    assert_eq!(c.take_wanted().len(), WANT_MAX);
  }
}
