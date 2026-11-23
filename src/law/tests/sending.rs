//! the message box read against the law's send cases.
//!
//! `emotes::draft::Draft::word` is the one place a word in the box becomes the emote it will send: the chip,
//! the vouch and the kick wire all ask it. like `find_word` it is built on the app's own stores, so the law
//! passing its vectors says nothing about the box. here each step of each session case is staged in those
//! stores, with the step's picks as the draft's, and every word of its line is asked of `Draft::word`: the
//! chips must be the law's. what the app has no way to say yet is counted, by reason, exactly.

use super::drawing::{SENDER, TS, alias, emote, provider, sets_of, stage_owner, url};
use super::*;
use crate::emotes::draft::Draft;
use crate::emotes::personal::{Ev, Personal};
use crate::emotes::{Emotes, Set};
use crate::heatsync::senders::{Senders, parse_set};
use crate::model::catalog::{Rec, Registry};
use crate::model::message::Platform as Feed;

// why a send step is one the box cannot say
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Cannot {
  // editing a sent chat line: the app edits posts only, and a post has no room
  Edit,
  // the case setting is the site's and the extension's only
  Fold,
  // heatsync's own global emotes are not a set the app loads
  HsGlobal,
  // a kick emote that is yours alone (a sub's) outranking the room's ffz/7tv/bttv of its name: the app
  // holds one free set per kick channel, and it ranks as the room's
  KickOwn,
  // a line too long for kick in kick's format goes out as words: the box still shows the chips
  KickOverflow,
  // `name0` as the name drawn as an overlay
  NameZero,
}

type Chips = Vec<(String, String)>;

fn step(
  file: &Map<String, Value>,
  c: &Map<String, Value>,
  s: &Map<String, Value>,
  seed: u32,
) -> Res<Result<(Chips, Chips), Cannot>> {
  let raw = obj(&file["worlds"][str_of(c, "world").ok_or("world")?], "world")?;
  let mut rng = Rng(seed);
  let w = build_world(file, raw, &mut rng)?;
  let expect = obj(&s["expect"], "expect")?;
  if s.get("stored").is_some_and(|v| !v.is_null()) {
    return Ok(Err(Cannot::Edit));
  }
  if s.get("fold").and_then(Value::as_bool) == Some(true) {
    return Ok(Err(Cannot::Fold));
  }
  if expect.contains_key("wireIsText") {
    return Ok(Err(Cannot::KickOverflow));
  }
  let globals = rng.shuffled(&strs(Some(&raw["globals"])));
  let want: Chips = expect["chips"]
    .as_array()
    .ok_or("chips")?
    .iter()
    .filter_map(|d| Some((d["word"].as_str()?.to_owned(), d["emote"].as_str()?.to_owned())))
    .collect();
  for (word, a) in &want {
    if word.strip_suffix('0') == Some(w.emote(a)?.name.as_str()) {
      return Ok(Err(Cannot::NameZero));
    }
    if w.emote(a)?.p == "hs" && globals.iter().any(|g| g == a) {
      return Ok(Err(Cannot::HsGlobal));
    }
  }
  let who = str_of(c, "who").ok_or("who")?;
  let me = obj(&raw["people"][who], who)?;
  let native = |p: &str| strs(me.get("native").and_then(|n| n.get(p)));

  let (mut emotes, mut senders, mut personal) = (Emotes::default(), Senders::default(), Personal::default());
  personal.set_on(true);
  for (p, set) in sets_of(&w, &globals)?.ok_or("two globals of one name in one provider")? {
    if let Some(p) = provider(&p) {
      emotes.set(None, p, set);
    }
  }
  // twitch emotes that are yours to send and no global: your subs'
  let mut subs = Set::new();
  for a in native("twitch").iter().filter(|a| !globals.contains(a)) {
    subs.insert(w.emote(a)?.name.as_str().into(), emote(&w, a)?);
  }
  emotes.set_subs(subs);
  let (leg, chan) = match str_of(s, "room") {
    None => (None, ""),
    Some(id) => {
      let (p, ch) = id.split_once(':').ok_or("room id")?;
      let r = obj(&raw["rooms"][id], id)?;
      let feed = match p {
        "kick" => Feed::Kick,
        "youtube" => Feed::Youtube,
        _ => Feed::Twitch,
      };
      if !stage_owner(&w, &mut emotes, feed, ch, &rng.shuffled(&strs(r.get("owner"))))? {
        return Err("two of one name in a streamer's set".into());
      }
      let third = strs(r.get("third"));
      // a room still loading has no sets yet: the box reads without them
      if r.get("loaded").and_then(Value::as_bool) != Some(false) {
        for (p, set) in sets_of(&w, &rng.shuffled(&third))?.ok_or("two of one name in one provider")? {
          emotes.set(Some(ch), provider(&p).ok_or("a third-party list holds an unknown provider")?, set);
        }
      }
      if p == "kick" {
        // the app's one set for a kick channel: the channel's free emotes and kick's globals
        let mut set = Set::new();
        for a in strs(r.get("native")).iter().chain(&native("kick")) {
          set.insert(w.emote(a)?.name.as_str().into(), emote(&w, a)?);
        }
        emotes.set_kick(ch, set);
        emotes.route_kick([(ch, ch)]);
        // a kick emote the law ranks as yours, over a third-party emote of its name in the room
        let froms = expect.get("from").and_then(Value::as_object);
        let mine = |word: &str| froms.and_then(|f| f.get(word)).and_then(Value::as_str) == Some("native");
        let clash = |word: &str| third.iter().any(|a| w.emote(a).is_ok_and(|e| e.name == word));
        if want.iter().any(|(word, _)| mine(word) && clash(word)) {
          return Ok(Err(Cannot::KickOwn));
        }
      }
      (Some(feed), ch)
    }
  };
  // you: your heatsync emotes and your 7tv personal set, the winners of each name
  let inv = law::build_index(&w.list(&mut rng, me.get("inventory"))?, None);
  let pers = law::build_index(&w.list(&mut rng, me.get("personal"))?, None);
  let named = |e: &Em| w.alias(Some(e)).as_str().map(String::from).ok_or("alias");
  let mut rows = Vec::new();
  for e in inv.values() {
    rows.push(json!({"custom_name": e.name, "url": url(&named(e)?), "width": 32, "height": 32,
      "zero_width": e.zw, "added_at": "2026-10-30T22:26:51.099Z"}));
  }
  senders.lookup(SENDER, "x", TS);
  senders.put(SENDER, parse_set(&Value::Array(rows)));
  let mut set = Set::new();
  for e in pers.values() {
    set.insert(e.name.as_str().into(), emote(&w, &named(e)?)?);
  }
  personal.apply(Ev::Grant { users: vec![SENDER], set: "SET".into() });
  personal.apply(Ev::Set("SET".into(), set));
  // the picks made in this draft
  let mut picks = Registry::default();
  if let Some(p) = s.get("picks").and_then(Value::as_object) {
    for (word, a) in p {
      let a = a.as_str().ok_or("pick alias")?;
      picks.put(word, Some(Rec { url: url(a).into(), zero_width: w.emote(a)?.zw, provider: "7tv" }));
    }
  }
  let text = match s.get("textRepeat") {
    Some(r) => {
      vec![r[0].as_str().ok_or("textRepeat word")?; r[1].as_u64().ok_or("textRepeat count")? as usize].join(" ")
    }
    None => str_of(s, "text").ok_or("step without text")?.to_string(),
  };
  let d = Draft {
    emotes: &emotes,
    senders: &senders,
    personal: &personal,
    me: Some(SENDER),
    inventory: true,
    leg,
    chan,
    now: TS,
  };
  let got: Chips = text
    .split(law::is_space)
    .filter_map(|word| d.word(Some(&picks), word).map(|(f, _)| (word.to_owned(), alias(&f.0))))
    .collect();
  Ok(Ok((got, want)))
}

#[test]
fn the_box_agrees_with_the_law_on_every_send_step_it_can_say() {
  let f = file();
  let (mut said, mut cannot, mut failed) = (0, HashMap::<Cannot, usize>::new(), Vec::new());
  for c in f["cases"].as_array().unwrap().iter().filter(|c| c["kind"] == "session") {
    let c = c.as_object().unwrap();
    for (i, s) in c["steps"].as_array().unwrap().iter().enumerate() {
      let s = s.as_object().unwrap();
      for seed in SHUFFLES {
        match step(&f, c, s, seed) {
          Ok(Err(why)) => {
            *cannot.entry(why).or_default() += 1;
            break;
          }
          Ok(Ok((got, want))) if got == want => {}
          Ok(Ok((got, want))) => {
            failed.push(format!("{} step {i} (shuffle {seed})\n  want {want:?}\n  got  {got:?}", c["name"]));
            break;
          }
          Err(e) => {
            failed.push(format!("{} step {i}: {e}", c["name"]));
            break;
          }
        }
        if seed == SHUFFLES[SHUFFLES.len() - 1] {
          said += 1;
        }
      }
    }
  }
  let mut why: Vec<(String, usize)> = cannot.iter().map(|(k, n)| (format!("{k:?}"), *n)).collect();
  why.sort();
  println!("the box: {said} send steps agree under every shuffle; cannot say: {why:?}");
  assert!(failed.is_empty(), "{} step(s) differ:\n{}", failed.len(), failed.join("\n"));
  // exact, so a quietly shrinking set fails. each reason is a gap with a name: closing one moves its
  // steps into `said`
  let n = |k| cannot.get(&k).copied().unwrap_or(0);
  let split = (
    said,
    n(Cannot::Edit),
    n(Cannot::Fold),
    n(Cannot::HsGlobal),
    n(Cannot::KickOwn),
    n(Cannot::KickOverflow),
    n(Cannot::NameZero),
  );
  assert_eq!(split, (15, 7, 1, 0, 1, 1, 2), "the sending split moved");
}
