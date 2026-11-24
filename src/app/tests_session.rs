// the whole app headless, a seeded random session in the box: names typed by hand, tab stepped onto a copy
// of a name (forward, round again, back), a picture picked on purpose, a word deleted and put back (keys,
// a paste), the tab switched in the middle of a draft, a sent line brought back with the up arrow and sent again. on every message the box, tab's list,
// what went out (the words, the vouches, the swap or add of your own emote) and the row that comes back are
// checked against a small model of the emote law written here from the law's words, never from the app's
// code:
//   a typed word is the open tab's room's emote, then yours, then a global
//   a pick is its message's: the last pick of a name answers every copy of that name in the line
//   a pick outlives a tab switch; a typed word is read again where it is sent
//   a pick is over when its word leaves the box: the name entered again is a typed word
//   a recalled line keeps only the picks that changed what their word sent from where it was sent
// the same hunt runs in a real browser on the site (emote-pick-random-session.spec.ts) and the extension
// (render-emote-collisions.ts, case y). HS_SESSION_SEEDS=5,6,7 hunts other seeds

use super::*;
use crate::emotes::{Emote, Provider, Set};
use crate::model::message::{Kind as MsgKind, Message, flag};
use crate::net::Cmd;
use crate::net::auth::Me;
use crate::net::emote_ops::EmoteCmd;
use std::collections::BTreeMap;
use winit::keyboard::{Key, ModifiersState, NamedKey};

const ROOMS: [&str; 2] = ["forsen", "lirik"];
const NAMES: [&str; 5] = ["Wlol", "OnlyA", "OnlyB", "Mine2", "Glob2"];
const MESSAGES: usize = 40;

fn url7(id: &str) -> String {
  format!("https://cdn.7tv.app/emote/{id}/1x.webp")
}

fn set(emotes: &[(&str, &str)]) -> Set {
  let e = |id: &str| Emote { url: url7(id).into(), w: 28, h: 28, zero_width: false };
  emotes.iter().map(|(n, id)| ((*n).into(), e(id))).collect()
}

// mulberry32: the generator the site's and the extension's sessions use
struct Rng(u32);
impl Rng {
  fn next(&mut self) -> u32 {
    self.0 = self.0.wrapping_add(0x6D2B_79F5);
    let mut t = self.0;
    t = (t ^ (t >> 15)).wrapping_mul(t | 1);
    t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
    t ^ (t >> 14)
  }
  fn below(&mut self, n: usize) -> usize {
    self.next() as usize % n
  }
  fn of<'a>(&mut self, all: &[&'a str]) -> &'a str {
    all[self.below(all.len())]
  }
}

type Urls = BTreeMap<String, String>;

// the law, as a model: what each place holds, and the three questions the session asks of it
struct Model {
  rooms: [Urls; 2],
  mine: Urls,
  globals: Urls,
  // sent lines the up arrow walks, and the picks kept with a line
  history: Vec<String>,
  kept: BTreeMap<String, Urls>,
}

impl Model {
  fn new() -> Self {
    let m = |v: &[(&str, &str)]| v.iter().map(|(n, id)| ((*n).to_string(), url7(id))).collect::<Urls>();
    Model {
      rooms: [m(&[("Wlol", "AW"), ("OnlyA", "AONLY")]), m(&[("Wlol", "BW"), ("OnlyB", "BONLY")])],
      mine: m(&[("Wlol", "MINE"), ("Mine2", "MINE2")]),
      globals: m(&[("Wlol", "GLOB"), ("Glob2", "GLOB2")]),
      history: Vec::new(),
      kept: BTreeMap::new(),
    }
  }

  // a word typed by hand, sent from this room
  fn typed(&self, room: usize, word: &str) -> Option<String> {
    self.rooms[room].get(word).or_else(|| self.mine.get(word)).or_else(|| self.globals.get(word)).cloned()
  }

  // tab's copies of a name: the room's, yours, the global, each picture once
  fn offers(&self, room: usize, name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for u in [self.rooms[room].get(name), self.mine.get(name), self.globals.get(name)].into_iter().flatten() {
      if !out.contains(u) {
        out.push(u.clone());
      }
    }
    out
  }

  // a word of a draft: the draft's pick of the name, else as typed
  fn word(&self, room: usize, picks: &Urls, word: &str) -> Option<String> {
    picks.get(word).cloned().or_else(|| self.typed(room, word))
  }
}

fn boot() -> (App, tokio::sync::mpsc::UnboundedReceiver<Cmd>) {
  let (net, rx) = Net::tap();
  let mut app = App::new(net, ROOMS.iter().map(|t| (*t).into()).collect(), false);
  app.me = Some(Me {
    username: "mellen".into(),
    platform: "twitch".into(),
    twitch: Some("mellen".into()),
    twitch_id: Some(1),
    names: vec!["mellen".into()],
    kick: None,
    youtube: None,
    packs: vec![],
  });
  app.res.emotes.set(Some("forsen"), Provider::SevenTv, set(&[("Wlol", "AW"), ("OnlyA", "AONLY")]));
  app.res.emotes.set(Some("lirik"), Provider::SevenTv, set(&[("Wlol", "BW"), ("OnlyB", "BONLY")]));
  app.res.emotes.set(None, Provider::SevenTv, set(&[("Wlol", "GLOB"), ("Glob2", "GLOB2")]));
  app.res.senders.lookup(1, "Wlol", 0);
  (app, rx)
}

// your own emotes as the server holds them: the whole set, read again (as after a swap or an add lands)
fn own(app: &mut App, mine: &Urls) {
  let rows: Vec<_> = mine
    .iter()
    .map(|(n, u)| {
      serde_json::json!({"custom_name": n, "url": u, "width": 32, "height": 32, "added_at": "2026-10-30T22:26:51.099Z"})
    })
    .collect();
  app.res.senders.put(1, crate::heatsync::senders::parse_set(&serde_json::json!(rows)));
}

// the row a sent line comes back as: your own line, wearing the picks the room was told
fn row(app: &mut App, chan: &str, text: &str, vouched: &Urls) -> Vec<Option<String>> {
  let mut m = Message::system(now_ms(), Platform::Twitch, text);
  (m.kind, m.user_id, m.login) = (MsgKind::Chat, "1".into(), "mellen".into());
  m.flags |= flag::OWN;
  if !vouched.is_empty() {
    m.extra().picks = vouched.iter().map(|(n, u)| (n.as_str().into(), u.as_str().into(), false)).collect();
  }
  let mut out = Vec::new();
  let mut a = 0;
  for w in text.split(' ') {
    let b = a + w.len();
    out.push(crate::emotes::resolve::find_word(&mut app.res.sources(), chan, &m, a, b, w).map(|f| f.0.to_string()));
    a = b + 1;
  }
  out
}

// keys pressed in the box, one at a time
fn keys(app: &mut App, s: &str) {
  for c in s.chars() {
    let c = c.to_string();
    match c.as_str() {
      " " => app.key_insert(Key::Named(NamedKey::Space), Some(" ")),
      c => app.key_insert(Key::Character(c), Some(c)),
    }
  }
}

// text is about to enter the box from outside it: a pick whose word is no longer there is over
fn entered(picks: &mut Urls, words: &[String]) {
  picks.retain(|name, _| words.iter().any(|w| w == name));
}

fn session(seed: u32) -> (usize, usize, usize, usize) {
  let (mut app, mut rx) = boot();
  app.mode = Mode::Insert;
  let mut model = Model::new();
  own(&mut app, &model.mine);
  let mut rng = Rng(seed);
  // the two rooms' tabs, wherever the app keeps them
  let tab = |app: &App, r: usize| app.tabs.iter().position(|t| &*t.view.channel == ROOMS[r]).unwrap();
  let mut room = 0usize;
  app.activate(tab(&app, room));
  while rx.try_recv().is_ok() {}
  let (mut picked_lines, mut swaps, mut recalls, mut ended) = (0, 0, 0, 0);
  for n in 0..MESSAGES {
    let at = |step: &str| format!("seed {seed} message {n}: {step}");
    // the draft, as the model holds it
    let mut words: Vec<String> = Vec::new();
    let mut picks = Urls::new();
    let mut log: Vec<String> = Vec::new();
    // one message in four starts as a sent line brought back
    if !model.history.is_empty() && rng.below(4) == 0 {
      let back = 1 + rng.below(model.history.len().min(2));
      for _ in 0..back {
        app.key_insert(Key::Named(NamedKey::ArrowUp), None);
      }
      let line = model.history[model.history.len() - back].clone();
      assert_eq!(app.line.text, line, "{}", at("the up arrow brings the sent line back"));
      picks = model.kept.get(&line).cloned().unwrap_or_default();
      words = line.split(' ').map(Into::into).collect();
      log.push(format!("<up {back}>"));
      recalls += 1;
    }
    let steps = if words.is_empty() { 1 + rng.below(4) } else { rng.below(3) };
    for _ in 0..steps {
      if !app.line.text.is_empty() && !app.line.text.ends_with(' ') {
        keys(&mut app, " ");
      }
      match rng.below(12) {
        // a word by hand, a key at a time
        0..=3 => {
          let w = rng.of(&["Wlol", "Wlol", "OnlyA", "OnlyB", "Mine2", "Glob2", "hello"]);
          entered(&mut picks, &words);
          keys(&mut app, &format!("{w} "));
          words.push(w.into());
          log.push(w.into());
        }
        // a name, and tab onto one of its copies
        4..=6 => {
          let name = rng.of(&["Wlol", "Wlol", "Wlol", "OnlyA", "OnlyB", "Mine2", "Glob2"]);
          let offers = model.offers(room, name);
          entered(&mut picks, &words);
          keys(&mut app, name);
          let seen: Vec<String> = app
            .candidates(name, false)
            .iter()
            .filter(|p| &*p.name == name)
            .map(|p| p.pic.as_ref().map_or_else(String::new, |r| r.url.to_string()))
            .collect();
          assert_eq!(seen, offers, "{}", at(&format!("tab's copies of {name}: room, yours, global ({log:?})")));
          let presses = 1 + rng.below(4);
          let back = rng.below(5) == 0;
          let mut i = usize::MAX;
          for p in 0..presses {
            let b = back && p + 1 == presses && p > 0;
            app.tab_complete(b);
            if p == 0 {
              // the catalogs answer what they were asked: nothing more of this name anywhere
              let asked = std::iter::from_fn(|| rx.try_recv().ok());
              let asked: Vec<u32> =
                asked.filter_map(|c| if let Cmd::Catalog { epoch, .. } = c { Some(epoch) } else { None }).collect();
              let none = || Some(crate::model::catalog::Page { hits: vec![], more: false });
              for epoch in asked {
                app.on_net(NetEvent::Catalog { epoch, stv: none(), ffz: none() });
              }
            }
            if !offers.is_empty() {
              let len = offers.len();
              i = match (i, b) {
                (usize::MAX, _) => 0,
                (i, false) => (i + 1) % len,
                (i, true) => (i + len - 1) % len,
              };
              let want = &offers[i];
              assert_eq!(app.box_url(name).as_deref(), Some(want.as_str()), "{}", at(&format!("tab {p} on {name}")));
            }
          }
          app.complete = None;
          if let Some(u) = offers.get(i) {
            picks.insert(name.into(), u.clone());
          }
          words.push(name.into());
          log.push(format!("{name}+tab{presses}{}", if back { "b" } else { "" }));
        }
        // a picture picked on purpose (a picker tile, an emote clicked in a chat line): any copy of the
        // name held anywhere, the other tab's room included
        7 => {
          let name = rng.of(&NAMES);
          let held =
            [model.rooms[0].get(name), model.rooms[1].get(name), model.mine.get(name), model.globals.get(name)];
          let held: Vec<&String> = held.into_iter().flatten().collect();
          let url = held[rng.below(held.len())].clone();
          let rec = app.pick_of(name, &url);
          app.insert_pick(name, rec);
          picks.insert(name.into(), url.clone());
          words.push(name.into());
          log.push(format!("{name}=pick {}", &url[27..url.len() - 8]));
        }
        // the last word deleted (backspace until it is gone, or ctrl-w), and half the time its name put
        // straight back: by keys, or pasted. its pick left with it unless the line holds the name again
        8..=9 if !words.is_empty() => {
          let gone = words.pop().unwrap();
          let ctrl_w = rng.below(2) == 0;
          if ctrl_w {
            app.mods = ModifiersState::CONTROL;
            app.key_insert(Key::Character("w"), None);
            app.mods = ModifiersState::empty();
          } else {
            while app.line.text.split_whitespace().count() > words.len() {
              app.key_insert(Key::Named(NamedKey::Backspace), None);
            }
          }
          log.push(format!("<del {gone}{}>", if ctrl_w { " ^w" } else { "" }));
          if rng.below(2) == 0 {
            let was = picks.get(&gone).cloned();
            entered(&mut picks, &words);
            if !app.line.text.is_empty() && !app.line.text.ends_with(' ') {
              keys(&mut app, " ");
            }
            if rng.below(2) == 0 {
              keys(&mut app, &format!("{gone} "));
              log.push(format!("{gone} again"));
            } else {
              app.on_pasted(false, Ok(crate::util::clip::Paste::Text(format!("{gone} "))));
              log.push(format!("{gone} pasted"));
            }
            words.push(gone.clone());
            ended += usize::from(was.is_some() && !picks.contains_key(&gone) && was != model.typed(room, &gone));
          }
        }
        // the other tab, in the middle of the draft
        _ => {
          room = 1 - room;
          app.activate(tab(&app, room));
          log.push(format!("<to {}>", ROOMS[room]));
        }
      }
      // the box after every step: each word is its draft's answer where the box now is
      for w in &words {
        let want = model.word(room, &picks, w);
        assert_eq!(app.box_url(w).map(|u| u.to_string()), want, "{}", at(&format!("the box, word {w}, after {log:?}")));
      }
    }
    if words.is_empty() {
      // nothing left to send: the box is emptied by hand
      while !app.line.text.is_empty() {
        app.key_insert(Key::Named(NamedKey::Backspace), None);
      }
      continue;
    }
    let text = words.join(" ");
    assert_eq!(app.line.text.trim(), text, "{}", at(&format!("the words in the box ({log:?})")));
    let boxed: Vec<Option<String>> = words.iter().map(|w| app.box_url(w).map(|u| u.to_string())).collect();
    let want: Vec<Option<String>> = words.iter().map(|w| model.word(room, &picks, w)).collect();
    assert_eq!(boxed, want, "{}", at(&format!("the box at enter ({log:?})")));
    // what the law says leaves: a pick is told to the room when it is not what the word sends anyway, and
    // it becomes your emote of that name when it is neither the room's here nor already yours (nor, with
    // none of your own, the global)
    picks.retain(|name, _| words.iter().any(|w| w == name));
    let told: Urls = picks
      .iter()
      .filter(|(n, u)| model.typed(room, n).as_ref() != Some(u))
      .map(|(n, u)| (n.clone(), u.clone()))
      .collect();
    let mut changes: Vec<(String, String, bool)> = Vec::new();
    for (name, u) in &told {
      let owned = model.mine.get(name);
      if owned == Some(u) || (owned.is_none() && model.globals.get(name) == Some(u)) {
        continue;
      }
      changes.push((name.clone(), u.clone(), owned.is_some()));
    }
    while rx.try_recv().is_ok() {}
    app.send();
    let out: Vec<Cmd> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    let sent: Vec<(String, String)> = out
      .iter()
      .filter_map(|c| match c {
        Cmd::Send { chan, text, .. } => Some((chan.to_string(), text.clone())),
        _ => None,
      })
      .collect();
    assert_eq!(sent, [(ROOMS[room].to_string(), text.clone())], "{}", at(&format!("goes to the open tab ({log:?})")));
    let vouched: Urls = out
      .iter()
      .filter_map(|c| match c {
        Cmd::Hs(crate::net::hs_ws::Cmd::Used(_, chan, v)) => {
          assert_eq!(&**chan, ROOMS[room], "{}", at("the vouch names the room the line went to"));
          Some(v.iter().map(|e| (e.name.to_string(), e.url.to_string())).collect::<Vec<_>>())
        }
        _ => None,
      })
      .flatten()
      .collect();
    assert_eq!(vouched, told, "{}", at(&format!("the room is told the picks that change a word ({log:?})")));
    let mut did: Vec<(String, String, bool)> = out
      .iter()
      .filter_map(|c| match c {
        Cmd::Emote(EmoteCmd::Swap { name, url, .. }) => Some((name.to_string(), url.to_string(), true)),
        Cmd::Emote(EmoteCmd::Add { name, url, .. }) => Some((name.to_string(), url.to_string(), false)),
        _ => None,
      })
      .collect();
    did.sort();
    changes.sort();
    assert_eq!(did, changes, "{}", at(&format!("what becomes yours: (name, picture, swapped) ({log:?})")));
    // the row that comes back is the box, before and after the swap lands
    let shipped: Vec<Option<String>> =
      words.iter().map(|w| vouched.get(w).cloned().or_else(|| model.typed(room, w))).collect();
    assert_eq!(shipped, boxed, "{}", at(&format!("what went out is the box ({log:?})")));
    assert_eq!(row(&mut app, ROOMS[room], &text, &vouched), boxed, "{}", at(&format!("the row is the box ({log:?})")));
    // the line is remembered with the picks that were a choice from here
    let chosen: Urls = told.clone();
    if model.history.last() != Some(&text) {
      model.history.push(text.clone());
    }
    model.kept.remove(&text);
    if !chosen.is_empty() {
      model.kept.insert(text.clone(), chosen);
    }
    for (name, u, swapped) in &changes {
      model.mine.insert(name.clone(), u.clone());
      swaps += usize::from(*swapped);
    }
    if !changes.is_empty() {
      own(&mut app, &model.mine);
      assert_eq!(row(&mut app, ROOMS[room], &text, &vouched), boxed, "{}", at("the row holds once the swap lands"));
    }
    picked_lines += usize::from(!told.is_empty());
    assert!(app.line.text.is_empty() && app.line.picks.is_empty(), "{}", at("the box is empty after a send"));
    for name in NAMES {
      let want = model.typed(room, name);
      assert_eq!(
        app.box_url(name).map(|u| u.to_string()),
        want,
        "{}",
        at(&format!("no pick outlives its message: {name}"))
      );
    }
  }
  (picked_lines, swaps, recalls, ended)
}

#[test]
fn a_random_session_in_the_box_is_the_law_on_every_message() {
  let gate = std::env::var("HS_SESSION_SEEDS").is_err();
  let seeds = std::env::var("HS_SESSION_SEEDS").unwrap_or_else(|_| "1,2,3,4".into());
  let (mut picked, mut swaps, mut recalls, mut ended) = (0, 0, 0, 0);
  for seed in seeds.split(',').filter_map(|s| s.trim().parse::<u32>().ok()) {
    let (p, s, r, e) = session(seed);
    (picked, swaps, recalls, ended) = (picked + p, swaps + s, recalls + r, ended + e);
  }
  // the gate's seeds must reach the cases the session is for
  if gate {
    assert!(
      picked >= 20 && swaps >= 4 && recalls >= 20,
      "{picked} lines with a told pick, {swaps} swaps, {recalls} recalls"
    );
  }
}

// a pick is over when its word leaves the box, however the name comes back: typed, pasted, through an
// input method, or quoted from a chat line. the name entered again is a hand-typed word, the room's emote
#[test]
fn a_pick_ends_when_its_word_leaves_the_box_however_the_name_comes_back() {
  type Put = fn(&mut App);
  let enter: [(&str, Put); 4] = [
    ("typed", |app| {
      for c in ["W", "l", "o", "l"] {
        app.key_insert(Key::Character(c), Some(c));
      }
    }),
    ("pasted", |app| app.on_pasted(false, Ok(crate::util::clip::Paste::Text("Wlol".into())))),
    ("through an input method", |app| app.on_ime(winit::event::Ime::Commit("Wlol".into()))),
    ("put in by the app (a quoted line)", |app| app.type_into_line("Wlol")),
  ];
  for (how, put) in enter {
    let (mut app, mut rx) = boot();
    let model = Model::new();
    own(&mut app, &model.mine);
    app.mode = Mode::Insert;
    let i = app.tabs.iter().position(|t| &*t.view.channel == "forsen").unwrap();
    app.activate(i);
    // your own Wlol, picked over the room's
    app.line.insert("Wlol");
    app.tab_complete(false);
    app.tab_complete(false);
    app.complete = None;
    assert_eq!(app.box_url("Wlol").as_deref(), Some(&*url7("MINE")), "{how}: the pick");
    // the word leaves the box
    while !app.line.text.is_empty() {
      app.key_insert(Key::Named(NamedKey::Backspace), None);
    }
    put(&mut app);
    assert_eq!(app.line.text.trim(), "Wlol", "{how}");
    assert_eq!(app.box_url("Wlol").as_deref(), Some(&*url7("AW")), "{how}: the name again is the room's emote");
    while rx.try_recv().is_ok() {}
    app.send();
    let out: Vec<Cmd> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(out.iter().any(|c| matches!(c, Cmd::Send { text, .. } if text == "Wlol")), "{how}: the line went out");
    assert!(
      out.iter().all(|c| !matches!(c, Cmd::Hs(crate::net::hs_ws::Cmd::Used(..)) | Cmd::Emote(_))),
      "{how}: no pick is told to the room, nothing of yours changes"
    );
  }
}
