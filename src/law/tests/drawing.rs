//! the app's own drawing path read against the law's read cases.
//!
//! `emotes::resolve::find_word` is the one place a word of a chat line becomes an emote. it is built on
//! the app's own stores (the channel and global catalogs, senders' sets, 7tv personal sets), not on the
//! law's types, so the law passing its vectors says nothing about what a row draws. here each read case
//! is staged in those stores the way the network fills them, and every word of its line is asked of
//! `find_word`. what the app has no way to say yet is counted, by reason, and the counts are exact: a
//! case that quietly stops being checked fails.

use super::*;
use crate::emotes::personal::{Ev, Personal};
use crate::emotes::resolve::{Sources, find_word};
use crate::emotes::{Emote, Emotes, Provider, Set};
use crate::heatsync::senders::{Senders, parse_set};
use crate::model::message::{HsEmote, Kind, Message, Platform as Feed};

pub(super) const SENDER: u64 = 42;
pub(super) const TS: i64 = 1_793_399_211_099;

// a picture's address, carrying its alias: an address the app's image allowlist takes
pub(super) fn url(alias: &str) -> String {
  format!("https://cdn.7tv.app/emote/{alias}/1x.webp")
}

pub(super) fn alias(url: &str) -> String {
  url.rsplit('/').nth(1).unwrap_or("").to_owned()
}

pub(super) fn emote(w: &World, a: &str) -> Res<Emote> {
  Ok(Emote { url: url(a).into(), w: 28, h: 28, zero_width: w.emote(a)?.zw })
}

pub(super) fn provider(p: &str) -> Option<Provider> {
  Some(match p {
    "7tv" => Provider::SevenTv,
    "bttv" => Provider::Bttv,
    "ffz" => Provider::Ffz,
    "twitch" => Provider::Twitch,
    _ => return None,
  })
}

// why a read case is one the app cannot say
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Cannot {
  // the case setting (caseInsensitiveEmotes) is the site's and the extension's only
  Fold,
  // heatsync's own global emotes are not a set the app loads
  HsGlobal,
  // one provider list holding two emotes of one name: a provider's set is name -> emote here
  TwoOfOneName,
}

// one provider's emotes as the set the app keeps, or None when two share a name
pub(super) fn sets_of(w: &World, aliases: &[String]) -> Res<Option<HashMap<String, Set>>> {
  let mut out: HashMap<String, Set> = HashMap::new();
  for a in aliases {
    let e = w.emote(a)?;
    if out.entry(e.p.clone()).or_default().insert(e.name.as_str().into(), emote(w, a)?).is_some() {
      return Ok(None);
    }
  }
  Ok(Some(out))
}

// the streamer's own heatsync emotes as the app keeps them: one set for the room's channel, routed to its
// tab the way the app routes it. false when two share a name (the set is name -> emote)
pub(super) fn stage_owner(w: &World, emotes: &mut Emotes, feed: Feed, ch: &str, aliases: &[String]) -> Res<bool> {
  let mut set = Set::new();
  for a in aliases {
    if set.insert(w.emote(a)?.name.as_str().into(), emote(w, a)?).is_some() {
      return Ok(false);
    }
  }
  let leg = [(feed, Box::from(ch))];
  emotes.route_owner([(ch, &leg[..])]);
  let [leg] = leg;
  Ok(aliases.is_empty() || emotes.set_owner(leg, set))
}

// a room's third-party sets where the app keeps them: a twitch channel's by its chat key, a kick or youtube
// channel's by the channel (`stage_owner` routed it)
pub(super) fn stage_third(emotes: &mut Emotes, feed: Feed, ch: &str, sets: Vec<(Provider, Set)>) {
  if feed == Feed::Twitch {
    for (p, set) in sets {
      emotes.set(Some(ch), p, set);
    }
  } else {
    assert!(emotes.set_leg((feed, ch.into()), sets), "{ch}: the room is routed before its sets land");
  }
}

// (emotes the app drew, emotes the law expects), or why the app cannot say the case
type Said = Result<(Vec<String>, Vec<String>), Cannot>;

fn drawing(file: &Map<String, Value>, c: &Value, seed: u32) -> Res<Said> {
  let c = obj(c, "case")?;
  let raw = obj(&file["worlds"][str_of(c, "world").ok_or("world")?], "world")?;
  let mut rng = Rng(seed);
  let w = build_world(file, raw, &mut rng)?;
  if c.get("fold").and_then(Value::as_bool) == Some(true) {
    return Ok(Err(Cannot::Fold));
  }
  let expect = c["expect"].as_array().ok_or("expect")?;
  let globals = rng.shuffled(&strs(Some(&raw["globals"])));
  for d in expect {
    let Some(a) = d["emote"].as_str() else { continue };
    if w.emote(a)?.p == "hs" && globals.iter().any(|g| g == a) {
      return Ok(Err(Cannot::HsGlobal));
    }
  }

  let (mut emotes, mut senders, mut personal) = (Emotes::default(), Senders::default(), Personal::default());
  personal.set_on(true);
  // the globals, a set per provider
  let Some(sets) = sets_of(&w, &globals)? else { return Ok(Err(Cannot::TwoOfOneName)) };
  for (p, set) in sets {
    // heatsync's own globals are not a set the app holds (`Cannot::HsGlobal`)
    if let Some(p) = provider(&p) {
      emotes.set(None, p, set);
    }
  }
  // the line's room: its channel's sets per provider, and a kick channel's own emotes
  let (feed, chan) = match str_of(c, "room") {
    None => (Feed::Twitch, ""),
    Some(id) => {
      let (p, ch) = id.split_once(':').ok_or("room id")?;
      let r = obj(&raw["rooms"][id], id)?;
      let feed = match p {
        "kick" => Feed::Kick,
        "youtube" => Feed::Youtube,
        _ => Feed::Twitch,
      };
      if !stage_owner(&w, &mut emotes, feed, ch, &rng.shuffled(&strs(r.get("owner"))))? {
        return Ok(Err(Cannot::TwoOfOneName));
      }
      let Some(third) = sets_of(&w, &rng.shuffled(&strs(r.get("third"))))? else {
        return Ok(Err(Cannot::TwoOfOneName));
      };
      let mut sets = Vec::new();
      for (p, set) in third {
        sets.push((provider(&p).ok_or_else(|| format!("{id}: a third-party list holds provider {p}"))?, set));
      }
      stage_third(&mut emotes, feed, ch, sets);
      let native = rng.shuffled(&strs(r.get("native")));
      if !native.is_empty() {
        let mut set = Set::new();
        for a in &native {
          set.insert(w.emote(a)?.name.as_str().into(), emote(&w, a)?);
        }
        emotes.set_kick(ch, set);
        emotes.route_kick([(ch, ch)]);
      }
      (feed, ch)
    }
  };

  let text = str_of(c, "text").ok_or("text")?;
  let mut m = Message::system(TS, feed, text);
  m.kind = Kind::Chat;
  (m.id, m.login, m.nick) = ("line".into(), "sender".into(), "sender".into());
  // the sender: on twitch their set is fetched by their id and their 7tv personal set is theirs by 7tv's
  // word; anywhere else the server matches their emotes to the line by name (`hsEmotes`)
  if let Some(who) = str_of(c, "sender") {
    let p = obj(&raw["people"][who], who)?;
    let inv = law::build_index(&w.list(&mut rng, p.get("inventory"))?, None);
    let pers = law::build_index(&w.list(&mut rng, p.get("personal"))?, None);
    let named = |e: &Em| w.alias(Some(e)).as_str().map(String::from).ok_or("alias");
    if feed == Feed::Twitch {
      m.user_id = SENDER.to_string().into();
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
    } else {
      let mut hs: Vec<HsEmote> = Vec::new();
      for e in inv.values() {
        hs.push((e.name.as_str().into(), url(&named(e)?).into(), e.zw));
      }
      m.hs = hs.into();
    }
  }
  // what the sender's app shipped with the line: its picks
  if let Some(refs) = c.get("refs").and_then(Value::as_object) {
    let mut picks: Vec<HsEmote> = Vec::new();
    for (name, v) in refs {
      let a = v.as_str().ok_or("ref alias")?;
      picks.push(match a {
        "gone" => (name.as_str().into(), "".into(), false),
        _ => (name.as_str().into(), url(a).into(), w.emote(a)?.zw),
      });
    }
    m.extra().picks = picks.into();
  }
  // platform tokens, by position: the file counts characters, a line's ranges are bytes
  let byte = |cp: usize| text.char_indices().nth(cp).map_or(text.len(), |(i, _)| i) as u32;
  let mut placed = Vec::new();
  for t in tokens_in(&w, c.get("tokens"))? {
    let a = w.alias(Some(t.emote)).as_str().map(String::from).ok_or("token alias")?;
    placed.push((byte(t.start)..byte(t.end), url(&a).into()));
  }
  m.emotes = placed.into();

  let mut src = Sources { emotes: &emotes, senders: &mut senders, personal: &mut personal, inventory: true };
  let mut got = Vec::new();
  let mut at = 0;
  for word in text.split(law::is_space) {
    let (a, b) = (at, at + word.len());
    at = b + text[b..].chars().next().map_or(0, char::len_utf8);
    if let Some(found) = find_word(&mut src, chan, &m, a, b, word) {
      got.push(alias(&found.0));
    }
  }
  let want = expect.iter().filter_map(|d| d["emote"].as_str().map(String::from)).collect();
  Ok(Ok((got, want)))
}

#[test]
fn the_apps_drawing_path_agrees_with_the_law_on_every_read_case_it_can_say() {
  let f = file();
  let (mut said, mut cannot, mut failed) = (0, HashMap::<Cannot, usize>::new(), Vec::new());
  for c in f["cases"].as_array().unwrap().iter().filter(|c| c["kind"] == "read") {
    for seed in SHUFFLES {
      match drawing(&f, c, seed) {
        Ok(Err(why)) => {
          *cannot.entry(why).or_default() += 1;
          break;
        }
        Ok(Ok((got, want))) if got == want => {}
        Ok(Ok((got, want))) => {
          failed.push(format!("{} (shuffle {seed})\n  want {want:?}\n  got  {got:?}", c["name"]));
          break;
        }
        Err(e) => {
          failed.push(format!("{}: {e}", c["name"]));
          break;
        }
      }
      if seed == SHUFFLES[SHUFFLES.len() - 1] {
        said += 1;
      }
    }
  }
  let mut why: Vec<(String, usize)> = cannot.iter().map(|(k, n)| (format!("{k:?}"), *n)).collect();
  why.sort();
  println!("the app's drawing: {said} read cases agree under every shuffle; cannot say: {why:?}");
  assert!(failed.is_empty(), "{} case(s) differ:\n{}", failed.len(), failed.join("\n"));
  // exact, so a quietly shrinking set fails. each reason is a gap with a name: closing one moves its
  // cases into `said`
  let n = |k| cannot.get(&k).copied().unwrap_or(0);
  assert_eq!(
    (said, n(Cannot::Fold), n(Cannot::HsGlobal), n(Cannot::TwoOfOneName)),
    (24, 2, 1, 0),
    "the drawing split moved"
  );
}
